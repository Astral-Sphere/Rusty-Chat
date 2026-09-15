//! `SharedChats` — mirrors open-webui `models/shared_chats.py`.

use crate::entity::{chat, shared_chat};
use rc_core::Result;
use rc_core::timestamp::Secs;
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter,
};

pub use shared_chat::Model as SharedChat;

/// `SharedChats.create`: snapshot the chat under a fresh share token.
pub async fn create(
    db: &DatabaseConnection,
    chat_id: &str,
    user_id: &str,
) -> Result<Option<SharedChat>> {
    let Some(chat_row) = chat::Entity::find_by_id(chat_id).one(db).await? else {
        return Ok(None);
    };
    let now = Secs::now().as_i64();
    let am = shared_chat::ActiveModel {
        id: Set(uuid::Uuid::new_v4().to_string()),
        chat_id: Set(Some(chat_id.to_string())),
        user_id: Set(Some(user_id.to_string())),
        title: Set(chat_row.title.clone().or(Some("New Chat".to_string()))),
        chat: Set(Some(
            chat_row.chat.clone().unwrap_or(serde_json::Value::Null),
        )),
        created_at: Set(Some(now)),
        updated_at: Set(Some(now)),
    };
    Ok(Some(am.insert(db).await?))
}

/// `SharedChats.update`: re-snapshot the current chat content.
pub async fn update(db: &DatabaseConnection, share_id: &str) -> Result<Option<SharedChat>> {
    let Some(shared) = shared_chat::Entity::find_by_id(share_id).one(db).await? else {
        return Ok(None);
    };
    let Some(chat_id) = shared.chat_id.clone() else {
        return Ok(None);
    };
    let Some(chat_row) = chat::Entity::find_by_id(&chat_id).one(db).await? else {
        return Ok(None);
    };
    let mut am: shared_chat::ActiveModel = shared.into();
    am.title = Set(chat_row.title.clone().or(Some("New Chat".to_string())));
    am.chat = Set(Some(
        chat_row.chat.clone().unwrap_or(serde_json::Value::Null),
    ));
    am.updated_at = Set(Some(Secs::now().as_i64()));
    Ok(Some(am.update(db).await?))
}

pub async fn get_by_id(db: &DatabaseConnection, share_id: &str) -> Result<Option<SharedChat>> {
    Ok(shared_chat::Entity::find_by_id(share_id).one(db).await?)
}

pub async fn get_chat_id_by_share_id(
    db: &DatabaseConnection,
    share_id: &str,
) -> Result<Option<String>> {
    Ok(get_by_id(db, share_id).await?.and_then(|s| s.chat_id))
}

pub async fn delete_by_id(db: &DatabaseConnection, share_id: &str) -> Result<bool> {
    let res = shared_chat::Entity::delete_by_id(share_id).exec(db).await?;
    Ok(res.rows_affected > 0)
}

/// `SharedChats.delete_by_chat_id`.
pub async fn delete_by_chat_id(db: &DatabaseConnection, chat_id: &str) -> Result<bool> {
    let res = shared_chat::Entity::delete_many()
        .filter(shared_chat::Column::ChatId.eq(chat_id))
        .exec(db)
        .await?;
    Ok(res.rows_affected > 0)
}

pub async fn get_shared_chats_by_user(
    db: &DatabaseConnection,
    user_id: &str,
) -> Result<Vec<(SharedChat, Option<chat::Model>)>> {
    use sea_orm::QueryOrder;
    let shared = shared_chat::Entity::find()
        .filter(shared_chat::Column::UserId.eq(user_id))
        .order_by_desc(shared_chat::Column::UpdatedAt)
        .all(db)
        .await?;
    let mut out = vec![];
    for s in shared {
        let original = match &s.chat_id {
            Some(id) => chat::Entity::find_by_id(id).one(db).await?,
            None => None,
        };
        out.push((s, original));
    }
    Ok(out)
}
