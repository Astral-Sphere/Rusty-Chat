//! `/api/v1/auths` — signin/signup/signout/session/password/api-key.
//! Response shapes mirror open-webui `routers/auths.py` exactly.

use axum::Json;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use rc_auth::{
    HashAlgorithm, ParsedDuration, create_token, hash_password, parse_duration, validate_password,
};
use rc_core::timestamp::Secs;
use rc_db::repo::auths::{self, SignupParams};
use rc_db::repo::users as users_repo;
use rc_db::repo::users::{self, UserPatch};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::extract::AuthUser;
use crate::state::AppState;

/// `Set-Cookie` for `token` — httponly, samesite=lax (open-webui defaults).
fn token_cookie(token: &str, max_age: Option<i64>) -> String {
    match max_age {
        Some(age) => format!("token={token}; Path=/; HttpOnly; SameSite=Lax; Max-Age={age}"),
        None => format!("token={token}; Path=/; HttpOnly; SameSite=Lax"),
    }
}

fn clear_cookie() -> String {
    "token=; Path=/; HttpOnly; SameSite=Lax; Max-Age=0".to_string()
}

fn bad_request(detail: &str) -> axum::response::Response {
    (StatusCode::BAD_REQUEST, Json(json!({ "detail": detail }))).into_response()
}

fn forbidden(detail: &str) -> axum::response::Response {
    (StatusCode::FORBIDDEN, Json(json!({ "detail": detail }))).into_response()
}

/// `create_session_response`: build token + cookie + response payload.
// Error variant carries a prebuilt HTTP Response by design (handlers return
// it verbatim); boxing would only add indirection.
#[allow(clippy::result_large_err)]
async fn create_session_response(
    app: &AppState,
    user: &users::User,
) -> std::result::Result<(HeaderMap, Value), axum::response::Response> {
    let expiry_cfg = app
        .config
        .get("auth.jwt_expiry")
        .await
        .ok()
        .flatten()
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_else(|| "4w".to_string());
    let delta = match parse_duration(&expiry_cfg) {
        Ok(ParsedDuration::Finite(d)) => Some(d),
        Ok(ParsedDuration::Never) => None,
        Err(_) => None,
    };
    let expires_at = delta.map(|d| Secs::now().as_i64() + d.as_secs() as i64);

    let (token, _claims) = create_token(&user.id, &app.secret_key, delta)
        .map_err(|_| internal("token creation failed"))?;

    let mut headers = HeaderMap::new();
    let max_age = delta.map(|d| d.as_secs() as i64);
    headers.insert(
        axum::http::header::SET_COOKIE,
        token_cookie(&token, max_age).parse().unwrap(),
    );

    let permissions = app
        .config
        .get("user.permissions")
        .await
        .ok()
        .flatten()
        .unwrap_or_else(|| json!({}));

    Ok((
        headers,
        json!({
            "token": token,
            "token_type": "Bearer",
            "expires_at": expires_at,
            "id": user.id,
            "email": user.email,
            "name": user.name,
            "role": user.role,
            "profile_image_url": format!("/api/v1/users/{}/profile/image", user.id),
            "permissions": permissions,
        }),
    ))
}

fn internal(detail: &str) -> axum::response::Response {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(json!({ "detail": detail })),
    )
        .into_response()
}

#[derive(Deserialize)]
pub struct SigninForm {
    pub email: String,
    pub password: String,
}

pub async fn signin(
    State(app): State<AppState>,
    Json(form): Json<SigninForm>,
) -> axum::response::Response {
    let verifier = PasswordCheck {
        candidate: form.password.clone(),
    };
    let user = match auths::authenticate_user(
        &app.db,
        &form.email,
        &form.password,
        &verifier,
        &app.placeholder_hash,
    )
    .await
    {
        Ok(u) => u,
        Err(_) => return bad_request("Invalid credentials"),
    };
    let Some(user) = user else {
        return bad_request("Invalid credentials");
    };
    match create_session_response(&app, &user).await {
        Ok((headers, body)) => (StatusCode::OK, headers, Json(body)).into_response(),
        Err(resp) => resp,
    }
}

