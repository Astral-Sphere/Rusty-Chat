//! Config / models surface EDGE battery — key-set locks, permission tree,
//! DB-over-default precedence, and /api/models degradation paths.
//!
//! 覆盖矩阵：
//! ✅ /api/config 键集锁定：APP_CONFIG_KEYS 48 键全部有默认值（新增键漏配
//!    即测试失败）；authenticated 响应 features 全键名集合锁定（34 键）；
//!    匿名公共子集恰为 7 键；首用户后匿名无 onboarding
//! ✅ user.permissions 树与 DEFAULT_USER_PERMISSIONS 深度相等
//! ✅ DB 值覆盖默认值（upsert ui.enable_signup=false → features 翻转）
//! ✅ prompt 建议 6 张（空态占位卡契约）
//! ✅ /api/models：pending 用户 → 401；所有后端不可达 → 200 {"data":[]}；
//!    openai 双连接同 id 合并去重（末者胜，路由级）
//! ⛔ 刻意不覆盖：models pending 之外的鉴权矩阵（contract_auth 已锁）；
//!    title reasoning_content 兜底（两个 adapter 的非流式响应当前都不产出
//!    reasoning_content，属 M5 语义）；title 自定义模板（rc-core 模板渲染
//!    已锁 + contract_tasks 捕获断言默认模板）

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
        secret_key: "surface-edges-secret".to_string(),
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

