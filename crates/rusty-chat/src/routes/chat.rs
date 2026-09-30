//! `POST /api/chat/completions` + `/ws` — the M1 chat pipeline:
//! auth → model resolution → user/assistant message persistence → provider
//! stream (rc-llm adapters) → WS `events` frames → persistence.

use axum::Json;
use axum::extract::State;
use axum::extract::ws::{Message as WsMessage, WebSocket, WebSocketUpgrade};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use futures::StreamExt;
use rc_core::chat::{ChatCompletionForm, StreamDelta};
use rc_core::events::{WsFrame, event_data};
use rc_core::timestamp::Secs;
use rc_llm::ollama_chat as ollama_adapter;
use rc_llm::openai_chat as openai_adapter;
use rc_llm::openai_chat::ChatTarget;
use rc_llm::registry::{BackendConfig, all_base_models};
use serde_json::{Value, json};
use std::pin::Pin;
use uuid::Uuid;

use crate::extract::VerifiedUser;
use crate::routes::models::backend_config;
use crate::state::AppState;

/// `POST /api/chat/completions` — parses the open-webui form, persists the
/// user message, spawns the generation task, returns the task envelope.
pub async fn chat_completion(
    State(app): State<AppState>,
    VerifiedUser(user): VerifiedUser,
    Json(value): Json<Value>,
) -> Response {
    let Ok(form) = serde_json::from_value::<ChatCompletionForm>(value.clone()) else {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"detail": "invalid chat form"})),
        )
            .into_response();
    };

    // ---- model resolution (workspace models arrive in M3) ----
    let cfg = backend_config(&app).await;
    let models = match all_base_models(&cfg).await {
        Ok(m) => m,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"detail": e.to_string()})),
            )
                .into_response();
        }
    };
    let Some(model) = models.iter().find(|m| m.id == form.model) else {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({"detail": "Model not found"})),
        )
            .into_response();
    };
    let model = model.clone();

    // ---- ids (open-webui main.py semantics) ----
    // open-webui: parent_id null → new chat; absent → legacy no-chat-management.
    // serde maps JSON null and absence alike to None for Option<String>, so
    // M1 treats "no chat_id" as new-chat intent (our frontend always sends it).
    let is_new_chat = form.chat_id.is_none();
    let chat_id = match form.chat_id.clone() {
        Some(id) if !id.is_empty() => id,
        _ => Uuid::new_v4().to_string(),
    };
    let message_id = form
        .id
        .clone()
        .unwrap_or_else(|| Uuid::new_v4().to_string());

    // ---- user message ----
    let user_message: Value = form.user_message.clone().unwrap_or_else(|| {
        let last_user = form
            .messages
            .iter()
            .rev()
            .find(|m| m.role == "user")
            .map(|m| json!({"content": m.content}));
        json!({"id": Uuid::new_v4().to_string(), "role": "user", "content": last_user.and_then(|m| m.get("content").cloned()).unwrap_or(json!("")), "timestamp": Secs::now().as_i64()})
    });

    // ---- persist user + assistant placeholder ----
    let stream_form = form.clone();
    let display_name = if model.name.is_empty() {
        &form.model
    } else {
        model.name.as_str()
    };
    let persist_user = chats_persist_user_message(
        &app,
        &user.id,
        &chat_id,
        user_message.clone(),
        &message_id,
        display_name,
    )
    .await;
    if let Err(e) = persist_user {
        tracing::warn!(chat_id = %chat_id, error = %e, "user message persistence failed");
    }

    // ---- stream=false → synchronous OpenAI-shaped JSON ----
    if form.stream == Some(false) {
        return match complete_sync(&app, &model, &stream_form).await {
            Ok(body) => Json(body).into_response(),
            Err(rc_core::Error::NotFound(detail)) => {
                (StatusCode::NOT_FOUND, Json(json!({"detail": detail}))).into_response()
            }
            Err(e) => (
                StatusCode::BAD_GATEWAY,
                Json(json!({"detail": e.to_string()})),
            )
                .into_response(),
        };
    }

    // ---- streaming: spawn task, return task envelope ----
    let task_id = Uuid::new_v4().to_string();
    let task_app = app.clone();
    let task_user_id = user.id.clone();
    let task_chat_id = chat_id.clone();
    let task_message_id = message_id.clone();
    let task_model = model.clone();
    tokio::spawn(async move {
        run_generation(
            task_app,
            task_user_id,
            task_chat_id,
            task_message_id,
            task_model,
            stream_form,
            is_new_chat,
        )
        .await;
    });

    Json(json!({
        "status": true,
        "task_ids": [task_id],
        "chat_id": chat_id,
    }))
    .into_response()
}

