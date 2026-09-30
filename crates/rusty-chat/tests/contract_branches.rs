//! Message-tree branching contract tests — editing a user message creates a
//! sibling branch, the backend maintains childrenIds/currentId, and the chat
//! update route can switch the active branch.
//!
//! 覆盖矩阵：
//! ✅ 两轮对话 → 链式树（root user → assistant → user → assistant），
//!    currentId 停在最后的 assistant
//! ✅ 编辑首轮用户消息重发（新 ids + parentId 指向原父节点）→ 根消息
//!    childrenIds 追加新分支；currentId 移到新 assistant；新 assistant
//!    内容由 mock 后端生成并持久化（blob + chat_message 行）
//! ✅ 旧分支保留（原 assistant 消息原样存在）
//! ✅ POST /api/v1/chats/{id} 局部更新 history.currentId → 切换分支
//! ✅ 局部更新不破坏 messages（history 深合并）
//! ⛔ 刻意不覆盖：越权编辑（chats 路由已覆盖 401 语义）；子树删除
//!    （rc-db history 单测覆盖）；前端 UI 交互（T5 浏览器冒烟）

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
        secret_key: "branch-contract-secret".to_string(),
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

/// Mock ollama backend — every chat answer is "Hello world".
async fn spawn_ollama_mock() -> String {
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
            axum::routing::post(|| async {
                let line1 = json!({"model": "m", "message": {"role": "assistant", "content": "Hello world"}, "done": false});
                let line2 = json!({"model": "m", "message": {"role": "assistant", "content": ""}, "done": true, "prompt_eval_count": 2, "eval_count": 4});
                axum::http::Response::builder()
                    .header("content-type", "application/x-ndjson")
                    .body(axum::body::Body::from(format!("{}\n{}\n", line1, line2)))
                    .unwrap()
            }),
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    format!("http://{addr}")
}

#[tokio::test]
async fn edit_creates_sibling_branch_and_switch_works() {
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

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    let base = format!("http://127.0.0.1:{port}");
    let client = reqwest::Client::new();

    let resp = client
        .post(format!("{base}/api/v1/auths/signup"))
        .json(&json!({"name": "U", "email": "u-branch@example.com", "password": "pw-branch-1"}))
        .send()
        .await
        .unwrap();
    let signup: Value = resp.json().await.unwrap();
    let token = signup["token"].as_str().unwrap().to_string();

    // disable title generation so the background task doesn't race assertions
    app_state
        .config
        .upsert("task.title.enable", &json!(false))
        .await
        .unwrap();

    let complete = |body: Value| {
        let client = client.clone();
        let base = base.clone();
        let token = token.clone();
        async move {
            client
                .post(format!("{base}/api/chat/completions"))
                .bearer_auth(&token)
                .json(&body)
                .send()
                .await
                .unwrap()
        }
    };
    // wait until `expected` is the currentId AND done (generation is async;
    // the placeholder upsert already moves currentId before content lands)
    let wait_current_id = |chat_id: String, expected: String| {
        let client = client.clone();
        let base = base.clone();
        let token = token.clone();
        async move {
            for _ in 0..50 {
                let chat: Value = client
                    .get(format!("{base}/api/v1/chats/{chat_id}"))
                    .bearer_auth(&token)
                    .send()
                    .await
                    .unwrap()
                    .json()
                    .await
                    .unwrap();
                let message = &chat["chat"]["history"]["messages"][expected.as_str()];
                if chat["chat"]["history"]["currentId"] == json!(expected)
                    && message["done"] == json!(true)
                {
                    return chat;
                }
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            }
            panic!("message {expected} never completed");
        }
    };

    // round 1
    let resp = complete(json!({
        "model": "llama3:8b",
        "messages": [{"role": "user", "content": "say hello"}],
        "stream": true,
        "id": "a1",
        "user_message": {"id": "u1", "role": "user", "content": "say hello"}
    }))
    .await;
    assert_eq!(resp.status(), 200);
    let envelope: Value = resp.json().await.unwrap();
    let chat_id = envelope["chat_id"].as_str().unwrap().to_string();
    wait_current_id(chat_id.clone(), "a1".into()).await;

    // round 2
    let resp = complete(json!({
        "model": "llama3:8b",
        "messages": [
            {"role": "user", "content": "say hello"},
            {"role": "assistant", "content": "Hello world"},
            {"role": "user", "content": "again"}
        ],
        "stream": true,
        "id": "a2",
        "chat_id": chat_id,
        "user_message": {"id": "u2", "parentId": "a1", "role": "user", "content": "again"}
    }))
    .await;
    assert_eq!(resp.status(), 200);
    wait_current_id(chat_id.clone(), "a2".into()).await;

    // linear tree so far
    let chat: Value = client
        .get(format!("{base}/api/v1/chats/{chat_id}"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let history = &chat["chat"]["history"];
    assert_eq!(history["messages"]["u1"]["childrenIds"], json!(["a1"]));
    assert_eq!(history["messages"]["a1"]["childrenIds"], json!(["u2"]));

    // edit the ROUND-1 user message → sibling branch off u1
    let resp = complete(json!({
        "model": "llama3:8b",
        "messages": [{"role": "user", "content": "say hello EDITED"}],
        "stream": true,
        "id": "a1-edited",
        "chat_id": chat_id,
        "user_message": {
            "id": "u1-edited",
            "parentId": null,
            "role": "user",
            "content": "say hello EDITED"
        }
    }))
    .await;
    assert_eq!(resp.status(), 200);
    let chat = wait_current_id(chat_id.clone(), "a1-edited".into()).await;
    let history = &chat["chat"]["history"];

    // editing a root message creates a sibling ROOT (shared null parentId);
    // the old branch is untouched
    assert_eq!(history["messages"]["u1"]["childrenIds"], json!(["a1"]));
    assert_eq!(
        history["messages"]["u1-edited"]["childrenIds"],
        json!(["a1-edited"])
    );
    assert_eq!(history["messages"]["u1"]["content"], json!("say hello"));
    assert_eq!(history["messages"]["a1"]["content"], json!("Hello world"));
    assert_eq!(
        history["messages"]["u1-edited"]["content"],
        json!("say hello EDITED")
    );
    assert_eq!(
        history["messages"]["a1-edited"]["content"],
        json!("Hello world")
    );
    assert_eq!(history["messages"]["a1-edited"]["done"], json!(true));

    // assistant row persisted for the new branch
    let row = rc_db::repo::chat_messages::get_message_by_id(
        &app_state.db,
        &format!("{chat_id}-a1-edited"),
    )
    .await
    .unwrap();
    assert!(row.is_some(), "edited branch assistant row persisted");

    // switch back to the original branch via a partial chat update
    let resp = client
        .post(format!("{base}/api/v1/chats/{chat_id}"))
        .bearer_auth(&token)
        .json(&json!({"chat": {"history": {"currentId": "a1"}}}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let chat: Value = client
        .get(format!("{base}/api/v1/chats/{chat_id}"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let history = &chat["chat"]["history"];
    assert_eq!(history["currentId"], json!("a1"));
    // deep merge kept the whole tree
    assert_eq!(history["messages"]["u1"]["childrenIds"], json!(["a1"]));
    assert_eq!(
        history["messages"]["a1-edited"]["content"],
        json!("Hello world")
    );
}