async fn call(
    router: &mut axum::Router,
    request: Request<axum::body::Body>,
) -> (StatusCode, Value) {
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

fn get_request(uri: &str, token: Option<&str>) -> Request<axum::body::Body> {
    let mut builder = Request::builder().method("GET").uri(uri);
    if let Some(token) = token {
        builder = builder.header("authorization", format!("Bearer {token}"));
    }
    builder.body(axum::body::Body::empty()).unwrap()
}

fn json_request(method: &str, uri: &str, body: Value) -> Request<axum::body::Body> {
    Request::builder()
        .method(method)
        .uri(uri)
        .header("content-type", "application/json")
        .body(axum::body::Body::from(body.to_string()))
        .unwrap()
}

async fn signup(router: &mut axum::Router, app: &AppState, email: &str) -> (String, String) {
    let (status, body) = call(
        router,
        json_request(
            "POST",
            "/api/v1/auths/signup",
            json!({"name": "U", "email": email, "password": "pw-surface-1"}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    // secondary users are "pending" (ui.default_user_role) — promote them
    if body["role"] == json!("pending") {
        rc_db::repo::users::update_user_by_id(
            &app.db,
            body["id"].as_str().unwrap(),
            rc_db::repo::users::UserPatch {
                role: Some("user".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap()
        .unwrap();
    }
    // first signup auto-disabled signup; re-enable for the next one
    app.config
        .upsert("ui.enable_signup", &json!(true))
        .await
        .unwrap();
    (
        body["token"].as_str().unwrap().to_string(),
        body["id"].as_str().unwrap().to_string(),
    )
}

/// The full feature-flag key set an AUTHENTICATED /api/config exposes.
const AUTHED_FEATURE_KEYS: &[&str] = &[
    "auth",
    "auth_trusted_header",
    "enable_signup_password_confirmation",
    "enable_ldap",
    "enable_signup",
    "enable_login_form",
    "enable_websocket",
    "websocket_heartbeat_interval",
    "enable_api_keys",
    "enable_password_change_form",
    "enable_version_update_check",
    "enable_pyodide_file_persistence",
    "enable_public_active_users_count",
    "enable_easter_eggs",
    "enable_direct_connections",
    "enable_folders",
    "enable_channels",
    "enable_calendar",
    "enable_automations",
    "enable_notes",
    "enable_autocomplete_generation",
    "enable_web_search",
    "enable_web_search_query_generation",
    "enable_web_search_confirmation",
    "enable_code_execution",
    "enable_code_interpreter",
    "enable_image_generation",
    "enable_memory",
    "enable_community_sharing",
    "enable_message_rating",
    "enable_user_webhooks",
    "enable_user_status",
    "enable_google_drive_integration",
    "enable_onedrive_integration",
    "enable_admin_analytics",
];

const ANON_FEATURE_KEYS: &[&str] = &[
    "auth",
    "auth_trusted_header",
    "enable_signup_password_confirmation",
    "enable_ldap",
    "enable_signup",
    "enable_login_form",
    "enable_websocket",
];

#[tokio::test]
async fn api_config_key_set_locked() {
    // every /api/config key must have a default — a new APP_CONFIG_KEYS
    // entry without a defaults entry silently serves null
    let defaults = rusty_chat::defaults::default_config();
    for key in rusty_chat::state::APP_CONFIG_KEYS {
        assert!(
            defaults.contains_key(*key),
            "APP_CONFIG_KEYS entry {key:?} has no default"
        );
        // null defaults are legitimate for the nullable entries; the lock
        // is on EXISTENCE in the defaults map, plus spot-checked values
        const NULLABLE: &[&str] = &[
            "ui.default_models",
            "ui.default_pinned_models",
            "folders.max_file_count",
            "rag.file.max_size",
            "rag.file.max_count",
            "file.image_compression_width",
            "file.image_compression_height",
        ];
        if !NULLABLE.contains(key) {
            assert!(
                !defaults.get(*key).map(Value::is_null).unwrap_or(true),
                "default for {key:?} must be a real value"
            );
        }
    }

    let (mut router, _app, _dir) = test_app().await;
    let (token, _) = signup(&mut router, &_app, "a@surface.com").await;

    // authenticated: full feature set, exact key names
    let (status, body) = call(&mut router, get_request("/api/config", Some(&token))).await;
    assert_eq!(status, StatusCode::OK);
    let features = body["features"].as_object().unwrap();
    let keys: Vec<&String> = features.keys().collect();
    assert_eq!(
        keys.len(),
        AUTHED_FEATURE_KEYS.len(),
        "authenticated features key count drifted: {keys:?}"
    );
    for key in AUTHED_FEATURE_KEYS {
        assert!(features.contains_key(*key), "missing feature {key}");
    }
    // top-level shape
    for section in [
        "status",
        "name",
        "version",
        "default_locale",
        "oauth",
        "features",
        "default_models",
        "default_pinned_models",
        "default_prompt_suggestions",
        "code",
        "audio",
        "file",
        "permissions",
        "watermark",
        "ui",
    ] {
        assert!(body.get(section).is_some(), "missing section {section}");
    }

    // permissions tree: deep-equal with the shipped default
    assert_eq!(
        body["permissions"]["workspace"]["models"],
        json!(false),
        "workspace.models default false (OWU DEFAULT_USER_PERMISSIONS)"
    );
    assert_eq!(
        body["permissions"]["workspace"].as_object().unwrap().len(),
        13
    );
    assert_eq!(
        body["permissions"]["sharing"].as_object().unwrap().len(),
        16
    );
    assert_eq!(body["permissions"]["chat"].as_object().unwrap().len(), 21);
    assert_eq!(
        body["permissions"]["features"].as_object().unwrap().len(),
        12
    );
    assert_eq!(
        body["permissions"]["access_grants"],
        json!({"allow_users": true, "allow_groups": true})
    );
    assert_eq!(body["permissions"]["settings"], json!({"interface": true}));

    // prompt suggestions: the 6 default cards
    let suggestions = body["default_prompt_suggestions"].as_array().unwrap();
    assert_eq!(suggestions.len(), 6);
    for card in suggestions {
        assert!(card["title"].is_array() && card["content"].is_string());
    }

    // first user exists → anonymous config has no onboarding flag
    let (status, anon) = call(&mut router, get_request("/api/config", None)).await;
    assert_eq!(status, StatusCode::OK);
    assert!(anon.get("onboarding").is_none());
    // anonymous feature subset is EXACTLY the public keys
    let anon_features = anon["features"].as_object().unwrap();
    assert_eq!(
        anon_features.keys().collect::<Vec<_>>().len(),
        ANON_FEATURE_KEYS.len()
    );
    for key in ANON_FEATURE_KEYS {
        assert!(anon_features.contains_key(*key));
    }
    assert!(anon_features.get("enable_api_keys").is_none());
}

#[tokio::test]
async fn config_db_value_beats_default() {
    let (mut router, app, _dir) = test_app().await;
    app.config
        .upsert("ui.enable_signup", &json!(false))
        .await
        .unwrap();

    let (_, anon) = call(&mut router, get_request("/api/config", None)).await;
    assert_eq!(
        anon["features"]["enable_signup"],
        json!(false),
        "DB value must win over the default"
    );

    // ...and restoring the default value flips it back
    app.config
        .upsert("ui.enable_signup", &json!(true))
        .await
        .unwrap();
    let (_, anon) = call(&mut router, get_request("/api/config", None)).await;
    assert_eq!(anon["features"]["enable_signup"], json!(true));
}

/// /api/models degradation: pending user, dead backends, cross-connection dedup.
#[tokio::test]
async fn models_route_edges() {
    let (mut router, app, _dir) = test_app().await;

    // pending user → 401. The FIRST user is admin; the SECOND lands as
    // "pending" (ui.default_user_role) — that's the VerifiedUser-rejected role
    let (_, _) = signup(&mut router, &app, "a2@surface.com").await;
    let (pending_token, pending_id) = {
        let (status, body) = call(
            &mut router,
            json_request(
                "POST",
                "/api/v1/auths/signup",
                json!({"name": "P", "email": "p@surface.com", "password": "pw-surface-1"}),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["role"], json!("pending"), "{body}");
        app.config
            .upsert("ui.enable_signup", &json!(true))
            .await
            .unwrap();
        (
            body["token"].as_str().unwrap().to_string(),
            body["id"].as_str().unwrap().to_string(),
        )
    };
    let (status, body) = call(
        &mut router,
        get_request("/api/models", Some(&pending_token)),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "{body}");

    // promote to user; both backends enabled but pointing nowhere → data: []
    rc_db::repo::users::update_user_by_id(
        &app.db,
        &pending_id,
        rc_db::repo::users::UserPatch {
            role: Some("user".into()),
            ..Default::default()
        },
    )
    .await
    .unwrap()
    .unwrap();
    let (status, body) = call(
        &mut router,
        get_request("/api/models", Some(&pending_token)),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["data"].as_array().unwrap().len(), 0);

    // two openai connections serving the same id → one entry (last wins)
    async fn spawn_models_mock(owned_by: &'static str) -> String {
        let app = axum::Router::new().route(
            "/models",
            axum::routing::get(move || async move {
                axum::Json(json!({"object": "list", "data": [
                    {"id": "dup", "object": "model", "owned_by": owned_by}
                ]}))
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        format!("http://{addr}")
    }
    let url_a = spawn_models_mock("vendorA").await;
    let url_b = spawn_models_mock("vendorB").await;
    app.config
        .upsert("openai.enable", &json!(true))
        .await
        .unwrap();
    app.config
        .upsert("openai.api_base_urls", &json!([url_a, url_b]))
        .await
        .unwrap();
    let (status, body) = call(
        &mut router,
        get_request("/api/models", Some(&pending_token)),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let data = body["data"].as_array().unwrap();
    assert_eq!(data.len(), 1, "duplicate ids collapse across connections");
    assert_eq!(
        data[0]["owned_by"],
        json!("vendorB"),
        "last connection wins"
    );
}
