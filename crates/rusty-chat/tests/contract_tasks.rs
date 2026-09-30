//! Title-generation contract tests — `POST /api/v1/tasks/title/completions`
//! plus the automatic first-round background title after a new-chat
//! streaming completion, against a real server on an ephemeral port.
//!
//! 覆盖矩阵：
//! ✅ 匿名请求 → 401
//! ✅ task.title.enable=false → 200 + {"detail": "Title generation is
//!    disabled"}（open-webui 的 200 门语义）
//! ✅ model 缺失/空 → 400 + 原文 detail
//! ✅ 未知模型 → 404 + "Model not found"
//! ✅ 响应为 OpenAI completion JSON 透传（choices[0].message.content）
//! ✅ task.model.default 解析：配置后请求路由到任务模型（响应 model 字段）
//! ✅ prompt 模板渲染（mock 后端捕获请求体断言 "USER: …" 行与模型 id）
//! ✅ 自动触发：新聊天首轮完成后 chat:title WS 事件 + title 列落库
//! ✅ 二轮不再触发（chat:title 仅一次）
//! ⛔ 刻意不覆盖：task.model.external 路径（与 default 同一解析函数，
//!    单元语义已由 rc-core 锁定）；task.model.params 透传（apply 分支
//!    由 openai adapter 参数测试覆盖）；PG 方言（repo 层已由双方言
//!    集成测试覆盖）

use axum::Json;
use axum::body::Body;
use axum::extract::State;
use axum::http::{Request, StatusCode};
use futures::{SinkExt, StreamExt};
use serde_json::{Value, json};
use std::sync::Arc;

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
        secret_key: "tasks-contract-secret".to_string(),
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

type Capture = Arc<tokio::sync::Mutex<Vec<Value>>>;

/// Mock ollama backend with two models; title requests (detected by the
/// "### Task:" template marker) answer with a title JSON, everything else
/// with the plain chat response. All /api/chat bodies are captured.
async fn spawn_ollama_mock(capture: Capture) -> String {
    let app = axum::Router::new()
        .route(
            "/api/tags",
            axum::routing::get(|| async {
                axum::Json(json!({"models": [
                    {"name": "llama3:8b", "model": "llama3:8b", "digest": "d1", "size": 1},
                    {"name": "qwen:0.5b", "model": "qwen:0.5b", "digest": "d2", "size": 1}
                ]}))
            }),
        )
        .route(
            "/api/chat",
            axum::routing::post(
                |State(capture): State<Capture>, Json(body): Json<Value>| async move {
                    let content = if body.to_string().contains("### Task:") {
                        r#"{"title": "Greetings"}"#
                    } else {
                        "Hello world"
                    };
                    let line1 = json!({
                        "model": "m",
                        "message": {"role": "assistant", "content": content},
                        "done": false,
                    });
                    let line2 = json!({
                        "model": "m",
                        "message": {"role": "assistant", "content": ""},
                        "done": true,
                        "prompt_eval_count": 2,
                        "eval_count": 4,
                    });
                    capture.lock().await.push(body);
                    let payload = format!("{}\n{}\n", line1, line2);
                    axum::http::Response::builder()
                        .header("content-type", "application/x-ndjson")
                        .body(axum::body::Body::from(payload))
                        .unwrap()
                },
            ),
        )
        .with_state(capture);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    format!("http://{addr}")
}

