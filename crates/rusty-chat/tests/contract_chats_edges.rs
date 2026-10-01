//! Chats route EDGE battery — pagination boundaries, nonexistent-id
//! semantics, archive-all/delete-all flows, share negative paths, search
//! edges, and the touch rule for ordering.
//!
//! 覆盖矩阵：
//! ✅ list 分页：page=0 → 归位第 1 页；page 超界 → []；page 非数字 → 400
//! ✅ 不存在 id：GET/POST/PIN/ARCHIVE/DELETE/tags 六路由统一 401
//!   （OWU 用 unauthorized 表达"不可见"）
//! ✅ archive/all + unarchive/all：只影响本人；归档列表对应变化
//! ✅ DELETE /（全删）：幂等（第二次仍 200 true）；只删本人
//! ✅ share 负路径：随机 share_id → 404；unshare 后旧链接立即 404；
//!   重复 delete-share → 200 false；非属主 delete-share → 401；
//!   /shared 行形状（share_id == id）
//! ✅ search：命中本人；空 text → 返回全部（语义钉死）；查不到他人；
//!   page=2 空页；缺 text 参数 → 400
//! ✅ title-only 更新不 bump updated_at（无 history/messages 键 → touch=false，
//!   列表顺序不变）；带 history 的更新前移
//! ⛔ 刻意不覆盖：folder 端点（M3）、pin/archive 单聊切换（contract_chats 已锁）

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
        secret_key: "edges-contract-secret".to_string(),
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

