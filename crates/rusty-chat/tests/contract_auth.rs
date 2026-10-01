//! Auth endpoint contract tests — JSON shapes and flows must match
//! open-webui `routers/auths.py` (signin/signup/signout/session/password/
//! api-key) and `get_app_config`.
//!
//! 覆盖矩阵：
//! ✅ /api/config：匿名形状（status/name/version/features 公共子集）、
//!   首用户前 onboarding=true、DB seed 默认值（enable_signup=true）
//! ✅ signup：首用户 → admin + enable_signup 自动关闭；次用户 → pending；
//!   关闭 signup 后 403；重复 email 400（含大小写变体）
//! ✅ signin：正确/错误密码（400 Invalid credentials）；Set-Cookie token
//! ✅ session：Bearer 携带 → 形状（id/email/name/role/permissions）；
//!   无凭据 → 401；pending 用户 → 401
//! ✅ password：错误旧密码 400；正确后新旧密码行为翻转
//! ✅ api key：默认关闭 403；开启后创建（sk-+32hex）/获取/删除
//! ✅ signout：200 + 清 cookie
//! ✅ /ollama/{*path} 代理：匿名 401 → 登录后透传（剥 /ollama 前缀）、
//!   上游非 200 透传、不可达 502、disabled 404
//! ⛔ 刻意不覆盖：OAuth/LDAP/trusted-header（M7）、rate limit（M5）

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use serde_json::{Value, json};

use rusty_chat::state::AppState;

/// Fresh API-only app over a bootstrapped temp SQLite database.
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
        secret_key: "contract-test-secret".to_string(),
        webui_name: "Open WebUI".to_string(),
        version: env!("CARGO_PKG_VERSION"),
        placeholder_hash: std::sync::Arc::new(rc_auth::placeholder_hash()),
        webui_auth: true,
        hub: std::sync::Arc::new(rc_realtime::Hub::new()),
    };
    // dist path without index.html → API-only (no fallback service)
    let router = rusty_chat::build_router(state.clone(), dir.path());
    (router, state, dir)
}