type DeltaStream = Pin<Box<dyn futures::Stream<Item = rc_core::Result<StreamDelta>> + Send>>;

/// Opens the provider stream for a model, routing ollama vs openai.
async fn open_provider_stream(
    app: &AppState,
    model: &rc_llm::ModelInfo,
    form: &ChatCompletionForm,
) -> rc_core::Result<DeltaStream> {
    match model.owned_by.as_str() {
        "ollama" => {
            let base = ollama_base_for(app, model).await;
            match base {
                Some(url) => {
                    let stream = ollama_adapter::stream_chat(&url, form).await?;
                    Ok(Box::pin(stream) as DeltaStream)
                }
                None => Err(rc_core::Error::NotFound(
                    "ollama backend not configured".into(),
                )),
            }
        }
        _ => {
            let target = openai_target_for(app, model).await;
            match target {
                Some(target) => {
                    let stream = openai_adapter::stream_chat(target, form).await?;
                    Ok(Box::pin(stream) as DeltaStream)
                }
                None => Err(rc_core::Error::NotFound(
                    "openai connection not configured".into(),
                )),
            }
        }
    }
}

/// Synchronous (stream=false) completion for any model — shared by the
/// chat pipeline and the task endpoints (`/api/v1/tasks/*`).
pub(crate) async fn complete_sync(
    app: &AppState,
    model: &rc_llm::ModelInfo,
    form: &ChatCompletionForm,
) -> rc_core::Result<Value> {
    match model.owned_by.as_str() {
        "ollama" => {
            let base = ollama_base_for(app, model).await;
            match base {
                Some(url) => ollama_adapter::complete(&url, form).await,
                None => Err(rc_core::Error::NotFound(
                    "ollama backend not configured".into(),
                )),
            }
        }
        _ => {
            let Some(target) = openai_target_for(app, model).await else {
                return Err(rc_core::Error::NotFound(
                    "openai connection not configured".into(),
                ));
            };
            openai_adapter::complete(target, form).await
        }
    }
}

/// open-webui `get_task_model_id`: keep the requested model unless a
/// configured task model matches the connection type (local ↔ default,
/// external ↔ external) and actually exists.
pub(crate) async fn task_model_id_for(
    app: &AppState,
    requested_model_id: &str,
    models: &[rc_llm::ModelInfo],
) -> String {
    let as_str = |v: Option<Value>| {
        v.and_then(|v| v.as_str().map(str::to_string))
            .filter(|s| !s.is_empty())
    };
    let configured = if models
        .iter()
        .find(|m| m.id == requested_model_id)
        .is_some_and(|m| m.owned_by == "ollama")
    {
        as_str(app.config.get("task.model.default").await.ok().flatten())
    } else {
        as_str(app.config.get("task.model.external").await.ok().flatten())
    };
    match configured {
        Some(id) if models.iter().any(|m| m.id == id) => id,
        _ => requested_model_id.to_string(),
    }
}

