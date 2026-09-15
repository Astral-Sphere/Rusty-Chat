use sea_orm::entity::prelude::*;

/// `user` table — identity & profile. `id` mirrors `auth.id`.
#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "user")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: String,
    pub email: Option<String>,
    pub username: Option<String>,
    /// `pending | user | admin`
    pub role: Option<String>,
    pub name: String,
    pub profile_image_url: Option<String>,
    pub profile_banner_image_url: Option<String>,
    pub bio: Option<String>,
    pub gender: Option<String>,
    pub date_of_birth: Option<Date>,
    pub timezone: Option<String>,
    pub presence_state: Option<String>,
    pub status_emoji: Option<String>,
    pub status_message: Option<String>,
    /// epoch seconds
    pub status_expires_at: Option<i64>,
    pub info: Option<Json>,
    pub variables: Option<Json>,
    /// Frontend-owned settings blob — merge, never replace wholesale.
    pub settings: Option<Json>,
    /// `{provider: {sub, email, ...}}`
    pub oauth: Option<Json>,
    /// `{provider: {external_id: ...}}` (SCIM)
    pub scim: Option<Json>,
    /// epoch seconds
    pub last_active_at: Option<i64>,
    /// epoch seconds
    pub updated_at: Option<i64>,
    /// epoch seconds
    pub created_at: Option<i64>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
