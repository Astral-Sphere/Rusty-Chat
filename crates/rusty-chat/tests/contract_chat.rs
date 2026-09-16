//! Chat pipeline contract test — full flow against a real server on an
//! ephemeral port: HTTP signup → WS connect (auth handshake) →
//! POST /api/chat/completions → collect WS `events` frames → verify
//! streamed deltas and persisted assistant message.
//!
//! 覆盖矩阵：
//! ✅ envelope 形状 {status, task_ids, chat_id}
//! ✅ WS auth 握手（首帧 token）→ connected 确认
//! ✅ WS 事件序列：chat:active(true) → chat:message:delta×N →
//!    chat:message(最终替换, done) → chat:active(false)
//! ✅ 持久化：助手消息写入 chat blob + chat_message 行（repo 验证）
//! ✅ 未配置模型 → 404 Model not found
//! ⛔ 刻意不覆盖：后台任务（标题/标签，M3）、工具调用循环（M6）、
//!    多模型 message_ids（M3）

use axum::body::Body;
use axum::http::{Request, StatusCode};
use futures::{SinkExt, StreamExt};
use serde_json::{Value, json};

use rusty_chat::state::AppState;

async fn test_app() -> (axum::Router, AppState, tempfile::TempDir) {
    rc_db::install_drivers();
    let dir = tempfile::tempdir().unwrap();
    let url = format!("sqlite://{}?mode=rwc", dir.path().join("test.db").display());
    let mut conn = sqlx::AnyPool::connect(&url)
        .await
        .unwrap()
        .acquire()
        .await
        .unwrap();
    rc_db::bootstrap::bootstrap(&mut conn, rc_db::bootstrap::Dialect::Sqlite)
        .await
        .unwrap();
    drop(conn);

    let db = sea_orm::Database::connect(&url).await.unwrap();
    let config = std::sync::Arc::new(rc_db::repo::config::ConfigEngine::new(db.clone()));
    config
        .seed_defaults(&rusty_chat::defaults::default_config())
        .await
        .unwrap();

    let state = AppState {
        db,
        config,
        secret_key: "chat-contract-secret".to_string(),
        webui_name: "Open WebUI".to_string(),
        version: env!("CARGO_PKG_VERSION"),
        placeholder_hash: std::sync::Arc::new(rc_auth::placeholder_hash()),
        webui_auth: true,
        hub: std::sync::Arc::new(rc_realtime::Hub::new()),
    };
    (
        rusty_chat::build_router(state.clone(), dir.path()),
        state,
        dir,
    )
}