async fn signup(router: &mut axum::Router, app: &AppState, name: &str, email: &str) -> String {
    let (status, body) = call(
        router,
        req(
            "POST",
            "/api/v1/auths/signup",
            None,
            Some(json!({"name": name, "email": email, "password": "pw-edges-1"})),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let token = body["token"].as_str().unwrap().to_string();
    // secondary users get ui.default_user_role = "pending" (OWU default);
    // promote so VerifiedUser endpoints accept them
    if let Some(id) = body["id"].as_str() {
        use rc_db::repo::users::UserPatch;
        rc_db::repo::users::update_user_by_id(
            &app.db,
            id,
            UserPatch {
                role: Some("user".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap()
        .unwrap();
    }
    // the first signup auto-disabled ui.enable_signup (OWU parity) —
    // re-enable immediately so the NEXT signup can proceed
    app.config
        .upsert("ui.enable_signup", &json!(true))
        .await
        .unwrap();
    token
}

#[tokio::test]
async fn chats_route_edges() {
    let (mut router, app, _dir) = test_app().await;
    let token_a = signup(&mut router, &app, "A", "a@edges.com").await;
    let token_b = signup(&mut router, &app, "B", "b@edges.com").await;

    // three chats for A (spaced for deterministic order), one for B
    let mut ids = Vec::new();
    for title in ["First", "Second", "Third"] {
        let (status, chat) = call(
            &mut router,
            req(
                "POST",
                "/api/v1/chats/new",
                Some(&token_a),
                Some(json!({"chat": blob(title)})),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        ids.push(chat["id"].as_str().unwrap().to_string());
        std::thread::sleep(std::time::Duration::from_millis(1100));
    }
    let (_, b_chat) = call(
        &mut router,
        req(
            "POST",
            "/api/v1/chats/new",
            Some(&token_b),
            Some(json!({"chat": blob("B Private")})),
        ),
    )
    .await;
    let b_id = b_chat["id"].as_str().unwrap().to_string();

    // ---- pagination edges ----
    let (status, page0) = call(
        &mut router,
        req("GET", "/api/v1/chats/?page=0", Some(&token_a), None),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(page0.as_array().unwrap().len(), 3, "page=0 → first page");

    let (status, over) = call(
        &mut router,
        req("GET", "/api/v1/chats/?page=2", Some(&token_a), None),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        over.as_array().unwrap().len(),
        0,
        "page beyond content → []"
    );

    let (status, _) = call(
        &mut router,
        req("GET", "/api/v1/chats/?page=abc", Some(&token_a), None),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "non-numeric page → 400");

    // ---- nonexistent id → 401 on every route ----
    let ghost = "00000000-0000-0000-0000-00000000edge";
    for (method, uri, body) in [
        ("GET", format!("/api/v1/chats/{ghost}"), None),
        (
            "POST",
            format!("/api/v1/chats/{ghost}"),
            Some(json!({"chat": {"title": "x"}})),
        ),
        ("POST", format!("/api/v1/chats/{ghost}/pin"), None),
        ("POST", format!("/api/v1/chats/{ghost}/archive"), None),
        ("DELETE", format!("/api/v1/chats/{ghost}"), None),
        (
            "POST",
            format!("/api/v1/chats/{ghost}/tags"),
            Some(json!({"tags": []})),
        ),
    ] {
        let (status, _) = call(&mut router, req(method, &uri, Some(&token_a), body)).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "{method} {uri}");
    }

    // ---- archive/all + unarchive/all: only the caller's chats ----
    let (status, _) = call(
        &mut router,
        req("POST", "/api/v1/chats/archive/all", Some(&token_a), None),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (_, archived) = call(
        &mut router,
        req("GET", "/api/v1/chats/archived", Some(&token_a), None),
    )
    .await;
    assert_eq!(archived.as_array().unwrap().len(), 3);
    let (_, list) = call(
        &mut router,
        req("GET", "/api/v1/chats/", Some(&token_a), None),
    )
    .await;
    assert_eq!(list.as_array().unwrap().len(), 0, "all archived");
    // B is untouched
    let (_, b_list) = call(
        &mut router,
        req("GET", "/api/v1/chats/", Some(&token_b), None),
    )
    .await;
    assert_eq!(b_list.as_array().unwrap().len(), 1);

    let (status, _) = call(
        &mut router,
        req("POST", "/api/v1/chats/unarchive/all", Some(&token_a), None),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (_, restored) = call(
        &mut router,
        req("GET", "/api/v1/chats/", Some(&token_a), None),
    )
    .await;
    assert_eq!(restored.as_array().unwrap().len(), 3);

    // ---- share negative paths + /shared row shape ----
    let (status, shared) = call(
        &mut router,
        req(
            "POST",
            &format!("/api/v1/chats/{}/share", ids[0]),
            Some(&token_a),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let share_id = shared["share_id"].as_str().unwrap().to_string();
    // row shape: share_id equals the snapshot's id
    let (_, shared_list) = call(
        &mut router,
        req("GET", "/api/v1/chats/shared", Some(&token_a), None),
    )
    .await;
    let row = &shared_list.as_array().unwrap()[0];
    assert_eq!(row["share_id"], row["id"], "snapshot id == share_id");
    assert_eq!(row["share_id"], json!(share_id));

    // random share id → 404
    let (status, _) = call(
        &mut router,
        req("GET", "/api/v1/chats/share/nope", None, None),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // unshare → old link 404 immediately
    let (status, _) = call(
        &mut router,
        req(
            "DELETE",
            &format!("/api/v1/chats/{}/share", ids[0]),
            Some(&token_a),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (status, _) = call(
        &mut router,
        req(
            "GET",
            &format!("/api/v1/chats/share/{share_id}"),
            None,
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "unshared link dies");

    // second delete-share → 200 false (idempotent)
    let (status, body) = call(
        &mut router,
        req(
            "DELETE",
            &format!("/api/v1/chats/{}/share", ids[0]),
            Some(&token_a),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, json!(false));

    // non-owner cannot delete someone's share
    call(
        &mut router,
        req(
            "POST",
            &format!("/api/v1/chats/{}/share", ids[0]),
            Some(&token_a),
            None,
        ),
    )
    .await;
    let (status, _) = call(
        &mut router,
        req(
            "DELETE",
            &format!("/api/v1/chats/{}/share", ids[0]),
            Some(&token_b),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    // ---- search edges ----
    let (status, hits) = call(
        &mut router,
        req(
            "GET",
            "/api/v1/chats/search?text=second",
            Some(&token_a),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(hits.as_array().unwrap().len(), 1);

    // empty text matches everything (LIKE %% semantics — pinned choice)
    let (_, hits) = call(
        &mut router,
        req("GET", "/api/v1/chats/search?text=", Some(&token_a), None),
    )
    .await;
    assert_eq!(hits.as_array().unwrap().len(), 3);

    // A cannot see B's identically-titled private chat
    call(
        &mut router,
        req(
            "POST",
            &format!("/api/v1/chats/{b_id}"),
            Some(&token_b),
            Some(json!({"chat": {"title": "Second"}})),
        ),
    )
    .await;
    let (_, hits) = call(
        &mut router,
        req(
            "GET",
            "/api/v1/chats/search?text=second",
            Some(&token_a),
            None,
        ),
    )
    .await;
    assert_eq!(hits.as_array().unwrap().len(), 1, "ownership scoping");
    // B does find it
    let (_, b_hits) = call(
        &mut router,
        req(
            "GET",
            "/api/v1/chats/search?text=second",
            Some(&token_b),
            None,
        ),
    )
    .await;
    assert_eq!(b_hits.as_array().unwrap().len(), 1);

    // page beyond the hits
    let (_, hits) = call(
        &mut router,
        req(
            "GET",
            "/api/v1/chats/search?text=second&page=2",
            Some(&token_a),
            None,
        ),
    )
    .await;
    assert_eq!(hits.as_array().unwrap().len(), 0);

    // missing text param → query rejection → 400
    let (status, _) = call(
        &mut router,
        req("GET", "/api/v1/chats/search", Some(&token_a), None),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    // ---- title-only update must not reorder (touch=false) ----
    // ids: newest last; Third currently leads the list
    let (_, before) = call(
        &mut router,
        req("GET", "/api/v1/chats/", Some(&token_a), None),
    )
    .await;
    assert_eq!(before.as_array().unwrap()[0]["id"], json!(ids[2]));
    std::thread::sleep(std::time::Duration::from_millis(1100));
    let (status, _) = call(
        &mut router,
        req(
            "POST",
            &format!("/api/v1/chats/{}", ids[0]),
            Some(&token_a),
            Some(json!({"chat": {"title": "First Renamed"}})),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (_, after) = call(
        &mut router,
        req("GET", "/api/v1/chats/", Some(&token_a), None),
    )
    .await;
    assert_eq!(
        after.as_array().unwrap()[0]["id"],
        json!(ids[2]),
        "title-only update must not bump updated_at"
    );

    // a history-bearing update DOES move the chat to the front
    let (status, _) = call(
        &mut router,
        req(
            "POST",
            &format!("/api/v1/chats/{}", ids[0]),
            Some(&token_a),
            Some(json!({"chat": {"history": {"messages": {}}}})),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (_, after) = call(
        &mut router,
        req("GET", "/api/v1/chats/", Some(&token_a), None),
    )
    .await;
    assert_eq!(after.as_array().unwrap()[0]["id"], json!(ids[0]));

    // ---- DELETE / (delete all): only own; idempotent ----
    let (status, body) = call(
        &mut router,
        req("DELETE", "/api/v1/chats", Some(&token_a), None),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, json!(true));
    let (_, list) = call(
        &mut router,
        req("GET", "/api/v1/chats/", Some(&token_a), None),
    )
    .await;
    assert_eq!(list.as_array().unwrap().len(), 0);
    // B survives
    let (_, b_list) = call(
        &mut router,
        req("GET", "/api/v1/chats/", Some(&token_b), None),
    )
    .await;
    assert_eq!(b_list.as_array().unwrap().len(), 1);
    // second delete → still 200 true (idempotent)
    let (status, body) = call(
        &mut router,
        req("DELETE", "/api/v1/chats", Some(&token_a), None),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, json!(true));
}