#[derive(Deserialize)]
pub struct SignupForm {
    pub name: String,
    pub email: String,
    pub password: String,
    #[serde(default)]
    pub profile_image_url: Option<String>,
}

pub async fn signup(
    State(app): State<AppState>,
    Json(form): Json<SignupForm>,
) -> axum::response::Response {
    let has_users = users_repo::get_num_users(&app.db).await.unwrap_or(0) > 0;
    if app.webui_auth && has_users {
        let enable_signup = app
            .config
            .get("ui.enable_signup")
            .await
            .ok()
            .flatten()
            .and_then(|v| v.as_bool())
            .unwrap_or(true);
        let enable_login_form = app
            .config
            .get("ui.enable_login_form")
            .await
            .ok()
            .flatten()
            .and_then(|v| v.as_bool())
            .unwrap_or(true);
        if !enable_signup || !enable_login_form {
            return forbidden("Access prohibited");
        }
    }

    match signup_handler(
        &app,
        &form.email,
        &form.password,
        &form.name,
        form.profile_image_url.as_deref(),
    )
    .await
    {
        Ok(user) => match create_session_response(&app, &user).await {
            Ok((headers, body)) => (StatusCode::OK, headers, Json(body)).into_response(),
            Err(err) => err.into_response(),
        },
        Err(resp) => resp,
    }
}

/// `signup_handler`: insert with default role first (TOCTOU-safe first-user
/// promotion), then promote to admin and auto-disable signup when this was
/// the first account.
#[allow(clippy::result_large_err)] // mirrors create_session_response
async fn signup_handler(
    app: &AppState,
    email: &str,
    password: &str,
    name: &str,
    profile_image_url: Option<&str>,
) -> std::result::Result<users::User, axum::response::Response> {
    validate_password(password, HashAlgorithm::Bcrypt).map_err(|e| bad_request(&e.to_string()))?;

    let default_role = app
        .config
        .get("ui.default_user_role")
        .await
        .ok()
        .flatten()
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_else(|| "pending".to_string());

    let hashed =
        hash_password(password, HashAlgorithm::Bcrypt).map_err(|e| internal(&e.to_string()))?;

    let user = auths::insert_new_auth(
        &app.db,
        SignupParams {
            email: &email.to_lowercase(),
            password_hash: &hashed,
            name,
            profile_image_url,
            role: Some(&default_role),
            oauth: None,
        },
    )
    .await;

    let user = match user {
        Ok(Some(u)) => u,
        Ok(None) => return Err(internal("ERROR: User not created")),
        Err(e) => return Err(bad_request(&e.to_string())),
    };

    // First user becomes admin; signup then auto-disables (atomicity note in
    // COMPATIBILITY.md §9.10 — single-process sequential check, same as OWU).
    if users_repo::get_num_users(&app.db).await.unwrap_or(0) == 1 {
        let _ = users_repo::update_user_by_id(
            &app.db,
            &user.id,
            UserPatch {
                role: Some("admin".to_string()),
                ..Default::default()
            },
        )
        .await;
        if let Some(updated) = users_repo::get_user_by_id(&app.db, &user.id).await.unwrap() {
            let _ = app.config.upsert("ui.enable_signup", &json!(false)).await;
            return Ok(updated);
        }
    }

    Ok(user)
}

pub async fn signout() -> impl IntoResponse {
    let mut headers = HeaderMap::new();
    headers.insert(
        axum::http::header::SET_COOKIE,
        clear_cookie().parse().unwrap(),
    );
    (headers, Json(json!({ "status": true, "message": null })))
}

