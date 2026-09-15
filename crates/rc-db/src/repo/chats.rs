//! `Chats` — mirrors open-webui `models/chats.py::ChatTable` (M1 subset:
//! CRUD, list/search, pin/archive, share, tags, blob↔chat_message sync).
//! Blob mutation helpers live in `crate::history`.

use crate::entity::{chat, shared_chat};
use crate::history;
use crate::repo::{chat_messages, not_internal, shared_chats, tags};
use rc_core::Result;
use rc_core::timestamp::Secs;
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, Condition, DatabaseConnection, EntityTrait,
    QueryFilter, QueryOrder, QuerySelect,
};

pub use chat::Model as Chat;

pub type ChatValue = serde_json::Value;

/// (id, title, updated_at, created_at, last_read_at) projection row.
type ChatListRow = (
    String,
    Option<String>,
    Option<i64>,
    Option<i64>,
    Option<i64>,
);

/// `ChatTitleIdResponse` — the lightweight list row.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ChatTitleId {
    pub id: String,
    pub title: String,
    pub updated_at: i64,
    pub created_at: i64,
    pub last_read_at: Option<i64>,
}

pub struct NewChatParams<'a> {
    pub id: &'a str,
    pub user_id: &'a str,
    /// The full chat blob (must contain `title` or defaults to "New Chat").
    pub chat: &'a ChatValue,
    pub folder_id: Option<&'a str>,
    pub variables: Option<&'a ChatValue>,
    pub internal_meta: Option<&'a ChatValue>,
    /// epoch nanos (timer chats)
    pub timer_at: Option<i64>,
}

/// `ChatTable.insert_new_chat` — including the dual-write of initial
/// messages into `chat_message`.
pub async fn insert_new_chat(
    db: &DatabaseConnection,
    params: NewChatParams<'_>,
) -> Result<Option<Chat>> {
    let chat_value = history::clean_null_bytes(params.chat);
    let title = chat_value
        .get("title")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("New Chat")
        .to_string();
    let now = Secs::now().as_i64();

    let am = chat::ActiveModel {
        id: Set(params.id.to_string()),
        user_id: Set(Some(params.user_id.to_string())),
        title: Set(Some(title)),
        chat: Set(Some(chat_value.clone())),
        created_at: Set(Some(now)),
        updated_at: Set(Some(now)),
        share_id: Set(None),
        archived: Set(Some(false)),
        pinned: Set(Some(false)),
        meta: Set(Some(
            params
                .internal_meta
                .cloned()
                .unwrap_or_else(|| serde_json::json!({})),
        )),
        variables: Set(Some(
            params
                .variables
                .cloned()
                .unwrap_or_else(|| serde_json::json!({})),
        )),
        folder_id: Set(params.folder_id.map(str::to_string)),
        tasks: Set(None),
        summary: Set(None),
        current_message_id: Set(history::get_current_message_id(&chat_value)),
        last_read_at: Set(Some(now)),
        timer_at: Set(params.timer_at),
    };
    am.insert(db)
        .await
        .map_err(|e| rc_core::Error::Internal(format!("insert_new_chat: {e}")))?;

    // Dual-write initial messages to chat_message (best-effort, like OWU).
    let messages = chat_value
        .get("history")
        .filter(|h| h.is_object())
        .and_then(|h| h.get("messages"))
        .filter(|m| m.is_object())
        .cloned()
        .or_else(|| {
            // legacy flat list fallback
            let list = chat_value.get("messages")?.as_array()?;
            let map: serde_json::Map<String, ChatValue> = list
                .iter()
                .filter_map(|m| {
                    let id = m.get("id")?.as_str()?.to_string();
                    Some((id, m.clone()))
                })
                .collect();
            Some(ChatValue::Object(map))
        })
        .unwrap_or_else(|| serde_json::json!({}));

    if let Some(map) = messages.as_object() {
        for (message_id, message) in map {
            if message.get("role").is_none() {
                continue;
            }
            if let Err(e) =
                chat_messages::upsert_message(db, message_id, params.id, params.user_id, message)
                    .await
            {
                tracing::warn!(chat_id = params.id, error = %e, "failed to backfill chat_message");
            }
        }
    }

    get_chat_by_id(db, params.id).await
}