/// Generation task: streams provider deltas as WS `events` frames and
/// persists the assistant message at the end (or the error).
async fn run_generation(
    app: AppState,
    user_id: String,
    chat_id: String,
    message_id: String,
    model: rc_llm::ModelInfo,
    form: ChatCompletionForm,
    is_new_chat: bool,
) {
    let emit = |data: Value| {
        let frame = WsFrame::chat_event(&chat_id, Some(&message_id), data);
        app.hub
            .send_to_user(&user_id, &serde_json::to_string(&frame).unwrap_or_default());
    };

    emit(event_data::chat_active(true));

    let result = open_provider_stream(&app, &model, &form).await;

    let mut deltas = match result {
        Ok(d) => d,
        Err(e) => {
            tracing::warn!(chat_id = %chat_id, error = %e, "generation failed");
            emit(event_data::message_error(json!({"detail": e.to_string()})));
            emit(event_data::chat_active(false));
            return;
        }
    };

    let mut acc = rc_core::chat::OutputAccumulator::default();
    while let Some(delta) = deltas.next().await {
        match delta {
            Ok(StreamDelta::Content { content }) => {
                emit(event_data::message_delta(&content));
                acc.push(&StreamDelta::Content { content });
            }
            Ok(other) => acc.push(&other),
            Err(e) => {
                emit(event_data::message_error(json!({"detail": e.to_string()})));
                emit(event_data::chat_active(false));
                return;
            }
        }
    }

    // ---- persist the assistant message (blob + chat_message row) ----
    let output = Value::Array(
        acc.clone()
            .into_output_items()
            .iter()
            .map(|i| serde_json::to_value(i).unwrap())
            .collect(),
    );
    let assistant_message = json!({
        "id": message_id,
        "role": "assistant",
        "content": acc.content,
        "output": output,
        "done": true,
        "usage": acc.usage.map(|(p, c, t)| json!({
            "prompt_tokens": p, "completion_tokens": c,
            "total_tokens": t.unwrap_or(p + c),
        })),
        "model": model.id,
        "timestamp": Secs::now().as_i64(),
    });
    match rc_db::repo::chats::upsert_message_to_chat_by_id_and_message_id(
        &app.db,
        &chat_id,
        &message_id,
        &assistant_message,
    )
    .await
    {
        Ok(_) => {
            let usage = acc.usage.map(|(p, c, t)| {
                json!({
                    "prompt_tokens": p, "completion_tokens": c,
                    "total_tokens": t.unwrap_or(p + c),
                })
            });
            emit(event_data::message_done(
                &acc.content,
                output.clone(),
                usage,
            ));
            // ---- background title generation (first round of a new chat) ----
            if is_new_chat && !acc.content.is_empty() {
                let messages: Vec<Value> = form
                    .messages
                    .iter()
                    .map(|m| serde_json::to_value(m).unwrap_or(Value::Null))
                    .collect();
                let title_app = app.clone();
                let title_user = user_id.clone();
                let title_chat = chat_id.clone();
                let requested_model = form.model.clone();
                tokio::spawn(crate::routes::tasks::run_background_title(
                    title_app,
                    title_user,
                    title_chat,
                    requested_model,
                    messages,
                ));
            }
        }
        Err(e) => {
            tracing::warn!(chat_id = %chat_id, error = %e, "assistant persistence failed");
            emit(event_data::message_error(json!({"detail": e.to_string()})));
        }
    }
    emit(event_data::chat_active(false));
}

/// Persist the incoming user message + assistant placeholder, creating the
/// chat on first message (open-webui main.py pre-stream persistence).
async fn chats_persist_user_message(
    app: &AppState,
    owner_id: &str,
    chat_id: &str,
    user_message: Value,
    assistant_message_id: &str,
    model_name: &str,
) -> rc_core::Result<()> {
    let user_message_id = user_message
        .get("id")
        .and_then(Value::as_str)
        .map(str::to_string)
        .unwrap_or_else(|| Uuid::new_v4().to_string());

    let exists = rc_db::repo::chats::get_chat_by_id(&app.db, chat_id)
        .await?
        .is_some();
    if !exists {
        let blob = json!({
            "title": "New Chat",
            "models": [model_name],
            "history": {"messages": {}, "currentId": null},
        });
        rc_db::repo::chats::insert_new_chat(
            &app.db,
            rc_db::repo::chats::NewChatParams {
                id: chat_id,
                user_id: owner_id,
                chat: &blob,
                folder_id: None,
                variables: None,
                internal_meta: None,
                timer_at: None,
            },
        )
        .await?;
    }

    // user message → blob + row; assistant placeholder linked as child
    rc_db::repo::chats::upsert_message_to_chat_by_id_and_message_id(
        &app.db,
        chat_id,
        &user_message_id,
        &user_message,
    )
    .await?;
    let placeholder = json!({
        "id": assistant_message_id,
        "parentId": user_message_id,
        "role": "assistant",
        "content": "",
        "done": false,
        "model": model_name,
        "timestamp": Secs::now().as_i64(),
    });
    rc_db::repo::chats::upsert_message_to_chat_by_id_and_message_id(
        &app.db,
        chat_id,
        assistant_message_id,
        &placeholder,
    )
    .await?;
    Ok(())
}

