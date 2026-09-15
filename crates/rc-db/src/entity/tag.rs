use sea_orm::entity::prelude::*;

/// `tag` table — composite primary key (id, user_id); `id` is the slug form
/// of the tag name (`name.replace(' ', '_').lower()`).
#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "tag")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: String,
    #[sea_orm(primary_key, auto_increment = false)]
    pub user_id: String,
    pub name: Option<String>,
    pub meta: Option<Json>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
