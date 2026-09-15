use sea_orm::entity::prelude::*;

/// `chat_message` table — normalized message store, source of truth for chat
/// contents (the `chat.chat` blob is the denormalized mirror).
#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "chat_message")]
pub struct Model {
    /// composite `{chat_id}-{message_id}`
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: String,
    pub chat_id: Option<String>,
    pub user_id: Option<String>,
    /// `user | assistant | system`
    pub role: Option<String>,
    pub parent_id: Option<String>,
    /// string OR list of content blocks
    pub content: Option<Json>,
    /// list of OR-style output items (assistant)
    pub output: Option<Json>,
    pub model_id: Option<String>,
    pub files: Option<Json>,
    pub sources: Option<Json>,
    pub embeds: Option<Json>,
    pub meta: Option<Json>,
    pub done: Option<bool>,
    pub status_history: Option<Json>,
    pub error: Option<Json>,
    pub usage: Option<Json>,
    pub context_summary: Option<String>,
    /// epoch seconds
    pub created_at: Option<i64>,
    /// epoch seconds
    pub updated_at: Option<i64>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
