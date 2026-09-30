//! `POST /api/v1/tasks/title/completions` — the title-generation task
//! endpoint, field-for-field aligned with open-webui `routers/tasks.py`
//! `generate_title` (0.11.3): plain-dict form `{model, messages, chat_id?}`,
//! OpenAI-shaped completion passthrough, `task.model.default/external`
//! resolution, and the same error/degradation semantics.

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use rc_core::chat::{ChatCompletionForm, ChatMessage};
use rc_core::events::event_data;
use rc_core::tasks;
use rc_llm::registry::all_base_models;
use serde_json::{Value, json};

use crate::extract::VerifiedUser;
use crate::routes::chat::{complete_sync, task_model_id_for};
use crate::routes::models::backend_config;
use crate::state::AppState;

/// Task-model params from config with open-webui's null/empty filtering
/// (`get_task_model_generation_config`).
pub async fn task_model_params(app: &AppState) -> Option<Value> {
    let raw = app.config.get("task.model.params").await.ok().flatten()?;
    let obj = raw.as_object()?;
    let filtered: serde_json::Map<String, Value> = obj
        .iter()
        .filter(|(_, v)| !(v.is_null() || v.as_str() == Some("")))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    if filtered.is_empty() {
        None
    } else {
        Some(Value::Object(filtered))
    }
}

/// `POST /api/v1/tasks/title/completions`.
pub async fn generate_title(
    State(app): State<AppState>,
    VerifiedUser(_user): VerifiedUser,
    Json(form): Json<Value>,
) -> Response {
    let enabled = app
        .config
        .get("task.title.enable")
        .await
        .ok()
        .flatten()
        .and_then(|v| v.as_bool())
        .unwrap_or(true);
    if !enabled {
        // open-webui returns 200 with a detail body on this gate
        return (
            StatusCode::OK,
            Json(json!({"detail": "Title generation is disabled"})),
        )
            .into_response();
    }

    let model_id = form
        .get("model")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    if model_id.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"detail": "No model specified for title generation. Please ensure a model is selected for this chat."})),
        )
            .into_response();
    }

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
    if !models.iter().any(|m| m.id == model_id) {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({"detail": "Model not found"})),
        )
            .into_response();
    }

    let messages: Vec<Value> = form
        .get("messages")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let template = app
        .config
        .get("task.title.prompt_template")
        .await
        .ok()
        .flatten()
        .and_then(|v| v.as_str().map(str::to_string))
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| tasks::DEFAULT_TITLE_GENERATION_PROMPT_TEMPLATE.to_string());
    let content = tasks::render_title_template(&template, &messages);

    let task_model_id = task_model_id_for(&app, &model_id, &models).await;
    let task_model = models.iter().find(|m| m.id == task_model_id).cloned();
    let Some(task_model) = task_model else {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({"detail": "Model not found"})),
        )
            .into_response();
    };

    let chat_form = ChatCompletionForm {
        model: task_model_id,
        messages: vec![ChatMessage {
            role: "user".into(),
            content: json!(content),
            ..Default::default()
        }],
        stream: Some(false),
        params: task_model_params(&app).await,
        chat_id: form
            .get("chat_id")
            .and_then(Value::as_str)
            .map(str::to_string),
        ..Default::default()
    };

    match complete_sync(&app, &task_model, &chat_form).await {
        Ok(body) => Json(body).into_response(),
        Err(_) => (
            StatusCode::BAD_REQUEST,
            Json(json!({"detail": "An internal error has occurred."})),
        )
            .into_response(),
    }
}

/// Background title generation after the first round of a new chat
/// (open-webui `background_tasks_handler` title path): generate → parse →
/// persist title → `chat:title` event.
pub async fn run_background_title(
    app: AppState,
    user_id: String,
    chat_id: String,
    requested_model_id: String,
    messages: Vec<Value>,
) {
    let emit = |data: Value| {
        let frame = rc_core::events::WsFrame::chat_event(&chat_id, None, data);
        app.hub
            .send_to_user(&user_id, &serde_json::to_string(&frame).unwrap_or_default());
    };

    let enabled = app
        .config
        .get("task.title.enable")
        .await
        .ok()
        .flatten()
        .and_then(|v| v.as_bool())
        .unwrap_or(true);
    if !enabled {
        return;
    }

    let cfg = backend_config(&app).await;
    let Ok(models) = all_base_models(&cfg).await else {
        return;
    };
    if !models.iter().any(|m| m.id == requested_model_id) {
        return;
    }
    let task_model_id = task_model_id_for(&app, &requested_model_id, &models).await;
    let Some(task_model) = models.iter().find(|m| m.id == task_model_id).cloned() else {
        return;
    };

    let template = app
        .config
        .get("task.title.prompt_template")
        .await
        .ok()
        .flatten()
        .and_then(|v| v.as_str().map(str::to_string))
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| tasks::DEFAULT_TITLE_GENERATION_PROMPT_TEMPLATE.to_string());
    let content = tasks::render_title_template(&template, &messages);

    let chat_form = ChatCompletionForm {
        model: task_model_id,
        messages: vec![ChatMessage {
            role: "user".into(),
            content: json!(content),
            ..Default::default()
        }],
        stream: Some(false),
        params: task_model_params(&app).await,
        chat_id: Some(chat_id.clone()),
        ..Default::default()
    };

    let Ok(response) = complete_sync(&app, &task_model, &chat_form).await else {
        return;
    };
    let response_message = &response["choices"][0]["message"];
    let title_source = response_message
        .get("content")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .or_else(|| {
            response_message
                .get("reasoning_content")
                .and_then(Value::as_str)
        })
        .unwrap_or_default();

    let first_message_content = messages.first().and_then(tasks::message_content);
    let user_message = tasks::last_user_message(&messages)
        .as_deref()
        .map(tasks::truncate_title_fallback);
    let title = tasks::extract_title(
        title_source,
        first_message_content.as_deref(),
        user_message.as_deref(),
    );
    if title.is_empty() {
        return;
    }

    if let Err(e) = rc_db::repo::chats::update_chat_title_by_id(&app.db, &chat_id, &title).await {
        tracing::warn!(chat_id = %chat_id, error = %e, "title persistence failed");
        return;
    }
    emit(event_data::chat_title(&title));
}
