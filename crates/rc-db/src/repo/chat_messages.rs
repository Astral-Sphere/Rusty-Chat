//! `ChatMessages` — mirrors open-webui `models/chat_messages.py` (M1 subset).

use crate::entity::chat_message;
use rc_core::Result;
use rc_core::timestamp::Secs;
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter,
    QueryOrder,
};

pub use chat_message::Model as ChatMessage;

/// Composite primary key convention: `{chat_id}-{message_id}`.
pub fn composite_id(chat_id: &str, message_id: &str) -> String {
    format!("{chat_id}-{message_id}")
}

fn apply_message_data(am: &mut chat_message::ActiveModel, data: &serde_json::Value, now: i64) {
    let get = |key: &str| data.get(key).cloned();
    let get_alt = |a: &str, b: &str| {
        data.get(a)
            .cloned()
            .filter(|v| !v.is_null())
            .or_else(|| data.get(b).cloned())
    };

    am.role = Set(get("role").and_then(|v| v.as_str().map(str::to_string)));
    am.parent_id =
        Set(get_alt("parent_id", "parentId").and_then(|v| v.as_str().map(str::to_string)));
    am.content = Set(get("content"));
    am.output = Set(get("output"));
    am.model_id = Set(get_alt("model_id", "model").and_then(|v| v.as_str().map(str::to_string)));
    am.files = Set(get("files"));
    am.sources = Set(get("sources"));
    am.embeds = Set(get("embeds"));
    am.meta = Set(get("meta"));
    am.done = Set(Some(get("done").and_then(|v| v.as_bool()).unwrap_or(true)));
    am.status_history = Set(get_alt("status_history", "statusHistory"));
    am.error = Set(get("error"));
    am.usage = Set(get("usage"));
    am.context_summary =
        Set(get_alt("context_summary", "contextSummary")
            .and_then(|v| v.as_str().map(str::to_string)));
    am.updated_at = Set(Some(now));
}

/// `ChatMessages.upsert_message` — insert or patch a message row keyed by
/// `{chat_id}-{message_id}`.
pub async fn upsert_message(
    db: &DatabaseConnection,
    message_id: &str,
    chat_id: &str,
    user_id: &str,
    data: &serde_json::Value,
) -> Result<Option<ChatMessage>> {
    let now = Secs::now().as_i64();
    let composite = composite_id(chat_id, message_id);

    if let Some(existing) = chat_message::Entity::find_by_id(&composite).one(db).await? {
        let mut am: chat_message::ActiveModel = existing.into();
        apply_message_data(&mut am, data, now);
        let updated = am.update(db).await?;
        return Ok(Some(updated));
    }

    let am = chat_message::ActiveModel {
        id: Set(composite),
        chat_id: Set(Some(chat_id.to_string())),
        user_id: Set(Some(user_id.to_string())),
        role: Set(Some(
            data.get("role")
                .and_then(|v| v.as_str())
                .unwrap_or("user")
                .to_string(),
        )),
        parent_id: Set(data
            .get("parent_id")
            .or_else(|| data.get("parentId"))
            .and_then(|v| v.as_str())
            .map(str::to_string)),
        content: Set(data.get("content").cloned()),
        output: Set(data.get("output").cloned()),
        model_id: Set(data
            .get("model_id")
            .or_else(|| data.get("model"))
            .and_then(|v| v.as_str())
            .map(str::to_string)),
        files: Set(data.get("files").cloned()),
        sources: Set(data.get("sources").cloned()),
        embeds: Set(data.get("embeds").cloned()),
        meta: Set(data.get("meta").cloned()),
        done: Set(Some(
            data.get("done").and_then(|v| v.as_bool()).unwrap_or(true),
        )),
        status_history: Set(data
            .get("status_history")
            .filter(|v| !v.is_null())
            .or_else(|| data.get("statusHistory"))
            .cloned()),
        error: Set(data.get("error").cloned()),
        usage: Set(data.get("usage").cloned()),
        context_summary: Set(data
            .get("context_summary")
            .or_else(|| data.get("contextSummary"))
            .and_then(|v| v.as_str())
            .map(str::to_string)),
        created_at: Set(Some(
            data.get("timestamp")
                .and_then(serde_json::Value::as_i64)
                .unwrap_or(now),
        )),
        updated_at: Set(Some(now)),
    };
    match chat_message::Entity::insert(am).exec(db).await {
        Ok(res) => Ok(chat_message::Entity::find_by_id(res.last_insert_id)
            .one(db)
            .await?),
        Err(sea_orm::DbErr::RecordNotInserted) => Ok(None),
        Err(e) => Err(rc_core::Error::Internal(format!("upsert_message: {e}"))),
    }
}

pub async fn get_message_by_id(db: &DatabaseConnection, id: &str) -> Result<Option<ChatMessage>> {
    Ok(chat_message::Entity::find_by_id(id).one(db).await?)
}

/// All messages of a chat, oldest first (created_at, then insertion id).
pub async fn get_messages_by_chat_id(
    db: &DatabaseConnection,
    chat_id: &str,
) -> Result<Vec<ChatMessage>> {
    Ok(chat_message::Entity::find()
        .filter(chat_message::Column::ChatId.eq(chat_id))
        .order_by_asc(chat_message::Column::CreatedAt)
        .order_by_asc(chat_message::Column::Id)
        .all(db)
        .await?)
}

pub async fn delete_messages_by_chat_id(db: &DatabaseConnection, chat_id: &str) -> Result<bool> {
    let res = chat_message::Entity::delete_many()
        .filter(chat_message::Column::ChatId.eq(chat_id))
        .exec(db)
        .await?;
    Ok(res.rows_affected > 0)
}

/// Delete message rows by their *blob* message ids for one chat (maps to
/// composite `{chat_id}-{message_id}`; also matches legacy bare ids).
pub async fn delete_messages_by_blob_ids(
    db: &DatabaseConnection,
    chat_id: &str,
    message_ids: &[String],
) -> Result<bool> {
    let mut ids: Vec<String> = message_ids
        .iter()
        .map(|m| composite_id(chat_id, m))
        .collect();
    ids.extend(message_ids.iter().cloned());
    let res = chat_message::Entity::delete_many()
        .filter(chat_message::Column::Id.is_in(ids))
        .exec(db)
        .await?;
    Ok(res.rows_affected > 0)
}
