//! `/api/v1/chats` — CRUD over chats (open-webui `routers/chats.py` M1
//! subset: new/list/search/get/update/delete/pin/archive/share/tags).
//! Ownership checks mirror `get_chat_by_id_and_user_id`; admins are NOT
//! special-cased in M1 (parity with the default router paths used here).

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use rc_db::repo::chats::{self, ChatTitleId, NewChatParams};
use rc_db::repo::{shared_chats, tags, users};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::extract::VerifiedUser;
use crate::state::AppState;

fn unauthorized(detail: &str) -> Response {
    (StatusCode::UNAUTHORIZED, Json(json!({"detail": detail}))).into_response()
}

fn not_found() -> Response {
    (StatusCode::NOT_FOUND, Json(json!({"detail": "Not found"}))).into_response()
}

fn bad_request(detail: &str) -> Response {
    (StatusCode::BAD_REQUEST, Json(json!({"detail": detail}))).into_response()
}

/// ChatTitleIdResponse + `active` marker added by OWU's list helper.
fn title_id_row(row: &ChatTitleId) -> Value {
    json!({
        "id": row.id,
        "title": row.title,
        "updated_at": row.updated_at,
        "created_at": row.created_at,
        "last_read_at": row.last_read_at,
        "active": false,
    })
}

/// Full chat row serialization (ChatResponse shape = entity fields).
fn chat_response(chat: &rc_db::repo::chats::Chat) -> Value {
    serde_json::to_value(chat).unwrap_or(Value::Null)
}

#[derive(Deserialize)]
pub struct ChatForm {
    pub chat: Value,
    #[serde(default)]
    pub variables: Option<Value>,
    #[serde(default)]
    pub folder_id: Option<String>,
}

/// `POST /new` — create chat; body `chat` must be a JSON object.
pub async fn create_new_chat(
    State(app): State<AppState>,
    VerifiedUser(user): VerifiedUser,
    Json(form): Json<ChatForm>,
) -> Response {
    let id = uuid::Uuid::new_v4().to_string();
    match chats::insert_new_chat(
        &app.db,
        NewChatParams {
            id: &id,
            user_id: &user.id,
            chat: &form.chat,
            folder_id: form.folder_id.as_deref(),
            variables: form.variables.as_ref(),
            internal_meta: None,
            timer_at: None,
        },
    )
    .await
    {
        Ok(Some(chat)) => Json(chat_response(&chat)).into_response(),
        Ok(None) => bad_request("chat not created"),
        Err(e) => bad_request(&e.to_string()),
    }
}

#[derive(Deserialize)]
pub struct ListQuery {
    pub page: Option<i64>,
    #[serde(default)]
    pub include_pinned: Option<bool>,
    #[serde(default)]
    pub include_folders: Option<bool>,
}

/// `GET /`|`/list` — sidebar list; page is 1-based with OWU's 60/page.
pub async fn list_chats(
    State(app): State<AppState>,
    VerifiedUser(user): VerifiedUser,
    Query(query): Query<ListQuery>,
) -> Response {
    let (skip, limit) = match query.page {
        Some(page) if page > 0 => (Some(((page - 1) * 60) as u64), Some(60)),
        Some(_) => (Some(0u64), Some(60)),
        None => (None, None),
    };
    match chats::get_chat_title_id_list_by_user_id(
        &app.db,
        &user.id,
        false,
        query.include_folders.unwrap_or(false),
        query.include_pinned.unwrap_or(false),
        skip,
        limit,
    )
    .await
    {
        Ok(rows) => Json(Value::Array(rows.iter().map(title_id_row).collect())).into_response(),
        Err(e) => bad_request(&e.to_string()),
    }
}

#[derive(Deserialize)]
pub struct SearchQuery {
    pub text: String,
    pub page: Option<i64>,
}

/// `GET /search?text=` — case-insensitive title search (M1: title only,
/// no snippet; content search lands with full-text index in M3).
pub async fn search_chats(
    State(app): State<AppState>,
    VerifiedUser(user): VerifiedUser,
    Query(query): Query<SearchQuery>,
) -> Response {
    let page = query.page.unwrap_or(1).max(1);
    let skip = ((page - 1) * 60) as u64;
    match chats::get_chat_list_by_search(&app.db, &user.id, &query.text).await {
        Ok(all) => {
            let rows: Vec<Value> = all
                .iter()
                .skip(skip as usize)
                .take(60)
                .map(|c| {
                    json!({
                        "id": c.id, "title": c.title,
                        "updated_at": c.updated_at, "created_at": c.created_at,
                        "last_read_at": c.last_read_at, "active": false,
                    })
                })
                .collect();
            Json(Value::Array(rows)).into_response()
        }
        Err(e) => bad_request(&e.to_string()),
    }
}

/// `GET /pinned`.
pub async fn pinned_chats(
    State(app): State<AppState>,
    VerifiedUser(user): VerifiedUser,
) -> Response {
    match chats::get_pinned_chats_by_user_id(&app.db, &user.id).await {
        Ok(rows) => Json(Value::Array(rows.iter().map(title_id_row).collect())).into_response(),
        Err(e) => bad_request(&e.to_string()),
    }
}