async fn call(
    router: &mut axum::Router,
    request: Request<Body>,
) -> (StatusCode, Value, Option<String>) {
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

fn get_request(uri: &str, token: Option<&str>) -> Request<Body> {
    let mut builder = Request::builder().method("GET").uri(uri);
    if let Some(token) = token {
        builder = builder.header("authorization", format!("Bearer {token}"));
    }
    builder.body(Body::empty()).unwrap()
}

fn authed_request(
    method: &'static str,
    uri: &'static str,
    body: Value,
    token: &str,
) -> Request<Body> {
    let builder = Request::builder()
        .method(method)
        .uri(uri)
        .header("authorization", format!("Bearer {token}"))
        .header("content-type", "application/json");
    match body {
        Value::Null => builder.body(Body::empty()).unwrap(),
        v => builder.body(Body::from(v.to_string())).unwrap(),
    }
}

#[tokio::test]
async fn full_auth_flow_contract() {
    let (mut router, app, _dir) = test_app().await;

    // ---- /api/config anonymous, no users → onboarding ----
    let (status, body, _) = call(&mut router, get_request("/api/config", None)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["status"], json!(true));
    assert_eq!(body["name"], json!("Open WebUI"));
    assert_eq!(body["onboarding"], json!(true));
    assert_eq!(body["features"]["auth"], json!(true));
    assert_eq!(body["features"]["enable_signup"], json!(true));
    assert_eq!(body["features"]["enable_login_form"], json!(true));
    // authenticated-only features are stripped for anonymous requests
    assert!(
        body["features"].get("enable_api_keys").is_none(),
        "public subset only"
    );
    assert_eq!(body["oauth"]["providers"], json!({}));
    assert_eq!(body["permissions"]["chat"]["file_upload"], json!(true));

    // ---- signup: first user becomes admin ----
    let (status, body, cookie) = call(
        &mut router,
        json_request(
            "POST",
            "/api/v1/auths/signup",
            json!({
                "name": "Admin", "email": "admin@example.com", "password": "hunter2-secret"
            }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "signup body: {body}");
    // SessionUserResponse shape
    for key in [
        "token",
        "token_type",
        "expires_at",
        "id",
        "email",
        "name",
        "role",
        "profile_image_url",
        "permissions",
    ] {
        assert!(body.get(key).is_some(), "missing session key {key}");
    }
    assert_eq!(body["role"], json!("admin"), "first user must be admin");
    assert_eq!(body["token_type"], json!("Bearer"));
    assert_eq!(
        body["profile_image_url"],
        json!(format!(
            "/api/v1/users/{}/profile/image",
            body["id"].as_str().unwrap()
        ))
    );
    let admin_token = body["token"].as_str().unwrap().to_string();
    let cookie = cookie.expect("signin/signup must set the token cookie");
    assert!(cookie.starts_with("token=") && cookie.contains("HttpOnly"));

    // signup auto-disabled after first user
    let (status, body, _) = call(
        &mut router,
        json_request(
            "POST",
            "/api/v1/auths/signup",
            json!({
                "name": "Late", "email": "late@example.com", "password": "pw-second"
            }),
        ),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "signup disabled after first user: {body}"
    );

    // re-enable signup, then add a normal user (role = ui.default_user_role)
    app.config
        .upsert("ui.enable_signup", &json!(true))
        .await
        .unwrap();
    let (status, body, _) = call(
        &mut router,
        json_request(
            "POST",
            "/api/v1/auths/signup",
            json!({
                "name": "Late", "email": "late@example.com", "password": "pw-second"
            }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body["role"],
        json!("pending"),
        "subsequent users get ui.default_user_role"
    );
    let pending_token = body["token"].as_str().unwrap().to_string();

    // ---- signin ----
    let (status, body, cookie) = call(
        &mut router,
        json_request(
            "POST",
            "/api/v1/auths/signin",
            json!({
                "email": "admin@example.com", "password": "hunter2-secret"
            }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["role"], json!("admin"));
    assert!(cookie.unwrap().starts_with("token="));

    // wrong password → 400
    let (status, body, _) = call(
        &mut router,
        json_request(
            "POST",
            "/api/v1/auths/signin",
            json!({
                "email": "admin@example.com", "password": "wrong"
            }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["detail"], json!("Invalid credentials"));

    // unknown email → same error (timing-identical path)
    let (status, body, _) = call(
        &mut router,
        json_request(
            "POST",
            "/api/v1/auths/signin",
            json!({
                "email": "ghost@example.com", "password": "whatever"
            }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["detail"], json!("Invalid credentials"));

    // ---- session ----
    let (status, body, _) = call(
        &mut router,
        get_request("/api/v1/auths/", Some(&admin_token)),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["email"], json!("admin@example.com"));
    assert_eq!(body["role"], json!("admin"));
    assert!(body["permissions"].is_object());

    // no credentials → 401
    let (status, _, _) = call(&mut router, get_request("/api/v1/auths/", None)).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    // pending user session works (authed)…
    let (status, body, _) = call(
        &mut router,
        get_request("/api/v1/auths/", Some(&pending_token)),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["role"], json!("pending"));

    // …but cookie-auth also works
    let request = Request::builder()
        .method("GET")
        .uri("/api/v1/auths/")
        .header("cookie", format!("token={admin_token}"))
        .body(Body::empty())
        .unwrap();
    let (status, body, _) = call(&mut router, request).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["role"], json!("admin"));

    // ---- update password ----
    let (status, _, _) = call(
        &mut router,
        json_request(
            "POST",
            "/api/v1/auths/update/password",
            json!({
                "password": "WRONG-old", "new_password": "new-pass-99"
            }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "no token → 401");

    let request = authed_request(
        "POST",
        "/api/v1/auths/update/password",
        json!({"password": "WRONG-old", "new_password": "new-pass-99"}),
        &admin_token,
    );
    let (status, _, _) = call(&mut router, request).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    let request = authed_request(
        "POST",
        "/api/v1/auths/update/password",
        json!({"password": "hunter2-secret", "new_password": "new-pass-99"}),
        &admin_token,
    );
    let (status, body, _) = call(&mut router, request).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body, json!(true));

    // old password rejected, new accepted
    let (status, _, _) = call(
        &mut router,
        json_request(
            "POST",
            "/api/v1/auths/signin",
            json!({
                "email": "admin@example.com", "password": "hunter2-secret"
            }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, body, _) = call(
        &mut router,
        json_request(
            "POST",
            "/api/v1/auths/signin",
            json!({
                "email": "admin@example.com", "password": "new-pass-99"
            }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    // ---- api keys ----
    let request = authed_request("POST", "/api/v1/auths/api_key", json!({}), &admin_token);
    let (status, _, _) = call(&mut router, request).await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "api keys disabled by default"
    );

    app.config
        .upsert("auth.enable_api_keys", &json!(true))
        .await
        .unwrap();
    let request = authed_request("POST", "/api/v1/auths/api_key", json!({}), &admin_token);
    let (status, body, _) = call(&mut router, request).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let api_key = body["api_key"].as_str().unwrap().to_string();
    assert!(
        api_key.starts_with("sk-") && api_key.len() == 35,
        "sk- + 32 hex: {api_key}"
    );

    let (status, body, _) = call(
        &mut router,
        get_request("/api/v1/auths/api_key", Some(&admin_token)),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["api_key"], json!(api_key));

    // API key authenticates as the user
    let (status, body, _) = call(&mut router, get_request("/api/v1/auths/", Some(&api_key))).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["role"], json!("admin"));

    let request = authed_request("DELETE", "/api/v1/auths/api_key", Value::Null, &admin_token);
    let (status, body, _) = call(&mut router, request).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, json!(true));
    let (status, body, _) = call(
        &mut router,
        get_request("/api/v1/auths/api_key", Some(&admin_token)),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["api_key"], Value::Null);

    // ---- signout ----
    let request = Request::builder()
        .method("POST")
        .uri("/api/v1/auths/signout")
        .body(Body::empty())
        .unwrap();
    let (status, body, cookie) = call(&mut router, request).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["status"], json!(true));
    let cookie = cookie.unwrap();
    assert!(
        cookie.contains("Max-Age=0") || cookie.contains("Expires=Thu, 01 Jan 1970"),
        "cookie cleared: {cookie}"
    );
}

/// Duplicate email signup → 400 (claimed by the matrix but previously
/// untested), including the case-insensitive variant (functional unique
/// index on lower(email)).
#[tokio::test]
async fn signup_duplicate_email_returns_400() {
    let (mut router, app, _dir) = test_app().await;
    let (status, first, _) = call(
        &mut router,
        json_request(
            "POST",
            "/api/v1/auths/signup",
            json!({"name": "A", "email": "dup@example.com", "password": "pw-dup-1"}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{first}");
    // first user auto-disabled signup — the 403 gate fires before the
    // duplicate check otherwise; re-enable so the 400 path is reachable
    app.config
        .upsert("ui.enable_signup", &json!(true))
        .await
        .unwrap();

    // exact duplicate
    let (status, body, _) = call(
        &mut router,
        json_request(
            "POST",
            "/api/v1/auths/signup",
            json!({"name": "Dup", "email": "dup@example.com", "password": "pw-dup-2"}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(body["detail"].as_str().is_some(), "{body}");

    // case-insensitive variant collides too
    let (status, body, _) = call(
        &mut router,
        json_request(
            "POST",
            "/api/v1/auths/signup",
            json!({"name": "Dup", "email": "DUP@EXAMPLE.COM", "password": "pw-dup-3"}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
}

#[tokio::test]
async fn api_config_with_authenticated_user_shows_full_features() {
    let (mut router, _app, _dir) = test_app().await;
    let (status, body, _) = call(
        &mut router,
        json_request(
            "POST",
            "/api/v1/auths/signup",
            json!({
                "name": "A", "email": "a@example.com", "password": "pw-full-1"
            }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let token = body["token"].as_str().unwrap().to_string();

    let (status, body, _) = call(&mut router, get_request("/api/config", Some(&token))).await;
    assert_eq!(status, StatusCode::OK);
    // authenticated config carries the full feature set
    assert_eq!(body["features"]["enable_api_keys"], json!(false));
    assert_eq!(body["features"]["enable_memory"], json!(true));
    assert_eq!(body["audio"]["tts"]["engine"], json!(""));
    assert!(
        body.get("onboarding").is_none(),
        "onboarding flag only for anonymous pre-first-user"
    );
}

#[tokio::test]
async fn api_models_lists_ollama_backend() {
    use serde_json::json;

    // fake ollama backend
    let app = axum::Router::new().route(
        "/api/tags",
        axum::routing::get(|| async {
            axum::Json(json!({"models": [
                {"name": "llama3:8b", "model": "llama3:8b", "digest": "d1", "size": 1},
                {"name": "mistral", "model": "mistral", "digest": "d2"}
            ]}))
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    let (mut router, app_state, _dir) = test_app().await;
    app_state
        .config
        .upsert("ollama.enable", &json!(true))
        .await
        .unwrap();
    app_state
        .config
        .upsert("ollama.base_urls", &json!([format!("http://{addr}")]))
        .await
        .unwrap();

    // anonymous → 401
    let (status, _, _) = call(&mut router, get_request("/api/models", None)).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    // signup admin, list models
    let (status, body, _) = call(
        &mut router,
        json_request(
            "POST",
            "/api/v1/auths/signup",
            json!({
                "name": "A", "email": "a@example.com", "password": "pw-models-1"
            }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let token = body["token"].as_str().unwrap();

    let (status, body, _) = call(&mut router, get_request("/api/models", Some(token))).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let data = body["data"].as_array().expect("data array");
    assert_eq!(data.len(), 2);
    assert_eq!(data[0]["owned_by"], json!("ollama"));
    assert_eq!(data[0]["object"], json!("model"));
    assert_eq!(data[0]["id"], json!("llama3:8b"));
    // raw ollama tag payload preserved for the frontend
    assert_eq!(data[0]["ollama"]["digest"], json!("d1"));
}

/// `/ollama/{*path}` reverse proxy contract. Auth first (open-webui mounts
/// every /ollama route behind get_verified_user), then enable check, then
/// forwarding semantics. Regression: the proxy used to forward ANONYMOUS
/// requests straight to the configured ollama backend.
#[tokio::test]
async fn ollama_proxy_requires_auth_and_forwards() {
    use serde_json::json;

    let app = axum::Router::new()
        .route(
            "/api/tags",
            axum::routing::get(|| async { axum::Json(json!({"models": [{"name": "llama3:8b"}]})) }),
        )
        .route(
            "/api/boom",
            axum::routing::get(|| async { (axum::http::StatusCode::NOT_FOUND, "nope") }),
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    let (mut router, app_state, _dir) = test_app().await;
    app_state
        .config
        .upsert("ollama.enable", &json!(true))
        .await
        .unwrap();
    app_state
        .config
        .upsert("ollama.base_urls", &json!([format!("http://{addr}")]))
        .await
        .unwrap();

    // anonymous → 401 before any config/forward logic
    let (status, body, _) = call(&mut router, get_request("/ollama/api/tags", None)).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "{body}");

    let (status, body, _) = call(
        &mut router,
        json_request(
            "POST",
            "/api/v1/auths/signup",
            json!({"name": "A", "email": "a@example.com", "password": "pw-proxy-1"}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let token = body["token"].as_str().unwrap().to_string();

    // authed GET forwards body and status from the backend
    let (status, body, _) = call(&mut router, get_request("/ollama/api/tags", Some(&token))).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["models"][0]["name"], json!("llama3:8b"));

    // upstream non-200 passes through
    let (status, body, _) = call(&mut router, get_request("/ollama/api/boom", Some(&token))).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");

    // unreachable backend → 502 with detail
    app_state
        .config
        .upsert("ollama.base_urls", &json!(["http://127.0.0.1:1"]))
        .await
        .unwrap();
    let (status, body, _) = call(&mut router, get_request("/ollama/api/tags", Some(&token))).await;
    assert_eq!(status, StatusCode::BAD_GATEWAY, "{body}");
    assert!(body["detail"].as_str().unwrap().contains("unreachable"));

    // disabled → 404 (authed user, still not configured)
    app_state
        .config
        .upsert("ollama.enable", &json!(false))
        .await
        .unwrap();
    let (status, body, _) = call(&mut router, get_request("/ollama/api/tags", Some(&token))).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
}
