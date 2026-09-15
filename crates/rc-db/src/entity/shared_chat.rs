use sea_orm::entity::prelude::*;

/// `shared_chat` table — snapshot of a chat at share time; `id` is the share
/// token used in `/s/{id}` URLs.
#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "shared_chat")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: String,
    pub chat_id: Option<String>,
    pub user_id: Option<String>,
    pub title: Option<String>,
    pub chat: Option<Json>,
    /// epoch seconds
    pub created_at: Option<i64>,
    /// epoch seconds
    pub updated_at: Option<i64>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