/// `ChatTable.get_chat_by_id` — with read-time sanitize + currentId repair
/// (writes back only when something changed, like OWU).
pub async fn get_chat_by_id(db: &DatabaseConnection, id: &str) -> Result<Option<Chat>> {
    let Some(row) = chat::Entity::find_by_id(id).one(db).await? else {
        return Ok(None);
    };
    Ok(Some(sanitize_and_repair(db, row).await?))
}

/// `ChatTable.get_chat_by_id_and_user_id`.
pub async fn get_chat_by_id_and_user_id(
    db: &DatabaseConnection,
    id: &str,
    user_id: &str,
) -> Result<Option<Chat>> {
    let Some(row) = chat::Entity::find()
        .filter(chat::Column::Id.eq(id))
        .filter(chat::Column::UserId.eq(user_id))
        .one(db)
        .await?
    else {
        return Ok(None);
    };
    Ok(Some(sanitize_and_repair(db, row).await?))
}

/// Shared read-path hygiene: null-byte sanitize + history currentId repair,
/// persisting only when changed (OWU `_sanitize_chat_row` +
/// `_repair_chat_current_id`).
async fn sanitize_and_repair(db: &DatabaseConnection, mut row: chat::Model) -> Result<Chat> {
    let mut changed = false;

    if let Some(title) = &row.title {
        let cleaned = match history::clean_null_bytes(&serde_json::Value::String(title.clone())) {
            serde_json::Value::String(s) => s,
            _ => title.clone(),
        };
        if cleaned != *title {
            row.title = Some(cleaned);
            changed = true;
        }
    }

    let chat_value = row.chat.clone().unwrap_or(serde_json::Value::Null);
    let cleaned = history::clean_null_bytes(&chat_value);
    if cleaned != chat_value {
        row.chat = Some(cleaned.clone());
        changed = true;
    }

    // Repair: recompute childrenIds linkage + currentId (see history.rs notes).
    let mut repaired = cleaned.clone();
    if repair_chat_current_id(&mut repaired) {
        row.chat = Some(repaired.clone());
        row.current_message_id =
            Some(history::get_current_message_id(&repaired).unwrap_or_default());
        changed = true;
    }

    if changed {
        let mut am: chat::ActiveModel = row.clone().into();
        am.title = Set(row.title.clone());
        am.chat = Set(row.chat.clone());
        am.current_message_id = Set(row.current_message_id.clone());
        am.update(db).await?;
    }
    Ok(row)
}

