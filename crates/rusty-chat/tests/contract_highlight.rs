//! Highlight endpoint contract tests — `POST /api/v1/utils/highlight` and
//! `GET /api/v1/utils/highlight/languages` against a real server.
//!
//! 覆盖矩阵：
//! ✅ 匿名请求 → 401
//! ✅ rust 代码 → unsupported=false，span 三元组格式 + 字节偏移切片回读
//! ✅ 未知语言 → unsupported=true + 空 spans
//! ✅ 空代码 → 200 + 空 spans（unsupported=false）
//! ✅ languages 端点列出编译通过的语言且无 errors
//! ⛔ 刻意不覆盖：前端回填 DOM（T5 浏览器冒烟）；每种语言的完整高亮
//!    质量（rc-highlight 单元测试已覆盖）

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
        secret_key: "hl-contract-secret".to_string(),
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

#[tokio::test]
async fn highlight_endpoint_contract() {
    let (router, _app, _dir) = test_app().await;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    let base = format!("http://127.0.0.1:{port}");
    let client = reqwest::Client::new();

    let resp = client
        .post(format!("{base}/api/v1/auths/signup"))
        .json(&json!({"name": "U", "email": "u-hl@example.com", "password": "pw-hl-123"}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let signup: Value = resp.json().await.unwrap();
    let token = signup["token"].as_str().unwrap().to_string();

    // anonymous → 401
    let resp = client
        .post(format!("{base}/api/v1/utils/highlight"))
        .json(&json!({"code": "fn main() {}", "language": "rust"}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 401);

    // rust code → spans with byte offsets
    let code = "fn main() {\n    let x = 1;\n}";
    let resp = client
        .post(format!("{base}/api/v1/utils/highlight"))
        .bearer_auth(&token)
        .json(&json!({"code": code, "language": "rust"}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: Value = resp.json().await.unwrap();
    assert_eq!(body["unsupported"], json!(false));
    assert_eq!(body["language"], json!("rust"));
    let spans = body["spans"].as_array().expect("spans array");
    assert!(!spans.is_empty());
    let mut saw_keyword = false;
    for triplet in spans {
        let parts = triplet.as_array().expect("triplet");
        assert_eq!(parts.len(), 3);
        let start = parts[0].as_u64().unwrap() as usize;
        let end = parts[1].as_u64().unwrap() as usize;
        let name = parts[2].as_str().unwrap();
        assert!(end > start);
        assert!(code.is_char_boundary(start) && code.is_char_boundary(end));
        if name == "keyword" && &code[start..end] == "fn" {
            saw_keyword = true;
        }
    }
    assert!(saw_keyword, "a keyword span must cover 'fn'");

    // unknown language → unsupported
    let resp = client
        .post(format!("{base}/api/v1/utils/highlight"))
        .bearer_auth(&token)
        .json(&json!({"code": "x", "language": "cobol"}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: Value = resp.json().await.unwrap();
    assert_eq!(body["unsupported"], json!(true));
    assert_eq!(body["spans"], json!([]));

    // empty code → supported, empty spans
    let resp = client
        .post(format!("{base}/api/v1/utils/highlight"))
        .bearer_auth(&token)
        .json(&json!({"code": "", "language": "rust"}))
        .send()
        .await
        .unwrap();
    let body: Value = resp.json().await.unwrap();
    assert_eq!(body["unsupported"], json!(false));
    assert_eq!(body["spans"], json!([]));

    // languages diagnostics
    let resp = client
        .get(format!("{base}/api/v1/utils/highlight/languages"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: Value = resp.json().await.unwrap();
    let languages = body["languages"].as_array().expect("languages array");
    assert!(
        languages.contains(&json!("rust")) && languages.contains(&json!("python")),
        "{}",
        serde_json::to_string(&languages).unwrap()
    );
    assert_eq!(body["errors"], json!([]));
}
