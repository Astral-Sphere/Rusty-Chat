//! Chat pipeline EDGE battery — stream=false, invalid forms, upstream
//! failures, WS handshake negatives, and title-task suppression.
//!
//! 覆盖矩阵：
//! ✅ stream=false：同步 OpenAI chat.completion 形状（chatcmpl- 前缀/
//!    finish_reason/usage）；用户消息仍落库（blob 含 user+assistant 占位，
//!    占位带 model 字段）
//! ✅ 非法表单：缺 model → 400 "invalid chat form"；数组体 → 400
//! ✅ 上游失败（/api/chat 500）：WS 收到 chat:message:error + chat:active
//!    (false)；占位消息保持 done=false
//! ✅ WS 握手负路径：非 JSON 首帧直接关闭；坏 token → "invalid token"；
//!    无凭据 → "auth required"；备形 {"event":"auth","data":{"token"}} 可用；
//!    认证后任意文本 → heartbeat-ack
//! ✅ chat_id:"" → 服务端生成新 UUID（envelope 非空）；user_message 缺省
//!    时从 messages 最后一条 user 推导
//! ✅ 空 content（纯 thinking）完成 → 不触发后台标题任务（mock 计数）
//! ⛔ 刻意不覆盖：正常流式全链与标题生成正路径（contract_chat/contract_tasks
//!    已锁）；多用户事件隔离（Hub 单测 + run_generation 单房间投递）

use axum::http::StatusCode;
use futures::{SinkExt as _, StreamExt};
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
        secret_key: "chat-edges-secret".to_string(),
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

/// Serves the router on an ephemeral port and returns the base URL.
async fn serve(router: axum::Router) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    format!("127.0.0.1:{port}")
}