/// `GET /api/v1/auths/` — session user echo (open-webui `get_session_user`).
pub async fn session_user(State(app): State<AppState>, AuthUser(user): AuthUser) -> Json<Value> {
    let permissions = app
        .config
        .get("user.permissions")
        .await
        .ok()
        .flatten()
        .unwrap_or_else(|| json!({}));
    Json(json!({
        "id": user.id,
        "email": user.email,
        "name": user.name,
        "role": user.role,
        "profile_image_url": user.profile_image_url,
        "bio": user.bio,
        "gender": user.gender,
        "date_of_birth": user.date_of_birth,
        "status_emoji": user.status_emoji,
        "status_message": user.status_message,
        "status_expires_at": user.status_expires_at,
        "permissions": permissions,
        "token": null,
        "token_type": "Bearer",
        "expires_at": null,
    }))
}

#[derive(Deserialize)]
pub struct UpdatePasswordForm {
    pub password: String,
    pub new_password: String,
}

pub async fn update_password(
    State(app): State<AppState>,
    AuthUser(session_user): AuthUser,
    Json(form): Json<UpdatePasswordForm>,
) -> axum::response::Response {
    let verifier = PasswordCheck {
        candidate: form.password.clone(),
    };
    let user = match auths::authenticate_user(
        &app.db,
        &session_user.email.clone().unwrap_or_default(),
        &form.password,
        &verifier,
        &app.placeholder_hash,
    )
    .await
    {
        Ok(Some(u)) => u,
        _ => return bad_request("Incorrect password"),
    };

    if let Err(e) = validate_password(&form.new_password, HashAlgorithm::Bcrypt) {
        return bad_request(&e.to_string());
    }
    let hashed = match hash_password(&form.new_password, HashAlgorithm::Bcrypt) {
        Ok(h) => h,
        Err(e) => return internal(&e.to_string()),
    };
    match auths::update_user_password_by_id(&app.db, &user.id, &hashed).await {
        Ok(true) => Json(json!(true)).into_response(),
        _ => internal("password update failed").into_response(),
    }
}

struct PasswordCheck {
    candidate: String,
}

impl auths::VerifyPassword for PasswordCheck {
    async fn verify(&self, hash: &str) -> bool {
        rc_auth::verify_password(&self.candidate, hash)
    }
}

/// API key endpoints (open-webui gates on `auth.enable_api_keys` + user
/// permission `features.api_keys`).
pub async fn get_api_key(State(app): State<AppState>, AuthUser(user): AuthUser) -> Json<Value> {
    let enabled = app
        .config
        .get("auth.enable_api_keys")
        .await
        .ok()
        .flatten()
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    if !enabled {
        return Json(json!({ "api_key": null }));
    }
    let key = auths::api_keys::get_by_user_id(&app.db, &user.id)
        .await
        .ok()
        .flatten();
    Json(json!({ "api_key": key.and_then(|k| k.key) }))
}

#[derive(Deserialize)]
pub struct CreateApiKeyForm {
    #[serde(default)]
    pub expires_at: Option<i64>,
}

pub async fn create_api_key(
    State(app): State<AppState>,
    AuthUser(user): AuthUser,
    Json(_form): Json<CreateApiKeyForm>,
) -> impl IntoResponse {
    let enabled = app
        .config
        .get("auth.enable_api_keys")
        .await
        .ok()
        .flatten()
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    if !enabled {
        return (
            StatusCode::FORBIDDEN,
            Json(json!({ "detail": "API keys are disabled" })),
        )
            .into_response();
    }
    let key = rc_auth::generate_api_key();
    match auths::api_keys::create(&app.db, &user.id, &key).await {
        Ok(_) => Json(json!({ "api_key": key })).into_response(),
        Err(_) => internal("api key creation failed").into_response(),
    }
}

pub async fn delete_api_key(
    State(app): State<AppState>,
    AuthUser(user): AuthUser,
) -> impl IntoResponse {
    match auths::api_keys::delete_by_user_id(&app.db, &user.id).await {
        Ok(deleted) => Json(json!(deleted)).into_response(),
        Err(_) => internal("api key deletion failed").into_response(),
    }
}