/// Mock ollama `/api/chat` ndjson backend.
async fn spawn_ollama_mock() -> String {
    let ndjson = concat!(
        r#"{"model":"m","message":{"role":"assistant","content":"Hel"},"done":false}"#,
        "\n",
        r#"{"model":"m","message":{"role":"assistant","content":"lo world"},"done":false}"#,
        "\n",
        r#"{"model":"m","message":{"role":"assistant","thinking":"thinking hard"},"done":false}"#,
        "\n",
        r#"{"model":"m","message":{"role":"assistant","content":""},"done":true,"prompt_eval_count":2,"eval_count":4}"#,
        "\n"
    );
    let app = axum::Router::new()
        .route(
            "/api/tags",
            axum::routing::get(|| async {
                axum::Json(json!({"models": [
                    {"name": "llama3:8b", "model": "llama3:8b", "digest": "d1", "size": 1}
                ]}))
            }),
        )
        .route(
            "/api/chat",
            axum::routing::post(move || {
                let ndjson = ndjson.to_string();
                async move {
                    axum::http::Response::builder()
                        .header("content-type", "application/x-ndjson")
                        .body(axum::body::Body::from(ndjson))
                        .unwrap()
                }
            }),
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    format!("http://{addr}")
}

#[tokio::test]
async fn chat_completion_streams_via_ws_and_persists() {
    let (router, app_state, _dir) = test_app().await;
    let ollama_url = spawn_ollama_mock().await;
    app_state
        .config
        .upsert("ollama.enable", &json!(true))
        .await
        .unwrap();
    app_state
        .config
        .upsert("ollama.base_urls", &json!([ollama_url]))
        .await
        .unwrap();

    // serve on ephemeral port
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    let base = format!("http://127.0.0.1:{port}");

    // signup → token
    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{base}/api/v1/auths/signup"))
        .json(&json!({"name": "U", "email": "u@example.com", "password": "pw-chat-123"}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let signup: Value = resp.json().await.unwrap();
    let token = signup["token"].as_str().unwrap().to_string();

    // WS connect + auth handshake
    let (mut ws, _) = tokio_tungstenite::connect_async(format!("ws://127.0.0.1:{port}/ws"))
        .await
        .unwrap();
    ws.send(tokio_tungstenite::tungstenite::Message::text(
        json!({"token": token}).to_string(),
    ))
    .await
    .unwrap();
    let connected = ws.next().await.unwrap().unwrap();
    let connected_json: Value = serde_json::from_str(connected.to_text().unwrap()).unwrap();
    assert_eq!(connected_json["event"], json!("connected"));

    // POST /api/chat/completions
    let resp = client
        .post(format!("{base}/api/chat/completions"))
        .bearer_auth(&token)
        .json(&json!({
            "model": "llama3:8b",
            "messages": [{"role": "user", "content": "say hello"}],
            "stream": true,
            "id": "assistant-msg-1",
            "parent_id": null,
            "user_message": {"id": "user-msg-1", "role": "user", "content": "say hello"}
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let envelope: Value = resp.json().await.unwrap();
    assert_eq!(envelope["status"], json!(true));
    let chat_id = envelope["chat_id"].as_str().unwrap().to_string();
    assert!(envelope["task_ids"].as_array().unwrap().len() == 1);
    assert!(!chat_id.is_empty());

    // collect events until generation completes
    let mut deltas = String::new();
    let mut saw_active_true = false;
    let mut saw_final_replace = false;
    let mut saw_active_false = false;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while std::time::Instant::now() < deadline {
        let frame = tokio::time::timeout(std::time::Duration::from_secs(5), ws.next()).await;
        let Ok(Some(Ok(msg))) = frame else { break };
        let text = msg.to_text().unwrap();
        let frame: Value = serde_json::from_str(text).unwrap();
        if frame["event"] != json!("events") {
            continue;
        }
        let data = &frame["data"]["data"];
        match data["type"].as_str() {
            Some("chat:active") => {
                if data["data"]["active"] == json!(true) {
                    saw_active_true = true;
                } else {
                    saw_active_false = true;
                    break;
                }
            }
            Some("chat:message:delta") => {
                deltas.push_str(data["data"]["content"].as_str().unwrap());
            }
            Some("chat:message") => {
                if data["data"].get("done") == Some(&json!(true)) {
                    saw_final_replace = true;
                    assert_eq!(data["data"]["content"], json!("Hello world"));
                    assert!(data["data"]["output"].is_array(), "output items present");
                }
            }
            Some("chat:message:error") => panic!("unexpected error event: {data}"),
            _ => {}
        }
    }
    assert!(saw_active_true, "chat:active true must be emitted first");
    assert_eq!(deltas, "Hello world", "deltas must arrive in order");
    assert!(
        saw_final_replace,
        "final replace with done flag must be emitted"
    );
    assert!(saw_active_false, "chat:active false must end generation");

    // persistence: assistant message in blob + chat_message row
    let persisted = rc_db::repo::chats::get_chat_by_id(&app_state.db, &chat_id)
        .await
        .unwrap()
        .expect("chat created");
    let blob = persisted.chat.unwrap();
    assert_eq!(
        blob["history"]["messages"]["assistant-msg-1"]["content"],
        json!("Hello world")
    );
    assert_eq!(
        blob["history"]["messages"]["assistant-msg-1"]["done"],
        json!(true)
    );
    let row = rc_db::repo::chat_messages::get_message_by_id(
        &app_state.db,
        &format!("{chat_id}-assistant-msg-1"),
    )
    .await
    .unwrap();
    assert!(row.is_some(), "composite id row persisted");
    assert_eq!(row.unwrap().content, Some(json!("Hello world")));
}

#[tokio::test]
async fn unknown_model_returns_404() {
    let (mut router, _app, _dir) = test_app().await;
    let (status, _body, _) = call(
        &mut router,
        json_request(
            "POST",
            "/api/chat/completions",
            json!({
                "model": "ghost", "messages": [{"role": "user", "content": "x"}]
            }),
        ),
    )
    .await;
    // unauthenticated requests fail first
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    let (_, auth, _) = call(
        &mut router,
        json_request(
            "POST",
            "/api/v1/auths/signup",
            json!({
                "name": "U", "email": "u@example.com", "password": "pw-chat-123"
            }),
        ),
    )
    .await;
    let token = auth["token"].as_str().unwrap().to_string();
    let mut request = Request::builder()
        .method("POST")
        .uri("/api/chat/completions")
        .header("authorization", format!("Bearer {token}"))
        .header("content-type", "application/json")
        .body(Body::from(
            json!({
                "model": "ghost", "messages": [{"role": "user", "content": "x"}]
            })
            .to_string(),
        ))
        .unwrap();
    let (status, body, _) = call(
        &mut router,
        std::mem::replace(
            &mut request,
            Request::builder().body(Body::empty()).unwrap(),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["detail"], json!("Model not found"));
}

// shared helpers (duplicated from contract_auth to keep test binaries independent)
async fn call(
    router: &mut axum::Router,
    request: Request<Body>,
) -> (StatusCode, Value, Option<String>) {
    use http_body_util::BodyExt as _;
    use tower::ServiceExt as _;
    let response = router.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let set_cookie = response
        .headers()
        .get(axum::http::header::SET_COOKIE)
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let body = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes)
            .unwrap_or(Value::String(String::from_utf8_lossy(&bytes).into()))
    };
    (status, body, set_cookie)
}

fn json_request(method: &str, uri: &str, body: Value) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri(uri)
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap()
}