/// Port of `_repair_chat_current_id`: fix missing childrenIds linkage; repair
/// a broken/absent `history.currentId` (latest-timestamp leaf heuristic).
/// Returns true if the blob changed.
pub fn repair_chat_current_id(chat_value: &mut ChatValue) -> bool {
    let Some(obj) = chat_value.as_object_mut() else {
        return false;
    };
    let history_obj = match obj.get("history").and_then(|h| h.as_object()) {
        Some(h) => h.clone(),
        None => return false,
    };
    let Some(messages) = history_obj.get("messages").and_then(|m| m.as_object()) else {
        return false;
    };
    let mut messages = messages.clone();

    let mut changed = false;
    let ids: Vec<String> = messages.keys().cloned().collect();
    for message_id in ids {
        let parent_id = messages
            .get(&message_id)
            .and_then(|m| m.get("parentId"))
            .and_then(|p| if p.is_null() { None } else { p.as_str() })
            .map(str::to_string);
        if add_child_id_to_parent(&mut messages, parent_id.as_deref(), &message_id) {
            changed = true;
        }
    }

    let write_back = |obj: &mut serde_json::Map<String, ChatValue>,
                      history: serde_json::Map<String, ChatValue>| {
        obj.insert("history".to_string(), serde_json::Value::Object(history));
    };

    let current_id = history_obj
        .get("currentId")
        .and_then(|v| v.as_str())
        .map(str::to_string);
    let current_message = current_id
        .as_deref()
        .and_then(|id| messages.get(id))
        .cloned();
    let output_role = current_message
        .as_ref()
        .and_then(|m| m.get("output"))
        .and_then(|o| o.as_array())
        .and_then(|items| {
            items
                .iter()
                .find_map(|i| i.get("role").and_then(|r| r.as_str()))
        });

    let current_is_bad_leaf = current_message
        .as_ref()
        .map(|m| {
            output_role == Some("assistant")
                && m.get("parentId")
                    .map(serde_json::Value::is_null)
                    .unwrap_or(true)
                && m.get("timestamp")
                    .and_then(|t| t.as_i64().or(t.as_u64().map(|v| v as i64)))
                    .unwrap_or(0)
                    == 0
                && messages.len() > 1
        })
        .unwrap_or(false);

    let has_id_and_role = current_message
        .as_ref()
        .map(|m| {
            m.get("id").map(|v| !v.is_null()).unwrap_or(false)
                && m.get("role").map(|v| !v.is_null()).unwrap_or(false)
        })
        .unwrap_or(false);

    if has_id_and_role && !current_is_bad_leaf {
        let has_context_summary = current_message
            .as_ref()
            .map(|m| {
                m.get("contextSummary")
                    .map(|v| !v.is_null())
                    .unwrap_or(false)
                    || m.get("context_summary")
                        .map(|v| !v.is_null())
                        .unwrap_or(false)
            })
            .unwrap_or(false);
        let mut history_mut = history_obj.clone();
        history_mut.insert(
            "messages".to_string(),
            serde_json::Value::Object(messages.clone()),
        );
        if let Some(current_id) = &current_id
            && has_context_summary
        {
            let last_descendant = history::last_descendant_id(&messages, current_id);
            if &last_descendant != current_id {
                history_mut.insert(
                    "currentId".to_string(),
                    serde_json::Value::String(last_descendant),
                );
                write_back(obj, history_mut);
                return true;
            }
        }
        if changed {
            write_back(obj, history_mut);
        }
        return changed;
    }

    // Pick the latest-timestamp leaf.
    let mut latest_leaf_id: Option<String> = None;
    let mut latest_timestamp: i64 = -1;
    for (message_id, message) in &messages {
        let Some(role) = message.get("role").and_then(|r| r.as_str()) else {
            continue;
        };
        if role.is_empty() {
            continue;
        }
        let children_len = message
            .get("childrenIds")
            .and_then(|c| c.as_array())
            .map(|a| a.len())
            .unwrap_or(0);
        let timestamp = message
            .get("timestamp")
            .and_then(|t| t.as_i64().or(t.as_u64().map(|v| v as i64)))
            .unwrap_or(0);
        if children_len == 0 && timestamp > latest_timestamp {
            latest_leaf_id = Some(message_id.clone());
            latest_timestamp = timestamp;
        }
    }

    if latest_leaf_id.is_none() || latest_leaf_id == current_id {
        if changed {
            let mut history_mut = history_obj;
            history_mut.insert("messages".to_string(), serde_json::Value::Object(messages));
            write_back(obj, history_mut);
        }
        return changed;
    }

    let mut history_mut = history_obj;
    history_mut.insert("messages".to_string(), serde_json::Value::Object(messages));
    history_mut.insert(
        "currentId".to_string(),
        serde_json::Value::String(latest_leaf_id.unwrap()),
    );
    write_back(obj, history_mut);
    true
}

