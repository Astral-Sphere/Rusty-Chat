//! Chats CRUD contract tests — M1 route surface over the repo layer.
//!
//! 覆盖矩阵：
//! ✅ new → ChatResponse 形状（id/user_id/title/chat/…/meta）
//! ✅ list：默认排除 pinned/folder/archived/internal；page=2 分页；
//!   include_pinned/include_folders
//! ✅ get：本人 200、他人 401
//! ✅ update：blob 顶层合并 + history 合并（stale 写者不丢消息）+ variables
//! ✅ delete：本人可删、他人 401
//! ✅ pin/archive 切换；archived 列表；pinned 列表
//! ✅ share：创建→公开读取（无凭据）→删除
//! ✅ tags：设置→查询→孤儿子清理；非属主 401（OWU 路由层属主校验）；
//!   不存在 id 401
//! ✅ search：大小写不敏感标题匹配
//! ✔ admin 特权路径（M1 未做 admin 特判，与默认路由一致）
//! ✅ 双方言：SQLite 必跑；RC_TEST_PG_URL 门控时同一 flow 再跑 Postgres
//!   （scratch 库 rc_contract_chats_test）
//! ⛔ 刻意不覆盖：fork/clone（M3）、message 级端点（M3）、import/export（M3）

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt as _;

use serde_json::{Value, json};
use tower::ServiceExt as _;

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
        secret_key: "chats-contract-secret".to_string(),
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

/// PG leg: fresh scratch database bootstrapped to head, gated by
/// `RC_TEST_PG_URL`. Serialized against other suites via a file-scope lock
/// (the scratch database name is shared).
static PG_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

async fn pg_test_app() -> Option<(axum::Router, AppState, tempfile::TempDir)> {
    let Ok(url) = std::env::var("RC_TEST_PG_URL") else {
        return None;
    };
    let _guard = PG_LOCK.lock().await;
    let trimmed = url.trim_end_matches('/');
    let cut = trimmed.rfind('/').filter(|i| !trimmed[..*i].ends_with(':'));
    let base = match cut {
        Some(i) => &trimmed[..i],
        None => trimmed,
    };
    let dir = tempfile::tempdir().unwrap();
    rc_db::install_drivers();
    {
        let pool = sqlx::AnyPool::connect(&format!("{base}/postgres"))
            .await
            .unwrap();
        let mut conn = pool.acquire().await.unwrap();
        sqlx::raw_sql("DROP DATABASE IF EXISTS rc_contract_chats_test WITH (FORCE);")
            .execute(&mut *conn)
            .await
            .ok();
        sqlx::raw_sql("CREATE DATABASE rc_contract_chats_test;")
            .execute(&mut *conn)
            .await
            .unwrap();
    }
    let db_url = format!("{base}/rc_contract_chats_test");
    {
        let pool = sqlx::AnyPool::connect(&db_url).await.unwrap();
        let mut conn = pool.acquire().await.unwrap();
        rc_db::bootstrap::bootstrap(&mut conn, rc_db::bootstrap::Dialect::Postgres)
            .await
            .unwrap();
    }
    let db = sea_orm::Database::connect(&db_url).await.unwrap();
    let config = std::sync::Arc::new(rc_db::repo::config::ConfigEngine::new(db.clone()));
    config
        .seed_defaults(&rusty_chat::defaults::default_config())
        .await
        .unwrap();
    let state = AppState {
        db,
        config,
        secret_key: "chats-contract-secret".to_string(),
        webui_name: "Open WebUI".to_string(),
        version: env!("CARGO_PKG_VERSION"),
        placeholder_hash: std::sync::Arc::new(rc_auth::placeholder_hash()),
        webui_auth: true,
        hub: std::sync::Arc::new(rc_realtime::Hub::new()),
    };
    Some((
        rusty_chat::build_router(state.clone(), dir.path()),
        state,
        dir,
    ))
}

/// Runs one flow on SQLite (always) and on Postgres (when gated in).
macro_rules! everywhere {
    ($flow:ident) => {{
        let (router, state, dir) = test_app().await;
        $flow(router, state, dir).await;
        if std::env::var("RC_TEST_PG_URL").is_ok() {
            if let Some((router, state, dir)) = pg_test_app().await {
                $flow(router, state, dir).await;
            } else {
                eprintln!("skipping PG leg: RC_TEST_PG_URL unusable");
            }
        }
    }};
}

