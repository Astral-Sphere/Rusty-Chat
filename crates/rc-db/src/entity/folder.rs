use sea_orm::entity::prelude::*;

/// `folder` table — nested folders for chats (and later prompts/models).
#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "folder")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: String,
    pub parent_id: Option<String>,
    pub user_id: Option<String>,
    pub name: Option<String>,
    /// legacy items listing
    pub items: Option<Json>,
    pub meta: Option<Json>,
    pub data: Option<Json>,
    pub is_expanded: Option<bool>,
    /// epoch seconds
    pub created_at: Option<i64>,
    /// epoch seconds
    pub updated_at: Option<i64>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
