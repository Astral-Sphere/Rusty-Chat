//! Auth extractors — open-webui `get_current_user` / `get_verified_user`
//! semantics: token from `Authorization: Bearer` → cookie `token` →
//! `x-api-key`; `sk-`-prefixed credentials authenticate as API keys;
//! `pending` users fail verified endpoints.

use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use rc_core::Error;
use rc_db::repo::users::User;
use rc_db::repo::{auths, users};

use crate::state::AppState;

fn unauthorized() -> (axum::http::StatusCode, String) {
    (
        axum::http::StatusCode::UNAUTHORIZED,
        "Invalid token".to_string(),
    )
}

/// `get_current_user`: requires a valid token AND a matching user row.
pub struct AuthUser(pub User);

impl FromRequestParts<AppState> for AuthUser {
    type Rejection = (axum::http::StatusCode, String);

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        resolve_user(parts, state)
            .await
            .map(AuthUser)
            .map_err(|_| unauthorized())
    }
}

/// `get_verified_user`: authenticated AND role ∈ {user, admin}.
pub struct VerifiedUser(pub User);

impl FromRequestParts<AppState> for VerifiedUser {
    type Rejection = (axum::http::StatusCode, String);

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let AuthUser(user) = AuthUser::from_request_parts(parts, state).await?;
        if user.role.as_deref() == Some("pending") {
            return Err((
                axum::http::StatusCode::UNAUTHORIZED,
                "User role is pending; awaiting activation.".to_string(),
            ));
        }
        Ok(VerifiedUser(user))
    }
}

/// Credential extraction order (open-webui parity):
/// `Authorization: Bearer <jwt|sk-…>` → cookie `token` → header `x-api-key`.
pub fn extract_credential(parts: &Parts) -> Option<String> {
    if let Some(auth_header) = parts
        .headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        && let Some(cred) = auth_header
            .strip_prefix("Bearer ")
            .or_else(|| auth_header.strip_prefix("bearer "))
        && !cred.is_empty()
    {
        return Some(cred.to_string());
    }
    if let Some(cookie_header) = parts
        .headers
        .get(axum::http::header::COOKIE)
        .and_then(|v| v.to_str().ok())
        && let Some(token) = extract_cookie(cookie_header, "token")
    {
        return Some(token);
    }
    if let Some(key) = parts.headers.get("x-api-key").and_then(|v| v.to_str().ok())
        && !key.is_empty()
    {
        return Some(key.to_string());
    }
    None
}

fn extract_cookie(header: &str, name: &str) -> Option<String> {
    for pair in header.split(';') {
        let pair = pair.trim();
        if let Some((k, v)) = pair.split_once('=')
            && k.trim() == name
        {
            return Some(v.trim().to_string());
        }
    }
    None
}

async fn resolve_user(parts: &mut Parts, app: &AppState) -> std::result::Result<User, Error> {
    let Some(cred) = extract_credential(parts) else {
        return Err(Error::Unauthorized("no credentials".to_string()));
    };
    let user = if cred.starts_with("sk-") {
        auths::authenticate_user_by_api_key(&app.db, &cred).await?
    } else {
        let claims = rc_auth::decode_token(&cred, &app.secret_key)?;
        users::get_user_by_id(&app.db, &claims.id).await?
    };
    user.ok_or(Error::Unauthorized("user not found".to_string()))
}

/// Used by `/api/config`: the user if credentials are present and valid;
/// anything else stays anonymous (matching open-webui, which only 401s when
/// an Authorization header fails to decode).
pub async fn optional_user(parts: &mut Parts, app: &AppState) -> Option<User> {
    resolve_user(parts, app).await.ok()
}