/// Backend base URL for an ollama model (first backend serving it).
async fn ollama_base_for(app: &AppState, model: &rc_llm::ModelInfo) -> Option<String> {
    let urls: Vec<usize> = model
        .extra
        .get("urls")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(Value::as_u64)
                .map(|v| v as usize)
                .collect()
        })
        .unwrap_or_default();
    let base_urls: Vec<String> = app
        .config
        .get("ollama.base_urls")
        .await
        .ok()
        .flatten()
        .and_then(|v| v.as_array().cloned())
        .map(|a| {
            a.into_iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    urls.first().and_then(|idx| base_urls.get(*idx).cloned())
}

/// OpenAI target for a model (connection by url_idx, else the first).
async fn openai_target_for(app: &AppState, model: &rc_llm::ModelInfo) -> Option<ChatTarget> {
    let cfg: BackendConfig = backend_config(app).await;
    let idx = model.url_idx.unwrap_or(0);
    let conn = cfg.openai_connections.get(idx)?;
    Some(ChatTarget {
        base_url: conn.base_url.clone(),
        api_key: conn.api_key.clone(),
        extra_headers: Default::default(),
    })
}

/// `GET /ws` — native WebSocket; first frame must carry the JWT
/// (`{"token": …}` or `{"event":"auth","data":{"token": …}}`).
pub async fn ws_handler(State(app): State<AppState>, upgrade: WebSocketUpgrade) -> Response {
    upgrade.on_upgrade(move |socket| handle_socket(app, socket))
}

async fn handle_socket(app: AppState, mut socket: WebSocket) {
    // ---- auth handshake (first frame) ----
    let token = match socket.recv().await {
        Some(Ok(WsMessage::Text(text))) => {
            let Ok(v) = serde_json::from_str::<Value>(&text) else {
                return;
            };
            v.get("token")
                .and_then(Value::as_str)
                .map(str::to_string)
                .or_else(|| {
                    v.get("data")
                        .and_then(|d| d.get("token"))
                        .and_then(Value::as_str)
                        .map(str::to_string)
                })
        }
        _ => None,
    };
    let Some(token) = token else {
        let _ = socket
            .send(WsMessage::Text(
                json!({"event": "error", "data": {"detail": "auth required"}})
                    .to_string()
                    .into(),
            ))
            .await;
        return;
    };
    let claims = match rc_auth::decode_token(&token, &app.secret_key) {
        Ok(c) => c,
        Err(_) => {
            let _ = socket
                .send(WsMessage::Text(
                    json!({"event": "error", "data": {"detail": "invalid token"}})
                        .to_string()
                        .into(),
                ))
                .await;
            return;
        }
    };
    let user = match rc_db::repo::users::get_user_by_id(&app.db, &claims.id).await {
        Ok(Some(u)) => u,
        _ => {
            let _ = socket
                .send(WsMessage::Text(
                    json!({"event": "error", "data": {"detail": "invalid token"}})
                        .to_string()
                        .into(),
                ))
                .await;
            return;
        }
    };

    let (sid, mut rx) = app.hub.connect(&user.id);
    let _ = socket
        .send(WsMessage::Text(
            json!({"event": "connected", "data": {"user_id": user.id, "sid": sid}})
                .to_string()
                .into(),
        ))
        .await;

    loop {
        tokio::select! {
            frame = rx.recv() => {
                match frame {
                    Some(frame) => {
                        if socket.send(WsMessage::Text(frame.into())).await.is_err() {
                            break;
                        }
                    }
                    None => break,
                }
            }
            incoming = socket.recv() => {
                match incoming {
                    // heartbeats: reply with an ack frame
                    Some(Ok(WsMessage::Text(_))) => {
                        let _ = socket.send(WsMessage::Text(json!({"event": "heartbeat-ack"}).to_string().into())).await;
                    }
                    Some(Ok(WsMessage::Close(_))) | None => break,
                    Some(Ok(_)) => {} // binary/ping ignored in M1
                    Some(Err(_)) => break,
                }
            }
        }
    }
    app.hub.disconnect(sid);
}