/// `GET /archived` — archived chats (full rows).
pub async fn archived_chats(
    State(app): State<AppState>,
    VerifiedUser(user): VerifiedUser,
) -> Response {
    match chats::get_archived_chat_list_by_user_id(&app.db, &user.id).await {
        Ok(rows) => Json(Value::Array(rows.iter().map(chat_response).collect())).into_response(),
        Err(e) => bad_request(&e.to_string()),
    }
}

/// `GET /{id}` — full chat; non-owner gets 401 (OWU ACCESS_PROHIBITED path).
pub async fn get_chat(
    State(app): State<AppState>,
    VerifiedUser(user): VerifiedUser,
    Path(id): Path<String>,
) -> Response {
    match chats::get_chat_by_id_and_user_id(&app.db, &id, &user.id).await {
        Ok(Some(chat)) => Json(chat_response(&chat)).into_response(),
        Ok(None) => unauthorized("Not found"),
        Err(e) => bad_request(&e.to_string()),
    }
}

/// `POST /{id}` — top-level blob merge + optional variables; touch only when
/// history/messages moved (open-webui update_chat_by_id parity).
pub async fn update_chat(
    State(app): State<AppState>,
    VerifiedUser(user): VerifiedUser,
    Path(id): Path<String>,
    Json(form): Json<ChatForm>,
) -> Response {
    if chats::get_chat_by_id_and_user_id(&app.db, &id, &user.id)
        .await
        .map_err(|e| bad_request(&e.to_string()))
        .ok()
        .flatten()
        .is_none()
    {
        return unauthorized("Access prohibited");
    }
    let touch = form.chat.get("history").is_some() || form.chat.get("messages").is_some();
    let chat = match chats::update_chat_by_id(&app.db, &id, &form.chat, touch).await {
        Ok(Some(c)) => c,
        Ok(None) => return unauthorized("Not found"),
        Err(e) => return bad_request(&e.to_string()),
    };
    let chat = if form.variables.is_some() {
        chats::update_chat_variables_by_id(&app.db, &id, form.variables.clone())
            .await
            .ok()
            .flatten()
            .unwrap_or(chat)
    } else {
        chat
    };
    Json(chat_response(&chat)).into_response()
}

/// `DELETE /{id}`.
pub async fn delete_chat(
    State(app): State<AppState>,
    VerifiedUser(user): VerifiedUser,
    Path(id): Path<String>,
) -> Response {
    match chats::delete_chat_by_id_and_user_id(&app.db, &id, &user.id).await {
        Ok(true) => Json(json!(true)).into_response(),
        Ok(false) => unauthorized("Not found"),
        Err(e) => bad_request(&e.to_string()),
    }
}

/// `POST /{id}/pin` — toggle.
pub async fn pin_chat(
    State(app): State<AppState>,
    VerifiedUser(user): VerifiedUser,
    Path(id): Path<String>,
) -> Response {
    if chats::get_chat_by_id_and_user_id(&app.db, &id, &user.id)
        .await
        .ok()
        .flatten()
        .is_none()
    {
        return unauthorized("Not found");
    }
    match chats::toggle_chat_pinned_by_id(&app.db, &id).await {
        Ok(Some(chat)) => Json(chat_response(&chat)).into_response(),
        _ => bad_request("pin failed"),
    }
}

/// `POST /{id}/archive` — toggle (clears folder, OWU parity).
pub async fn archive_chat(
    State(app): State<AppState>,
    VerifiedUser(user): VerifiedUser,
    Path(id): Path<String>,
) -> Response {
    if chats::get_chat_by_id_and_user_id(&app.db, &id, &user.id)
        .await
        .ok()
        .flatten()
        .is_none()
    {
        return unauthorized("Not found");
    }
    match chats::toggle_chat_archive_by_id(&app.db, &id).await {
        Ok(Some(chat)) => Json(chat_response(&chat)).into_response(),
        _ => bad_request("archive failed"),
    }
}

/// `POST /archive/all` / `POST /unarchive/all`.
pub async fn archive_all(
    State(app): State<AppState>,
    VerifiedUser(user): VerifiedUser,
) -> Response {
    Json(json!(
        chats::archive_all_chats_by_user_id(&app.db, &user.id)
            .await
            .unwrap_or(false)
    ))
    .into_response()
}

pub async fn unarchive_all(
    State(app): State<AppState>,
    VerifiedUser(user): VerifiedUser,
) -> Response {
    Json(json!(
        chats::unarchive_all_chats_by_user_id(&app.db, &user.id)
            .await
            .unwrap_or(false)
    ))
    .into_response()
}

/// `POST /{id}/share` — create/refresh share snapshot; returns updated chat.
pub async fn share_chat(
    State(app): State<AppState>,
    VerifiedUser(user): VerifiedUser,
    Path(id): Path<String>,
) -> Response {
    if chats::get_chat_by_id_and_user_id(&app.db, &id, &user.id)
        .await
        .ok()
        .flatten()
        .is_none()
    {
        return unauthorized("Not found");
    }
    match chats::share_chat(&app.db, &id).await {
        Ok(Some(chat)) => Json(chat_response(&chat)).into_response(),
        Ok(None) => not_found(),
        Err(e) => bad_request(&e.to_string()),
    }
}

