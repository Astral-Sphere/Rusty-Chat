use sea_orm::entity::prelude::*;

/// `chat` table — the core conversation entity. The `chat` JSON blob holds
/// the full message tree (`history.messages` map + `currentId`) and is kept
/// in sync with the `chat_message` table (see `crate::history`).
#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "chat")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: String,
    pub user_id: Option<String>,
    pub title: Option<String>,
    /// Full conversation blob: {title, models, history:{messages,currentId}, ...}
    pub chat: Option<Json>,
    /// epoch seconds
    pub created_at: Option<i64>,
    /// epoch seconds
    pub updated_at: Option<i64>,
    #[sea_orm(unique)]
    pub share_id: Option<String>,
    pub archived: Option<bool>,
    pub pinned: Option<bool>,
    /// server_default '{}'
    pub meta: Option<Json>,
    pub variables: Option<Json>,
    pub folder_id: Option<String>,
    pub tasks: Option<Json>,
    pub summary: Option<String>,
    /// pointer into the chat_message table
    pub current_message_id: Option<String>,
    /// epoch seconds
    pub last_read_at: Option<i64>,
    /// epoch NANOS (due time for timer chats) — same row as second-based
    /// created_at; see docs/COMPATIBILITY.md §2.
    pub timer_at: Option<i64>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