fn add_child_id_to_parent(
    messages: &mut serde_json::Map<String, ChatValue>,
    parent_id: Option<&str>,
    child_id: &str,
) -> bool {
    let Some(parent_id) = parent_id else {
        return false;
    };
    let Some(parent) = messages.get_mut(parent_id) else {
        return false;
    };
    let Some(obj) = parent.as_object_mut() else {
        return false;
    };
    let children = obj
        .entry("childrenIds".to_string())
        .or_insert_with(|| serde_json::Value::Array(vec![]));
    if !children.is_array() {
        *children = serde_json::Value::Array(vec![]);
    }
    let arr = children.as_array_mut().unwrap();
    if arr.iter().any(|c| c.as_str() == Some(child_id)) {
        return false;
    }
    arr.push(serde_json::Value::String(child_id.to_string()));
    true
}

/// `ChatTable.update_chat_by_id` — top-level keys replaced, `history`
/// merged so stale writers don't drop messages; title derived; touch
/// bumps updated_at.
pub async fn update_chat_by_id(
    db: &DatabaseConnection,
    id: &str,
    chat_update: &ChatValue,
    touch: bool,
) -> Result<Option<Chat>> {
    let Some(row) = chat::Entity::find_by_id(id).one(db).await? else {
        return Ok(None);
    };
    let stored = row.chat.clone().unwrap_or_else(|| serde_json::json!({}));
    let mut updated = merge_top_level(&stored, chat_update);
    if chat_update.get("history").is_some() {
        let merged_history =
            history::merge_history(stored.get("history"), chat_update.get("history"));
        updated
            .as_object_mut()
            .unwrap()
            .insert("history".to_string(), merged_history);
    }
    let updated = history::clean_null_bytes(&updated);
    let title = updated
        .get("title")
        .and_then(|t| t.as_str())
        .unwrap_or("New Chat")
        .to_string();

    let mut am: chat::ActiveModel = row.into();
    am.chat = Set(Some(updated.clone()));
    am.title = Set(Some(title));
    if chat_update.get("history").is_some()
        || chat_update.get("messages").is_some()
        || chat_update.get("currentId").is_some()
        || chat_update.get("branchPointMessageId").is_some()
    {
        am.current_message_id = Set(history::get_current_message_id(&updated));
    }
    if touch {
        am.updated_at = Set(Some(Secs::now().as_i64()));
    }
    let result = am.update(db).await;
    match result {
        Ok(m) => Ok(Some(m)),
        Err(e) => Err(rc_core::Error::Internal(format!("update_chat_by_id: {e}"))),
    }
}

/// Python `{**stored, **chat}`: shallow merge with right-hand wins.
fn merge_top_level(base: &ChatValue, update: &ChatValue) -> ChatValue {
    let mut out = base.as_object().cloned().unwrap_or_default();
    if let Some(obj) = update.as_object() {
        for (k, v) in obj {
            out.insert(k.clone(), v.clone());
        }
    }
    serde_json::Value::Object(out)
}

/// Folder-move path: updates the `folder_id` COLUMN (open-webui
/// `move_chat_to_folder`), which `update_chat_by_id` (blob merge) never does.
pub async fn update_chat_folder_by_id(
    db: &DatabaseConnection,
    id: &str,
    folder_id: Option<&str>,
) -> Result<Option<Chat>> {
    let Some(row) = chat::Entity::find_by_id(id).one(db).await? else {
        return Ok(None);
    };
    let mut am: chat::ActiveModel = row.into();
    am.folder_id = Set(folder_id.map(str::to_string));
    am.updated_at = Set(Some(Secs::now().as_i64()));
    Ok(Some(am.update(db).await?))
}

pub async fn update_chat_title_by_id(
    db: &DatabaseConnection,
    id: &str,
    title: &str,
) -> Result<Option<Chat>> {
    let Some(row) = chat::Entity::find_by_id(id).one(db).await? else {
        return Ok(None);
    };
    let mut am: chat::ActiveModel = row.into();
    am.title = Set(Some(title.to_string()));
    Ok(Some(am.update(db).await?))
}