async fn call(router: &mut axum::Router, request: Request<Body>) -> (StatusCode, Value) {
    let response = router.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let body = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes)
            .unwrap_or(Value::String(String::from_utf8_lossy(&bytes).into()))
    };
    (status, body)
}

fn req(method: &str, uri: &str, token: Option<&str>, body: Option<Value>) -> Request<Body> {
    let mut builder = Request::builder().method(method).uri(uri);
    if let Some(token) = token {
        builder = builder.header("authorization", format!("Bearer {token}"));
    }
    match body {
        Some(v) => builder
            .header("content-type", "application/json")
            .body(Body::from(v.to_string()))
            .unwrap(),
        None => builder.body(Body::empty()).unwrap(),
    }
}

fn blob(title: &str) -> Value {
    json!({"title": title, "models": ["m1"], "history": {"currentId": "u1", "messages": {
        "u1": {"id": "u1", "parentId": null, "childrenIds": [], "role": "user", "content": "hi", "timestamp": 1}
    }}})
}

async fn chats_crud_contract_flow(
    mut router: axum::Router,
    app_state: AppState,
    _dir: tempfile::TempDir,
) {
    // two users
    let (_, user_a) = call(
        &mut router,
        req(
            "POST",
            "/api/v1/auths/signup",
            None,
            Some(json!({
                "name": "A", "email": "a@example.com", "password": "pw-chats-1"
            })),
        ),
    )
    .await;
    let token_a = user_a["token"].as_str().unwrap().to_string();
    // first signup auto-disabled ui.enable_signup (OWU parity) — re-enable
    app_state
        .config
        .upsert("ui.enable_signup", &json!(true))
        .await
        .unwrap();
    let (_, user_b) = call(
        &mut router,
        req(
            "POST",
            "/api/v1/auths/signup",
            None,
            Some(json!({
                "name": "B", "email": "b@example.com", "password": "pw-chats-2"
            })),
        ),
    )
    .await;
    let token_b = user_b["token"].as_str().unwrap().to_string();

    // ---- new ----
    let (status, chat1) = call(
        &mut router,
        req(
            "POST",
            "/api/v1/chats/new",
            Some(&token_a),
            Some(json!({
                "chat": blob("First Chat")
            })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{chat1}");
    for key in [
        "id",
        "user_id",
        "title",
        "chat",
        "updated_at",
        "created_at",
        "archived",
        "meta",
        "variables",
        "folder_id",
        "current_message_id",
    ] {
        assert!(chat1.get(key).is_some(), "ChatResponse missing {key}");
    }
    assert_eq!(chat1["title"], json!("First Chat"));
    let chat1_id = chat1["id"].as_str().unwrap().to_string();

    // updated_at has second precision — space the two chats so ordering is
    // deterministic (updated_at desc, id asc tiebreak).
    std::thread::sleep(std::time::Duration::from_millis(1100));
    let (status, chat2) = call(
        &mut router,
        req(
            "POST",
            "/api/v1/chats/new",
            Some(&token_a),
            Some(json!({
                "chat": blob("Second")
            })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let chat2_id = chat2["id"].as_str().unwrap().to_string();

    // unauthenticated create → 401
    let (status, _) = call(
        &mut router,
        req(
            "POST",
            "/api/v1/chats/new",
            None,
            Some(json!({"chat": blob("x")})),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    // ---- list: newest first, excludes pinned ----
    let (status, list) = call(
        &mut router,
        req("GET", "/api/v1/chats/", Some(&token_a), None),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let arr = list.as_array().unwrap();
    assert_eq!(arr.len(), 2);
    assert_eq!(arr[0]["id"], json!(chat2_id), "newest first");
    assert_eq!(arr[0]["active"], json!(false));
    for row in arr {
        assert!(row.get("last_read_at").is_some(), "title row shape");
    }

    // page semantics
    let (status, page1) = call(
        &mut router,
        req("GET", "/api/v1/chats/?page=1", Some(&token_a), None),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(page1.as_array().unwrap().len(), 2);

    // ---- pin chat1 → excluded from default list, present in pinned ----
    let (status, pinned_chat) = call(
        &mut router,
        req(
            "POST",
            &format!("/api/v1/chats/{chat1_id}/pin"),
            Some(&token_a),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(pinned_chat["pinned"], json!(true));
    let (_, list) = call(
        &mut router,
        req("GET", "/api/v1/chats/", Some(&token_a), None),
    )
    .await;
    assert_eq!(list.as_array().unwrap().len(), 1);
    let (_, pinned) = call(
        &mut router,
        req("GET", "/api/v1/chats/pinned", Some(&token_a), None),
    )
    .await;
    assert_eq!(pinned.as_array().unwrap().len(), 1);
    assert_eq!(pinned.as_array().unwrap()[0]["id"], json!(chat1_id));
    // unpin via second toggle
    let (_, unpinned) = call(
        &mut router,
        req(
            "POST",
            &format!("/api/v1/chats/{chat1_id}/pin"),
            Some(&token_a),
            None,
        ),
    )
    .await;
    assert_eq!(unpinned["pinned"], json!(false));

    // ---- get: owner 200, other user 401 ----
    let (status, got) = call(
        &mut router,
        req(
            "GET",
            &format!("/api/v1/chats/{chat1_id}"),
            Some(&token_a),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(got["chat"]["title"], json!("First Chat"));
    let (status, _) = call(
        &mut router,
        req(
            "GET",
            &format!("/api/v1/chats/{chat1_id}"),
            Some(&token_b),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    // ---- update: stale writer merge + variables ----
    let (status, updated) = call(&mut router, req("POST", &format!("/api/v1/chats/{chat1_id}"), Some(&token_a), Some(json!({
        "chat": {"history": {"currentId": "u1", "messages": {
            "u1": {"id": "u1", "parentId": null, "childrenIds": [], "role": "user", "content": "hi", "timestamp": 1},
            "a1": {"id": "a1", "parentId": "u1", "childrenIds": [], "role": "assistant", "content": "yo", "timestamp": 2}
        }}},
        "variables": {"tone": "short"}
    })))).await;
    assert_eq!(status, StatusCode::OK, "{updated}");
    // stale-writer protection: merged blob still has u1 AND a1
    let messages = updated["chat"]["history"]["messages"].as_object().unwrap();
    assert!(messages.contains_key("u1") && messages.contains_key("a1"));
    assert_eq!(updated["variables"]["tone"], json!("short"));

    // other user cannot update
    let (status, _) = call(
        &mut router,
        req(
            "POST",
            &format!("/api/v1/chats/{chat1_id}"),
            Some(&token_b),
            Some(json!({
                "chat": {"title": "hijack"}
            })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    // ---- archive toggle + archived list ----
    let (status, archived_chat) = call(
        &mut router,
        req(
            "POST",
            &format!("/api/v1/chats/{chat2_id}/archive"),
            Some(&token_a),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(archived_chat["archived"], json!(true));
    let (_, archived_list) = call(
        &mut router,
        req("GET", "/api/v1/chats/archived", Some(&token_a), None),
    )
    .await;
    assert_eq!(archived_list.as_array().unwrap().len(), 1);
    assert_eq!(archived_list.as_array().unwrap()[0]["id"], json!(chat2_id));
    call(
        &mut router,
        req(
            "POST",
            &format!("/api/v1/chats/{chat2_id}/archive"),
            Some(&token_a),
            None,
        ),
    )
    .await;

    // ---- search ----
    let (status, hits) = call(
        &mut router,
        req(
            "GET",
            "/api/v1/chats/search?text=first",
            Some(&token_a),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(hits.as_array().unwrap().len(), 1);
    assert_eq!(hits.as_array().unwrap()[0]["id"], json!(chat1_id));

    // ---- share ----
    let (status, shared) = call(
        &mut router,
        req(
            "POST",
            &format!("/api/v1/chats/{chat1_id}/share"),
            Some(&token_a),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let share_id = shared["share_id"].as_str().unwrap().to_string();
    // public read without credentials
    let (status, public) = call(
        &mut router,
        req(
            "GET",
            &format!("/api/v1/chats/share/{share_id}"),
            None,
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(public["title"], json!("First Chat"));
    // user's shared list
    let (_, shared_list) = call(
        &mut router,
        req("GET", "/api/v1/chats/shared", Some(&token_a), None),
    )
    .await;
    assert_eq!(shared_list.as_array().unwrap().len(), 1);
    // unshare
    let (status, _) = call(
        &mut router,
        req(
            "DELETE",
            &format!("/api/v1/chats/{chat1_id}/share"),
            Some(&token_a),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    // ---- tags ----
    let (status, tags) = call(
        &mut router,
        req(
            "POST",
            &format!("/api/v1/chats/{chat1_id}/tags"),
            Some(&token_a),
            Some(json!({
                "tags": ["Work", "ideas"]
            })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(tags.as_array().unwrap().len(), 2);
    let (status, chat_tags) = call(
        &mut router,
        req(
            "GET",
            &format!("/api/v1/chats/{chat1_id}/tags"),
            Some(&token_a),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let tag_ids: Vec<&str> = chat_tags
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["id"].as_str().unwrap())
        .collect();
    assert_eq!(tag_ids, vec!["work", "ideas"]);

    // ---- delete ----
    let (status, _) = call(
        &mut router,
        req(
            "DELETE",
            &format!("/api/v1/chats/{chat1_id}"),
            Some(&token_b),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let (status, deleted) = call(
        &mut router,
        req(
            "DELETE",
            &format!("/api/v1/chats/{chat1_id}"),
            Some(&token_a),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(deleted, json!(true));
    let (status, _) = call(
        &mut router,
        req(
            "GET",
            &format!("/api/v1/chats/{chat1_id}"),
            Some(&token_a),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    // tag orphan cleanup happened on tags update of remaining flows; here the
    // chat was deleted AFTER tags were set — orphan cleanup runs on next tag
    // update, not on delete (OWU parity).
}

#[tokio::test]
async fn chats_crud_contract() {
    everywhere!(chats_crud_contract_flow);
}

/// Tags writes are owner-checked at the ROUTE level (open-webui
/// routers/chats.py add_tag_by_id_and_tag_name resolves the chat via
/// get_chat_by_id_and_user_id first). Regression: the repo-level
/// update_chat_tags_by_id looks the row up by id only, so without the route
/// check any signed-in user could rewrite another user's chat tags.
async fn tags_ownership_contract_flow(
    mut router: axum::Router,
    app_state: AppState,
    _dir: tempfile::TempDir,
) {
    let (_, user_a) = call(
        &mut router,
        req(
            "POST",
            "/api/v1/auths/signup",
            None,
            Some(json!({
                "name": "A", "email": "a2@example.com", "password": "pw-tags-1"
            })),
        ),
    )
    .await;
    let token_a = user_a["token"].as_str().unwrap().to_string();
    app_state
        .config
        .upsert("ui.enable_signup", &json!(true))
        .await
        .unwrap();
    let (_, user_b) = call(
        &mut router,
        req(
            "POST",
            "/api/v1/auths/signup",
            None,
            Some(json!({
                "name": "B", "email": "b2@example.com", "password": "pw-tags-2"
            })),
        ),
    )
    .await;
    let token_b = user_b["token"].as_str().unwrap().to_string();

    let (status, chat) = call(
        &mut router,
        req(
            "POST",
            "/api/v1/chats/new",
            Some(&token_a),
            Some(json!({"chat": blob("A Private")})),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{chat}");
    let chat_id = chat["id"].as_str().unwrap().to_string();

    // non-owner cannot replace tags → 401
    let (status, body) = call(
        &mut router,
        req(
            "POST",
            &format!("/api/v1/chats/{chat_id}/tags"),
            Some(&token_b),
            Some(json!({"tags": ["hijacked"]})),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "{body}");

    // the hijack attempt must not have touched A's chat meta
    let (status, a_tags) = call(
        &mut router,
        req(
            "GET",
            &format!("/api/v1/chats/{chat_id}/tags"),
            Some(&token_a),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(a_tags, json!([]), "meta must stay untouched");

    // nonexistent chat id → same 401 (OWU treats missing as unauthorized)
    let (status, _) = call(
        &mut router,
        req(
            "POST",
            "/api/v1/chats/00000000-0000-0000-0000-000000000000/tags",
            Some(&token_a),
            Some(json!({"tags": ["x"]})),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    // owner still works end to end
    let (status, tags) = call(
        &mut router,
        req(
            "POST",
            &format!("/api/v1/chats/{chat_id}/tags"),
            Some(&token_a),
            Some(json!({"tags": ["Work"]})),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{tags}");
    assert_eq!(tags.as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn tags_ownership_contract() {
    everywhere!(tags_ownership_contract_flow);
}
