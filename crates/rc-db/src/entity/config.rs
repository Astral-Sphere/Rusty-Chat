use sea_orm::entity::prelude::*;

/// `config` table — per-key persistent configuration (open-webui compat:
/// dotted keys like `ui.enable_signup`; DB value wins over env after seed).
#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "config")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub key: String,
    pub value: Json,
    /// epoch seconds
    pub updated_at: Option<i64>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