/// `DELETE /{id}/share`.
pub async fn delete_share(
    State(app): State<AppState>,
    VerifiedUser(user): VerifiedUser,
    Path(id): Path<String>,
) -> Response {
    if chats::get_chat_by_id_and_user_id(&app.db, &id, &user.id)
        .await
        .ok()
        .flatten()
        .is_none()
    {
        return unauthorized("Not found");
    }
    match chats::delete_shared_chat_by_chat_id(&app.db, &id).await {
        Ok(deleted) => Json(json!(deleted)).into_response(),
        Err(e) => bad_request(&e.to_string()),
    }
}

/// `GET /share/{share_id}` — PUBLIC (share links work logged out).
pub async fn get_shared_chat(
    State(app): State<AppState>,
    Path(share_id): Path<String>,
) -> Response {
    match chats::get_chat_by_share_id(&app.db, &share_id).await {
        Ok(Some(chat)) => Json(chat_response(&chat)).into_response(),
        Ok(None) => not_found(),
        Err(e) => bad_request(&e.to_string()),
    }
}

#[derive(Deserialize)]
pub struct UpdateTagsForm {
    #[serde(default)]
    pub tags: Vec<String>,
}

/// `POST /{id}/tags` — replace a chat's tags (body: {tags: [...]}).
/// Owner-checked here (OWU routers/chats.py resolves the chat via
/// get_chat_by_id_and_user_id before touching tags); the repo helper is
/// id-only because it also serves background tag generation.
pub async fn update_chat_tags(
    State(app): State<AppState>,
    VerifiedUser(user): VerifiedUser,
    Path(id): Path<String>,
    Json(form): Json<UpdateTagsForm>,
) -> Response {
    let owned = chats::get_chat_by_id_and_user_id(&app.db, &id, &user.id)
        .await
        .ok()
        .flatten()
        .is_some();
    if !owned {
        return unauthorized("Not found");
    }
    let tag_refs: Vec<&str> = form.tags.iter().map(String::as_str).collect();
    match chats::update_chat_tags_by_id(&app.db, &id, &tag_refs, &user.id).await {
        Ok(()) => {
            let rows = tags::get_tags_by_user_id(&app.db, &user.id)
                .await
                .unwrap_or_default();
            let tag_list: Vec<Value> = rows
                .iter()
                .map(|t| json!({"id": t.id, "name": t.name, "user_id": t.user_id, "meta": t.meta}))
                .collect();
            Json(Value::Array(tag_list)).into_response()
        }
        Err(e) => bad_request(&e.to_string()),
    }
}

/// `DELETE /` — delete every chat of the current user.
pub async fn delete_all_chats(
    State(app): State<AppState>,
    VerifiedUser(user): VerifiedUser,
) -> Response {
    match chats::delete_chats_by_user_id(&app.db, &user.id).await {
        Ok(deleted) => Json(json!(deleted)).into_response(),
        Err(e) => bad_request(&e.to_string()),
    }
}

/// `GET /{id}/tags` — tags of one chat (from its meta).
pub async fn get_chat_tags(
    State(app): State<AppState>,
    VerifiedUser(user): VerifiedUser,
    Path(id): Path<String>,
) -> Response {
    let Some(chat) = chats::get_chat_by_id_and_user_id(&app.db, &id, &user.id)
        .await
        .ok()
        .flatten()
    else {
        return unauthorized("Not found");
    };
    let tag_ids: Vec<String> = chat
        .meta
        .as_ref()
        .and_then(|m| m.get("tags"))
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    let all = tags::get_tags_by_user_id(&app.db, &user.id)
        .await
        .unwrap_or_default();
    let rows: Vec<Value> = all
        .iter()
        .filter(|t| tag_ids.contains(&t.id))
        .map(|t| json!({"id": t.id, "name": t.name, "user_id": t.user_id, "meta": t.meta}))
        .collect();
    Json(Value::Array(rows)).into_response()
}

/// `GET /shared` — the user's shared-chat snapshots.
pub async fn shared_chats(
    State(app): State<AppState>,
    VerifiedUser(user): VerifiedUser,
) -> Response {
    match shared_chats::get_shared_chats_by_user(&app.db, &user.id).await {
        Ok(rows) => {
            let out: Vec<Value> = rows
                .iter()
                .map(|(s, _)| {
                    json!({
                        "id": s.id, "chat_id": s.chat_id, "title": s.title,
                        "share_id": s.id, "updated_at": s.updated_at, "created_at": s.created_at,
                    })
                })
                .collect();
            Json(Value::Array(out)).into_response()
        }
        Err(e) => bad_request(&e.to_string()),
    }
}

// users::user referenced by VerifiedUser; keep imports meaningful
#[allow(unused_imports)]
use users as _users_alias;