async fn signup_token(base: &str) -> String {
    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{base}/api/v1/auths/signup"))
        .json(&json!({"name": "U", "email": "u-tasks@example.com", "password": "pw-tasks-123"}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let signup: Value = resp.json().await.unwrap();
    signup["token"].as_str().unwrap().to_string()
}

async fn serve(router: axum::Router) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    format!("http://127.0.0.1:{port}")
}

#[tokio::test]
async fn title_endpoint_contract() {
    let (router, app_state, _dir) = test_app().await;
    let capture: Capture = Arc::new(tokio::sync::Mutex::new(Vec::new()));
    let ollama_url = spawn_ollama_mock(capture.clone()).await;
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
    let base = serve(router).await;
    let client = reqwest::Client::new();
    let token = signup_token(&base).await;

    let post_title = |token: &str, body: Value| {
        let client = client.clone();
        let base = base.clone();
        let token = token.to_string();
        async move {
            client
                .post(format!("{base}/api/v1/tasks/title/completions"))
                .bearer_auth(&token)
                .json(&body)
                .send()
                .await
                .unwrap()
        }
    };

    // anonymous → 401
    let resp = reqwest::Client::new()
        .post(format!("{base}/api/v1/tasks/title/completions"))
        .json(&json!({"model": "llama3:8b", "messages": []}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);

    // empty model → 400 with the open-webui detail text
    let resp = post_title(&token, json!({"model": "", "messages": []})).await;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let body: Value = resp.json().await.unwrap();
    assert_eq!(
        body["detail"],
        json!(
            "No model specified for title generation. Please ensure a model is selected for this chat."
        )
    );

    // unknown model → 404
    let resp = post_title(&token, json!({"model": "ghost", "messages": []})).await;
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    let body: Value = resp.json().await.unwrap();
    assert_eq!(body["detail"], json!("Model not found"));

    // disabled gate → 200 + detail body
    app_state
        .config
        .upsert("task.title.enable", &json!(false))
        .await
        .unwrap();
    let resp = post_title(
        &token,
        json!({"model": "llama3:8b", "messages": [{"role": "user", "content": "hi"}]}),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let body: Value = resp.json().await.unwrap();
    assert_eq!(body["detail"], json!("Title generation is disabled"));
    app_state
        .config
        .upsert("task.title.enable", &json!(true))
        .await
        .unwrap();

    // valid request → OpenAI completion passthrough
    let resp = post_title(
        &token,
        json!({"model": "llama3:8b", "messages": [{"role": "user", "content": "say hello"}]}),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let body: Value = resp.json().await.unwrap();
    assert_eq!(body["object"], json!("chat.completion"));
    assert_eq!(body["model"], json!("llama3:8b"));
    assert_eq!(
        body["choices"][0]["message"]["content"],
        json!(r#"{"title": "Greetings"}"#)
    );

    // template rendering reached the backend
    let captured = capture.lock().await;
    let title_request = captured
        .iter()
        .find(|b| b.to_string().contains("### Task:"))
        .expect("title template request must reach the backend");
    assert!(
        title_request.to_string().contains("USER: say hello"),
        "MESSAGES:END:2 rendering missing: {title_request}"
    );
}

#[tokio::test]
async fn title_endpoint_routes_to_task_model() {
    let (router, app_state, _dir) = test_app().await;
    let capture: Capture = Arc::new(tokio::sync::Mutex::new(Vec::new()));
    let ollama_url = spawn_ollama_mock(capture.clone()).await;
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
    app_state
        .config
        .upsert("task.model.default", &json!("qwen:0.5b"))
        .await
        .unwrap();
    let base = serve(router).await;
    let client = reqwest::Client::new();
    let token = signup_token(&base).await;

    let resp = client
        .post(format!("{base}/api/v1/tasks/title/completions"))
        .bearer_auth(&token)
        .json(&json!({
            "model": "llama3:8b",
            "messages": [{"role": "user", "content": "say hello"}]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body: Value = resp.json().await.unwrap();
    // task model resolution: the completion must carry the task model id
    assert_eq!(body["model"], json!("qwen:0.5b"));
}

#[tokio::test]
async fn background_title_after_first_round_only() {
    let (router, app_state, _dir) = test_app().await;
    let capture: Capture = Arc::new(tokio::sync::Mutex::new(Vec::new()));
    let ollama_url = spawn_ollama_mock(capture.clone()).await;
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

    let base = serve(router).await;
    let client = reqwest::Client::new();
    let token = signup_token(&base).await;

    let (mut ws, _) =
        tokio_tungstenite::connect_async(format!("ws://{}/ws", base.trim_start_matches("http://")))
            .await
            .unwrap();
    ws.send(tokio_tungstenite::tungstenite::Message::text(
        json!({"token": token}).to_string(),
    ))
    .await
    .unwrap();
    ws.next().await; // connected

    // first round → new chat. The web frontend ALWAYS sends a client-generated
    // chat_id, so "first round" must be detected by chat creation, not by an
    // absent chat_id (regression guard for exactly that bug).
    let resp = client
        .post(format!("{base}/api/chat/completions"))
        .bearer_auth(&token)
        .json(&json!({
            "model": "llama3:8b",
            "messages": [{"role": "user", "content": "say hello"}],
            "stream": true,
            "id": "assistant-msg-1",
            "chat_id": "client-generated-chat-id",
            "user_message": {"id": "user-msg-1", "role": "user", "content": "say hello"}
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let envelope: Value = resp.json().await.unwrap();
    let chat_id = envelope["chat_id"].as_str().unwrap().to_string();

    // collect frames until the chat:title event (background task timing)
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    let mut saw_done = false;
    let mut title: Option<String> = None;
    while std::time::Instant::now() < deadline {
        let frame = tokio::time::timeout(std::time::Duration::from_secs(5), ws.next()).await;
        let Ok(Some(Ok(msg))) = frame else { break };
        let frame: Value = serde_json::from_str(msg.to_text().unwrap()).unwrap();
        if frame["event"] != json!("events") {
            continue;
        }
        let data = &frame["data"]["data"];
        match data["type"].as_str() {
            Some("chat:message") if data["data"].get("done") == Some(&json!(true)) => {
                saw_done = true;
            }
            Some("chat:title") => {
                title = Some(
                    data["data"]
                        .as_str()
                        .expect("chat:title data must be the title string")
                        .to_string(),
                );
                break;
            }
            Some("chat:message:error") => panic!("unexpected error event: {data}"),
            _ => {}
        }
    }
    assert!(saw_done, "first round must complete");
    assert_eq!(
        title.as_deref(),
        Some("Greetings"),
        "chat:title must arrive"
    );

    // title persisted on the chat row
    let chat = rc_db::repo::chats::get_chat_by_id(&app_state.db, &chat_id)
        .await
        .unwrap()
        .expect("chat exists");
    assert_eq!(chat.title.as_deref(), Some("Greetings"));

    // second round on the same chat → no further chat:title
    let resp = client
        .post(format!("{base}/api/chat/completions"))
        .bearer_auth(&token)
        .json(&json!({
            "model": "llama3:8b",
            "messages": [
                {"role": "user", "content": "say hello"},
                {"role": "assistant", "content": "Hello world"},
                {"role": "user", "content": "again"}
            ],
            "stream": true,
            "id": "assistant-msg-2",
            "chat_id": chat_id,
            "user_message": {"id": "user-msg-2", "role": "user", "content": "again"}
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    let mut second_done = false;
    while std::time::Instant::now() < deadline {
        let frame = tokio::time::timeout(std::time::Duration::from_secs(5), ws.next()).await;
        let Ok(Some(Ok(msg))) = frame else { break };
        let frame: Value = serde_json::from_str(msg.to_text().unwrap()).unwrap();
        if frame["event"] != json!("events") {
            continue;
        }
        let data = &frame["data"]["data"];
        match data["type"].as_str() {
            Some("chat:message") if data["data"].get("done") == Some(&json!(true)) => {
                second_done = true;
                break;
            }
            Some("chat:title") => panic!("chat:title must not be re-emitted on follow-ups"),
            _ => {}
        }
    }
    assert!(second_done, "second round must complete");
}

// shared oneshot helpers (kept for parity with the other contract files)
#[allow(dead_code)]
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