/// `update_chat_tags_by_id`: meta-only read/write + tag rows sync. `user_id`
/// is the owning user.
pub async fn update_chat_tags_by_id(
    db: &DatabaseConnection,
    id: &str,
    new_tags: &[&str],
    user_id: &str,
) -> Result<()> {
    let Some(row) = chat::Entity::find_by_id(id).one(db).await? else {
        return Ok(());
    };
    let meta = row.meta.clone().unwrap_or_else(|| serde_json::json!({}));
    let old_tags: Vec<String> = meta
        .get("tags")
        .and_then(|t| t.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();

    let new_tag_ids: Vec<String> = new_tags
        .iter()
        .map(|t| tags::tag_id_from_name(t))
        .filter(|tid| tid != "none")
        .collect();

    let mut new_meta = meta.as_object().cloned().unwrap_or_default();
    new_meta.insert(
        "tags".to_string(),
        serde_json::Value::Array(
            new_tag_ids
                .iter()
                .map(|t| serde_json::Value::String(t.clone()))
                .collect(),
        ),
    );
    let mut am: chat::ActiveModel = row.into();
    am.meta = Set(Some(serde_json::Value::Object(new_meta)));
    am.update(db).await?;

    // OWU parity: ensure_tags_exist receives the FILTERED labels (slug != none).
    let names: Vec<&str> = new_tags
        .iter()
        .copied()
        .filter(|t| tags::tag_id_from_name(t) != "none")
        .collect();
    tags::ensure_tags_exist(db, &names, user_id).await?;

    let removed: Vec<String> = old_tags
        .iter()
        .filter(|t| !new_tag_ids.contains(t))
        .cloned()
        .collect();
    if !removed.is_empty() {
        let in_use = tags::collect_chat_tag_ids_in_use(db, user_id).await?;
        tags::delete_orphan_tags_for_user(db, &removed, user_id, &in_use).await?;
    }
    Ok(())
}

/// `toggle_chat_pinned_by_id`.
pub async fn toggle_chat_pinned_by_id(db: &DatabaseConnection, id: &str) -> Result<Option<Chat>> {
    toggle_field(db, id, Toggle::Pinned).await
}

/// `toggle_chat_archive_by_id` — also clears folder_id.
pub async fn toggle_chat_archive_by_id(db: &DatabaseConnection, id: &str) -> Result<Option<Chat>> {
    toggle_field(db, id, Toggle::Archived).await
}

enum Toggle {
    Pinned,
    Archived,
}

async fn toggle_field(db: &DatabaseConnection, id: &str, what: Toggle) -> Result<Option<Chat>> {
    let Some(row) = chat::Entity::find_by_id(id).one(db).await? else {
        return Ok(None);
    };
    let now = Secs::now().as_i64();
    let mut am: chat::ActiveModel = row.clone().into();
    match what {
        Toggle::Pinned => {
            am.pinned = Set(Some(!row.pinned.unwrap_or(false)));
        }
        Toggle::Archived => {
            am.archived = Set(Some(!row.archived.unwrap_or(false)));
            am.folder_id = Set(None);
        }
    }
    am.updated_at = Set(Some(now));
    am.last_read_at = Set(Some(now));
    Ok(Some(am.update(db).await?))
}

pub async fn archive_all_chats_by_user_id(db: &DatabaseConnection, user_id: &str) -> Result<bool> {
    let res = chat::Entity::update_many()
        .col_expr(
            chat::Column::Archived,
            sea_orm::sea_query::Expr::value(true),
        )
        .filter(chat::Column::UserId.eq(user_id))
        .exec(db)
        .await?;
    Ok(res.rows_affected > 0)
}

pub async fn unarchive_all_chats_by_user_id(
    db: &DatabaseConnection,
    user_id: &str,
) -> Result<bool> {
    let res = chat::Entity::update_many()
        .col_expr(
            chat::Column::Archived,
            sea_orm::sea_query::Expr::value(false),
        )
        .filter(chat::Column::UserId.eq(user_id))
        .exec(db)
        .await?;
    Ok(res.rows_affected > 0)
}

/// `get_chat_title_id_list_by_user_id` — the sidebar list.
pub async fn get_chat_title_id_list_by_user_id(
    db: &DatabaseConnection,
    user_id: &str,
    include_archived: bool,
    include_folders: bool,
    include_pinned: bool,
    skip: Option<u64>,
    limit: Option<u64>,
) -> Result<Vec<ChatTitleId>> {
    let backend = db.get_database_backend();
    let mut q = chat::Entity::find()
        .filter(chat::Column::UserId.eq(user_id))
        .filter(not_internal(backend));
    if !include_folders {
        q = q.filter(chat::Column::FolderId.is_null());
    }
    if !include_pinned {
        q = q.filter(
            Condition::any()
                .add(chat::Column::Pinned.eq(false))
                .add(chat::Column::Pinned.is_null()),
        );
    }
    if !include_archived {
        q = q.filter(chat::Column::Archived.eq(false));
    }
    // chat_list_order default: updated_at desc, id
    q = q
        .order_by_desc(chat::Column::UpdatedAt)
        .order_by_asc(chat::Column::Id);
    if let Some(s) = skip {
        q = q.offset(s);
    }
    if let Some(l) = limit {
        q = q.limit(l);
    }
    let rows: Vec<ChatListRow> = q
        .select_only()
        .columns([
            chat::Column::Id,
            chat::Column::Title,
            chat::Column::UpdatedAt,
            chat::Column::CreatedAt,
            chat::Column::LastReadAt,
        ])
        .into_tuple()
        .all(db)
        .await?;
    Ok(rows
        .into_iter()
        .map(
            |(id, title, updated_at, created_at, last_read_at)| ChatTitleId {
                id,
                title: title.unwrap_or_else(|| "New Chat".to_string()),
                updated_at: updated_at.unwrap_or(0),
                created_at: created_at.unwrap_or(0),
                last_read_at,
            },
        )
        .collect())
}

/// `get_archived_chat_list_by_user_id` (full models, newest first).
pub async fn get_archived_chat_list_by_user_id(
    db: &DatabaseConnection,
    user_id: &str,
) -> Result<Vec<Chat>> {
    let backend = db.get_database_backend();
    Ok(chat::Entity::find()
        .filter(chat::Column::UserId.eq(user_id))
        .filter(chat::Column::Archived.eq(true))
        .filter(not_internal(backend))
        .order_by_desc(chat::Column::UpdatedAt)
        .all(db)
        .await?)
}

/// Title-substring search over the user's chats (OWU `get_chat_ids_by_user_id`
/// + search semantics; M1: returns full list rows).
pub async fn get_chat_list_by_search(
    db: &DatabaseConnection,
    user_id: &str,
    query: &str,
) -> Result<Vec<Chat>> {
    let backend = db.get_database_backend();
    Ok(chat::Entity::find()
        .filter(chat::Column::UserId.eq(user_id))
        .filter(chat::Column::Archived.eq(false))
        .filter(not_internal(backend))
        .filter(crate::repo::ci_like(chat::Column::Title, query, backend))
        .order_by_desc(chat::Column::UpdatedAt)
        .all(db)
        .await?)
}

pub async fn get_chats_by_folder_id(
    db: &DatabaseConnection,
    folder_id: &str,
    skip: u64,
    limit: u64,
) -> Result<Vec<Chat>> {
    let backend = db.get_database_backend();
    Ok(chat::Entity::find()
        .filter(chat::Column::FolderId.eq(folder_id))
        .filter(chat::Column::Archived.eq(false))
        .filter(not_internal(backend))
        .order_by_desc(chat::Column::UpdatedAt)
        .offset_if(skip > 0, skip)
        .limit_if(limit > 0, limit)
        .all(db)
        .await?)
}

trait OffsetIf: Sized {
    fn offset_if(self, cond: bool, v: u64) -> Self;
    fn limit_if(self, cond: bool, v: u64) -> Self;
}
impl<T> OffsetIf for T
where
    T: QuerySelect,
{
    fn offset_if(mut self, cond: bool, v: u64) -> Self {
        if cond {
            self = self.offset(v);
        }
        self
    }
    fn limit_if(mut self, cond: bool, v: u64) -> Self {
        if cond {
            self = self.limit(v);
        }
        self
    }
}

/// `Chats.share_chat`: create-or-refresh the shared snapshot; returns the
/// updated chat (share_id set).
pub async fn share_chat(db: &DatabaseConnection, chat_id: &str) -> Result<Option<Chat>> {
    let Some(row) = chat::Entity::find_by_id(chat_id).one(db).await? else {
        return Ok(None);
    };
    let Some(share_id) = &row.share_id else {
        let Some(user_id) = row.user_id.clone() else {
            return Ok(None);
        };
        let Some(shared) = shared_chats::create(db, chat_id, &user_id).await? else {
            return Ok(None);
        };
        let mut am: chat::ActiveModel = row.into();
        am.share_id = Set(Some(shared.id));
        return Ok(Some(am.update(db).await?));
    };
    shared_chats::update(db, share_id).await?;
    Ok(Some(row))
}

pub async fn delete_shared_chat_by_chat_id(db: &DatabaseConnection, chat_id: &str) -> Result<bool> {
    // OWU parity: only the snapshot row is deleted; chat.share_id stays.
    shared_chats::delete_by_chat_id(db, chat_id).await
}

/// `get_chat_by_share_id`: ChatModel-shaped view of the snapshot.
pub async fn get_chat_by_share_id(db: &DatabaseConnection, share_id: &str) -> Result<Option<Chat>> {
    let Some(shared) = shared_chats::get_by_id(db, share_id).await? else {
        return Ok(None);
    };
    Ok(Some(Chat {
        id: shared.id.clone(),
        user_id: shared.user_id.clone(),
        title: shared.title.clone(),
        chat: shared.chat.clone(),
        created_at: shared.created_at,
        updated_at: shared.updated_at,
        share_id: Some(shared.id),
        archived: Some(false),
        pinned: Some(false),
        meta: Some(serde_json::json!({})),
        variables: None,
        folder_id: None,
        tasks: None,
        summary: None,
        current_message_id: None,
        last_read_at: None,
        timer_at: None,
    }))
}

/// `delete_chat_by_id_and_user_id` — explicit chat_message cleanup, shared
/// snapshot removal, share_id clearing. (automation_run unlink arrives in M4.)
pub async fn delete_chat_by_id_and_user_id(
    db: &DatabaseConnection,
    id: &str,
    user_id: &str,
) -> Result<bool> {
    chat_messages::delete_messages_by_chat_id(db, id).await?;
    // shared_chat rows cascade via FK, but OWU also clears chat.share_id by
    // deleting the snapshot explicitly first.
    shared_chats::delete_by_chat_id(db, id).await?;
    let res = chat::Entity::delete_many()
        .filter(chat::Column::Id.eq(id))
        .filter(chat::Column::UserId.eq(user_id))
        .exec(db)
        .await?;
    Ok(res.rows_affected > 0)
}

pub async fn delete_chats_by_user_id(db: &DatabaseConnection, user_id: &str) -> Result<bool> {
    let chat_ids: Vec<String> = chat::Entity::find()
        .filter(chat::Column::UserId.eq(user_id))
        .select_only()
        .column(chat::Column::Id)
        .into_tuple()
        .all(db)
        .await?;
    for id in &chat_ids {
        chat_messages::delete_messages_by_chat_id(db, id).await?;
    }
    shared_chat::Entity::delete_many()
        .filter(shared_chat::Column::UserId.eq(user_id))
        .exec(db)
        .await?;
    let res = chat::Entity::delete_many()
        .filter(chat::Column::UserId.eq(user_id))
        .exec(db)
        .await?;
    Ok(res.rows_affected > 0)
}

/// `upsert_message_to_chat_by_id_and_message_id`: blob upsert +
/// chat_message dual-write + touch. Returns updated chat.
pub async fn upsert_message_to_chat_by_id_and_message_id(
    db: &DatabaseConnection,
    id: &str,
    message_id: &str,
    message: &ChatValue,
) -> Result<Option<Chat>> {
    let Some(row) = chat::Entity::find_by_id(id).one(db).await? else {
        return Ok(None);
    };
    // Python operates on blob['history'] (upsert_message_to_history(history, …)).
    let mut blob = row.chat.clone().unwrap_or_else(|| serde_json::json!({}));
    let mut history_val = blob
        .get("history")
        .cloned()
        .unwrap_or_else(|| serde_json::json!({}));
    history::upsert_message_to_history(&mut history_val, message_id, message);
    blob.as_object_mut()
        .expect("chat blob is an object")
        .insert("history".to_string(), history_val);

    let mut am: chat::ActiveModel = row.into();
    am.chat = Set(Some(history::clean_null_bytes(&blob)));
    am.current_message_id = Set(history::get_current_message_id(&blob));
    am.updated_at = Set(Some(Secs::now().as_i64()));
    let updated = am.update(db).await?;

    if let Some(user_id) = &updated.user_id {
        chat_messages::upsert_message(db, message_id, &updated.id, user_id, message).await?;
    }
    Ok(Some(updated))
}

/// `delete_message_from_chat_by_id_and_message_id`: blob deletion (returns
/// the deleted subtree ids) + chat_message row removal.
pub async fn delete_message_from_chat_by_id_and_message_id(
    db: &DatabaseConnection,
    id: &str,
    message_id: &str,
) -> Result<Option<(Chat, Vec<String>)>> {
    let Some(row) = chat::Entity::find_by_id(id).one(db).await? else {
        return Ok(None);
    };
    // Python operates on blob['history'] here as well.
    let mut blob = row.chat.clone().unwrap_or_else(|| serde_json::json!({}));
    let mut history_val = blob
        .get("history")
        .cloned()
        .unwrap_or_else(|| serde_json::json!({}));
    let deleted = history::delete_message_from_history(&mut history_val, message_id);
    blob.as_object_mut()
        .expect("chat blob is an object")
        .insert("history".to_string(), history_val);

    let mut am: chat::ActiveModel = row.into();
    am.chat = Set(Some(history::clean_null_bytes(&blob)));
    am.current_message_id = Set(history::get_current_message_id(&blob));
    am.updated_at = Set(Some(Secs::now().as_i64()));
    let updated = am.update(db).await?;

    if !deleted.is_empty() {
        chat_messages::delete_messages_by_blob_ids(db, id, &deleted).await?;
    }
    Ok(Some((updated, deleted)))
}

/// Update `last_read_at` to now (returns (ts, was_unread)).
pub async fn update_chat_last_read_at_by_id(
    db: &DatabaseConnection,
    id: &str,
    user_id: &str,
) -> Result<Option<(i64, bool)>> {
    let Some(row) = chat::Entity::find_by_id(id).one(db).await? else {
        return Ok(None);
    };
    if row.user_id.as_deref() != Some(user_id) {
        return Ok(None);
    }
    let ts = Secs::now().as_i64();
    let was_unread = match row.last_read_at {
        None => true,
        Some(last_read) => row.updated_at.unwrap_or(0) > last_read,
    };
    let mut am: chat::ActiveModel = row.into();
    am.last_read_at = Set(Some(ts));
    am.update(db).await?;
    Ok(Some((ts, was_unread)))
}