async fn signup(base: &str) -> String {
    let client = reqwest::Client::new();
    let resp = client
        .post(format!("http://{base}/api/v1/auths/signup"))
        .json(&json!({"name": "U", "email": "u@example.com", "password": "pw-edges-1"}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: Value = resp.json().await.unwrap();
    body["token"].as_str().unwrap().to_string()
}

/// Ollama mock: /api/tags lists llama3:8b; /api/chat answers with `status`
/// and the given ndjson lines joined with newlines.
async fn spawn_ollama(status: StatusCode, lines: Vec<String>) -> String {
    let app = axum::Router::new()
        .route(
            "/api/tags",
            axum::routing::get(|| async {
                axum::Json(json!({"models": [
                    {"name": "llama3:8b", "model": "llama3:8b", "digest": "d", "size": 1}
                ]}))
            }),
        )
        .route(
            "/api/chat",
            axum::routing::post(move || {
                let (status, lines) = (status, lines.clone());
                async move {
                    if status.is_success() {
                        axum::http::Response::builder()
                            .header("content-type", "application/x-ndjson")
                            .body(axum::body::Body::from(lines.join("\n") + "\n"))
                            .unwrap()
                    } else {
                        axum::http::Response::builder()
                            .status(status)
                            .body(axum::body::Body::from("upstream exploded"))
                            .unwrap()
                    }
                }
            }),
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    format!("http://{addr}")
}

fn lines(slice: &[&str]) -> Vec<String> {
    slice.iter().map(|s| s.to_string()).collect()
}

/// WS connect + auth handshake; returns the socket and the connected frame.
async fn ws_connect(base: &str, token: &str) -> (WsStream, Value) {
    use tokio_tungstenite::tungstenite::Message;
    let (mut ws, _) = tokio_tungstenite::connect_async(format!("ws://{base}/ws"))
        .await
        .unwrap();
    ws.send(Message::text(json!({"token": token}).to_string()))
        .await
        .unwrap();
    let connected = ws.next().await.unwrap().unwrap();
    let frame: Value = serde_json::from_str(connected.to_text().unwrap()).unwrap();
    (ws, frame)
}

type WsStream =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

/// Collects `events` payloads until chat:active(false) arrives.
async fn collect_until_inactive(ws: &mut WsStream) -> Vec<Value> {
    let mut events = Vec::new();
    loop {
        let frame = tokio::time::timeout(std::time::Duration::from_secs(5), ws.next()).await;
        let Ok(Some(Ok(msg))) = frame else {
            panic!("stream ended before chat:active(false)");
        };
        let parsed: Value = serde_json::from_str(msg.to_text().unwrap()).unwrap();
        if parsed["event"] == json!("events") {
            let inner = parsed["data"]["data"].clone();
            if inner["type"] == json!("chat:active") && inner["data"]["active"] == json!(false) {
                events.push(inner);
                break;
            }
            events.push(inner);
        }
    }
    events
}

async fn post_completion(base: &str, token: &str, body: Value) -> (StatusCode, Value) {
    let client = reqwest::Client::new();
    let resp = client
        .post(format!("http://{base}/api/chat/completions"))
        .bearer_auth(token)
        .json(&body)
        .send()
        .await
        .unwrap();
    let status = resp.status();
    (status, resp.json().await.unwrap())
}

#[tokio::test]
async fn stream_false_returns_sync_completion_and_persists() {
    let (router, app, _dir) = test_app().await;
    let ollama = spawn_ollama(
        StatusCode::OK,
        lines(&[
            r#"{"model":"m","message":{"role":"assistant","content":"Hello "},"done":false}"#,
            r#"{"model":"m","message":{"role":"assistant","content":"world"},"done":false}"#,
            r#"{"model":"m","message":{"role":"assistant","content":""},"done":true,"prompt_eval_count":2,"eval_count":3}"#,
        ]),
    )
    .await;
    app.config
        .upsert("ollama.enable", &json!(true))
        .await
        .unwrap();
    app.config
        .upsert("ollama.base_urls", &json!([ollama]))
        .await
        .unwrap();
    let base = serve(router).await;
    let token = signup(&base).await;

    let (status, body) = post_completion(
        &base,
        &token,
        json!({
            "model": "llama3:8b",
            "messages": [{"role": "user", "content": "say hello"}],
            "stream": false
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    // OpenAI chat.completion shape
    assert!(
        body["id"].as_str().unwrap().starts_with("chatcmpl-"),
        "{body}"
    );
    assert_eq!(body["object"], json!("chat.completion"));
    assert_eq!(body["choices"][0]["message"]["role"], json!("assistant"));
    assert_eq!(
        body["choices"][0]["message"]["content"],
        json!("Hello world")
    );
    assert_eq!(body["choices"][0]["finish_reason"], json!("stop"));
    assert_eq!(body["usage"]["total_tokens"], json!(5));

    // the user message + assistant placeholder were persisted pre-stream
    let client = reqwest::Client::new();
    let list: Value = client
        .get(format!("http://{base}/api/v1/chats/"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(list.as_array().unwrap().len(), 1, "chat created");
    let chat_id = list[0]["id"].as_str().unwrap().to_string();
    let chat: Value = client
        .get(format!("http://{base}/api/v1/chats/{chat_id}"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let messages = chat["chat"]["history"]["messages"].as_object().unwrap();
    assert_eq!(messages.len(), 2, "user + assistant placeholder");
    let assistant = messages
        .values()
        .find(|m| m["role"] == json!("assistant"))
        .unwrap();
    assert_eq!(assistant["done"], json!(false), "placeholder state");
    assert_eq!(assistant["model"], json!("llama3:8b"));
}

#[tokio::test]
async fn invalid_form_returns_400() {
    let (router, app, _dir) = test_app().await;
    let ollama = spawn_ollama(StatusCode::OK, lines(&[])).await;
    app.config
        .upsert("ollama.enable", &json!(true))
        .await
        .unwrap();
    app.config
        .upsert("ollama.base_urls", &json!([ollama]))
        .await
        .unwrap();
    let base = serve(router).await;
    let token = signup(&base).await;

    // missing model (the only required field)
    let (status, body) = post_completion(&base, &token, json!({"messages": []})).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(body["detail"], json!("invalid chat form"));

    // non-object body
    let client = reqwest::Client::new();
    let resp = client
        .post(format!("http://{base}/api/chat/completions"))
        .bearer_auth(&token)
        .header("content-type", "application/json")
        .body("[1,2,3]")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

    // nothing was persisted
    let list: Value = client
        .get(format!("http://{base}/api/v1/chats/"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(list.as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn upstream_failure_emits_error_event() {
    let (router, app, _dir) = test_app().await;
    let ollama = spawn_ollama(StatusCode::INTERNAL_SERVER_ERROR, vec![]).await;
    app.config
        .upsert("ollama.enable", &json!(true))
        .await
        .unwrap();
    app.config
        .upsert("ollama.base_urls", &json!([ollama]))
        .await
        .unwrap();
    let base = serve(router).await;
    let token = signup(&base).await;

    let (mut ws, connected) = ws_connect(&base, &token).await;
    assert_eq!(connected["event"], json!("connected"));

    let (status, envelope) = post_completion(
        &base,
        &token,
        json!({
            "model": "llama3:8b",
            "messages": [{"role": "user", "content": "hi"}],
            "stream": true
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{envelope}");
    let chat_id = envelope["chat_id"].as_str().unwrap().to_string();

    let events = collect_until_inactive(&mut ws).await;
    let types: Vec<&str> = events.iter().filter_map(|e| e["type"].as_str()).collect();
    assert!(
        types.contains(&"chat:message:error"),
        "expected an error event: {types:?}"
    );
    // active(true) came first, error in the middle, active(false) last
    assert_eq!(types[0], "chat:active");
    assert_eq!(*types.last().unwrap(), "chat:active");

    // the placeholder stays done=false (generation never completed)
    let client = reqwest::Client::new();
    let chat: Value = client
        .get(format!("http://{base}/api/v1/chats/{chat_id}"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let assistant = chat["chat"]["history"]["messages"]
        .as_object()
        .unwrap()
        .values()
        .find(|m| m["role"] == json!("assistant"))
        .unwrap();
    assert_eq!(assistant["done"], json!(false));
}

#[tokio::test]
async fn ws_handshake_negative_paths() {
    let (router, _app, _dir) = test_app().await;
    let base = serve(router).await;
    use tokio_tungstenite::tungstenite::Message;
    let url = format!("ws://{base}/ws");

    // non-JSON first frame → closed without a connected frame
    let (mut ws, _) = tokio_tungstenite::connect_async(&url).await.unwrap();
    ws.send(Message::text("not json")).await.unwrap();
    let outcome = tokio::time::timeout(std::time::Duration::from_secs(3), ws.next()).await;
    match outcome {
        Err(_) => panic!("server should close the socket"),
        Ok(Some(Ok(Message::Text(t)))) => {
            let v: Value = serde_json::from_str(&t).unwrap();
            assert_ne!(v["event"], json!("connected"), "must not authenticate");
        }
        Ok(Some(Ok(_))) | Ok(None) => {}
        Ok(Some(Err(_))) => {}
    }

    // garbage token → "invalid token"
    let (mut ws, _) = tokio_tungstenite::connect_async(&url).await.unwrap();
    ws.send(Message::text(json!({"token": "garbage"}).to_string()))
        .await
        .unwrap();
    let msg = tokio::time::timeout(std::time::Duration::from_secs(3), ws.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let v: Value = serde_json::from_str(msg.to_text().unwrap()).unwrap();
    assert_eq!(v["event"], json!("error"));
    assert_eq!(v["data"]["detail"], json!("invalid token"));

    // no credentials at all → "auth required"
    let (mut ws, _) = tokio_tungstenite::connect_async(&url).await.unwrap();
    ws.send(Message::text(json!({"data": {}}).to_string()))
        .await
        .unwrap();
    let msg = tokio::time::timeout(std::time::Duration::from_secs(3), ws.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let v: Value = serde_json::from_str(msg.to_text().unwrap()).unwrap();
    assert_eq!(v["data"]["detail"], json!("auth required"));

    // alternate handshake shape {"event":"auth","data":{"token"}} works —
    // but only with a VALID token (there is no user yet → invalid token)
    let (mut ws, _) = tokio_tungstenite::connect_async(&url).await.unwrap();
    ws.send(Message::text(
        json!({"event": "auth", "data": {"token": "garbage"}}).to_string(),
    ))
    .await
    .unwrap();
    let msg = tokio::time::timeout(std::time::Duration::from_secs(3), ws.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let v: Value = serde_json::from_str(msg.to_text().unwrap()).unwrap();
    assert_eq!(v["data"]["detail"], json!("invalid token"));

    // full handshake + heartbeat on a live socket
    let token = signup(&base).await;
    let (mut ws, connected) = ws_connect(&base, &token).await;
    assert_eq!(connected["event"], json!("connected"));
    ws.send(Message::text("ping")).await.unwrap();
    let msg = tokio::time::timeout(std::time::Duration::from_secs(3), ws.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let v: Value = serde_json::from_str(msg.to_text().unwrap()).unwrap();
    assert_eq!(v["event"], json!("heartbeat-ack"));
}

#[tokio::test]
async fn empty_chat_id_and_derived_user_message() {
    let (router, app, _dir) = test_app().await;
    let ollama = spawn_ollama(
        StatusCode::OK,
        lines(&[
            r#"{"model":"m","message":{"role":"assistant","content":"ok"},"done":true,"prompt_eval_count":1,"eval_count":1}"#,
        ]),
    )
    .await;
    app.config
        .upsert("ollama.enable", &json!(true))
        .await
        .unwrap();
    app.config
        .upsert("ollama.base_urls", &json!([ollama]))
        .await
        .unwrap();
    let base = serve(router).await;
    let token = signup(&base).await;

    let (mut ws, _) = ws_connect(&base, &token).await;
    // chat_id:"" → server-side new chat; no user_message → derived from messages
    let (status, envelope) = post_completion(
        &base,
        &token,
        json!({
            "model": "llama3:8b",
            "messages": [{"role": "user", "content": "say hello"}],
            "stream": true,
            "chat_id": ""
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{envelope}");
    let chat_id = envelope["chat_id"].as_str().unwrap().to_string();
    assert!(
        !chat_id.is_empty(),
        "empty chat_id must generate a new UUID"
    );
    collect_until_inactive(&mut ws).await;

    let client = reqwest::Client::new();
    let chat: Value = client
        .get(format!("http://{base}/api/v1/chats/{chat_id}"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let messages = chat["chat"]["history"]["messages"].as_object().unwrap();
    let user = messages
        .values()
        .find(|m| m["role"] == json!("user"))
        .expect("user message derived from form.messages");
    assert_eq!(user["content"], json!("say hello"));
    assert!(
        user["id"].as_str().is_some_and(|s| !s.is_empty()),
        "derived user message gets a generated id"
    );
}

#[tokio::test]
async fn thinking_only_completion_triggers_no_title_task() {
    let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let calls_captured = calls.clone();
    let (router, app, _dir) = test_app().await;
    let ollama = {
        let calls = calls_captured.clone();
        let app = axum::Router::new()
            .route(
                "/api/tags",
                axum::routing::get(|| async {
                    axum::Json(json!({"models": [
                        {"name": "llama3:8b", "model": "llama3:8b", "digest": "d", "size": 1}
                    ]}))
                }),
            )
            .route(
                "/api/chat",
                axum::routing::post(move || {
                    let calls = calls.clone();
                    async move {
                        calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                        axum::body::Body::from(
                            concat!(
                                r#"{"model":"m","message":{"role":"assistant","thinking":"hmm"},"done":false}"#,
                                "\n",
                                r#"{"model":"m","message":{"role":"assistant","content":""},"done":true,"prompt_eval_count":1,"eval_count":1}"#,
                                "\n",
                            ),
                        )
                    }
                }),
            );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        format!("http://{addr}")
    };
    app.config
        .upsert("ollama.enable", &json!(true))
        .await
        .unwrap();
    app.config
        .upsert("ollama.base_urls", &json!([ollama]))
        .await
        .unwrap();
    let base = serve(router).await;
    let token = signup(&base).await;

    let (mut ws, _) = ws_connect(&base, &token).await;
    let (status, envelope) = post_completion(
        &base,
        &token,
        json!({
            "model": "llama3:8b",
            "messages": [{"role": "user", "content": "think only"}],
            "stream": true
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{envelope}");
    collect_until_inactive(&mut ws).await;

    // title tasks re-call /api/chat — give the background task a moment
    tokio::time::sleep(std::time::Duration::from_millis(700)).await;
    assert_eq!(
        calls.load(std::sync::atomic::Ordering::SeqCst),
        1,
        "empty assistant content must not spawn a title task"
    );
}
