use sea_orm::entity::prelude::*;

/// `api_key` table. `key` is stored PLAINTEXT (open-webui compat), format
/// `sk-` + 32 hex chars.
#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "api_key")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: String,
    pub user_id: Option<String>,
    #[sea_orm(unique)]
    pub key: Option<String>,
    pub data: Option<Json>,
    /// epoch seconds
    pub expires_at: Option<i64>,
    /// epoch seconds
    pub last_used_at: Option<i64>,
    /// epoch seconds
    pub created_at: Option<i64>,
    /// epoch seconds
    pub updated_at: Option<i64>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
