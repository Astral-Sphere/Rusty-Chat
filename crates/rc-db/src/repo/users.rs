//! `Users` — mirrors open-webui `models/users.py::UsersTable` (M1 subset).

use crate::entity::{api_key, user};
use rc_core::Result;
use rc_core::timestamp::Secs;
use sea_orm::{
    ActiveModelTrait,
    ActiveValue::Set,
    ColumnTrait, Condition, DatabaseConnection, EntityTrait, IntoActiveModel, PaginatorTrait,
    QueryFilter, QueryOrder, QuerySelect,
    sea_query::{Expr, ExprTrait, Func},
};

pub use user::Model as User;

/// Patch for `update_user_by_id` — only `Some` fields are written.
#[derive(Debug, Default, Clone)]
pub struct UserPatch {
    pub role: Option<String>,
    pub name: Option<String>,
    pub email: Option<String>,
    pub profile_image_url: Option<String>,
    pub bio: Option<String>,
    pub gender: Option<String>,
    pub timezone: Option<String>,
    pub settings: Option<serde_json::Value>,
    pub variables: Option<serde_json::Value>,
    pub info: Option<serde_json::Value>,
    pub last_active_at: Option<Secs>,
}

/// open-webui `validate_profile_image_url` fallback: invalid values become
/// `/user.png` (see utils/validate.py — relative paths and http(s) URLs).
fn sanitize_profile_image_url(url: &str) -> String {
    let ok = url.starts_with('/')
        || url.starts_with("http://")
        || url.starts_with("https://")
        || url.starts_with("data:image/");
    if ok {
        url.to_string()
    } else {
        "/user.png".to_string()
    }
}

/// Arguments for [`insert_new_user`].
pub struct NewUserParams<'a> {
    pub id: &'a str,
    pub name: &'a str,
    pub email: &'a str,
    pub profile_image_url: Option<&'a str>,
    pub role: Option<&'a str>,
    pub username: Option<&'a str>,
    pub oauth: Option<serde_json::Value>,
}

/// `Users.insert_new_user` — timestamps = now; profile image sanitized.
pub async fn insert_new_user(
    db: &DatabaseConnection,
    p: NewUserParams<'_>,
) -> Result<Option<User>> {
    let now = Secs::now().as_i64();
    let image = sanitize_profile_image_url(p.profile_image_url.unwrap_or("/user.png"));
    let active = user::ActiveModel {
        id: Set(p.id.to_string()),
        email: Set(Some(p.email.to_string())),
        username: Set(p.username.map(str::to_string)),
        role: Set(Some(p.role.unwrap_or("pending").to_string())),
        name: Set(p.name.to_string()),
        profile_image_url: Set(Some(image)),
        profile_banner_image_url: Set(None),
        bio: Set(None),
        gender: Set(None),
        date_of_birth: Set(None),
        timezone: Set(None),
        presence_state: Set(None),
        status_emoji: Set(None),
        status_message: Set(None),
        status_expires_at: Set(None),
        info: Set(None),
        variables: Set(None),
        settings: Set(None),
        oauth: Set(p.oauth),
        scim: Set(None),
        last_active_at: Set(Some(now)),
        updated_at: Set(Some(now)),
        created_at: Set(Some(now)),
    };
    match user::Entity::insert(active).exec(db).await {
        Ok(_) => Ok(get_user_by_id(db, p.id).await?),
        Err(sea_orm::DbErr::RecordNotInserted) => Ok(None),
        Err(e) => Err(rc_core::Error::Internal(format!("insert_new_user: {e}"))),
    }
}

pub async fn get_user_by_id(db: &DatabaseConnection, id: &str) -> Result<Option<User>> {
    Ok(user::Entity::find_by_id(id).one(db).await?)
}

/// Case-insensitive email lookup (`lower(email) == lower(?)`).
pub async fn get_user_by_email(db: &DatabaseConnection, email: &str) -> Result<Option<User>> {
    let cond = Condition::all().add(
        Expr::expr(Func::cust("lower").arg(Expr::col(user::Column::Email)))
            .eq(Expr::val(email.to_lowercase())),
    );
    Ok(user::Entity::find().filter(cond).one(db).await?)
}

/// Resolve user from plaintext API key (JOIN semantics via two-step lookup —
/// `api_key.key` is unique, so this is exactly the JOIN result).
pub async fn get_user_by_api_key(db: &DatabaseConnection, api_key: &str) -> Result<Option<User>> {
    if api_key.is_empty() {
        return Ok(None);
    }
    let Some(key) = api_key::Entity::find()
        .filter(api_key::Column::Key.eq(api_key))
        .one(db)
        .await?
    else {
        return Ok(None);
    };
    get_user_by_id(db, &key.user_id.unwrap_or_default()).await
}

pub async fn get_num_users(db: &DatabaseConnection) -> Result<u64> {
    Ok(user::Entity::find().count(db).await?)
}

/// `Users.get_users` (M1 subset): `query` filter on name/email, order_by in
/// {created_at|name|email|last_active_at|updated_at|role} × asc/desc,
/// count-before-pagination.
pub async fn get_users(
    db: &DatabaseConnection,
    query: Option<&str>,
    order_by: Option<&str>,
    direction: Option<&str>,
    skip: Option<u64>,
    limit: Option<u64>,
) -> Result<(Vec<User>, u64)> {
    let backend = db.get_database_backend();
    let mut select = user::Entity::find();
    if let Some(q) = query.filter(|q| !q.is_empty()) {
        let name_like = super::ci_like(user::Column::Name, q, backend);
        let email_like = super::ci_like(user::Column::Email, q, backend);
        select = select.filter(Condition::any().add(name_like).add(email_like));
    }
    let desc = direction
        .map(|d| d.eq_ignore_ascii_case("desc"))
        .unwrap_or(true);
    let col = match order_by {
        Some("name") => user::Column::Name,
        Some("email") => user::Column::Email,
        Some("last_active_at") => user::Column::LastActiveAt,
        Some("updated_at") => user::Column::UpdatedAt,
        Some("role") => user::Column::Role,
        _ => user::Column::CreatedAt,
    };
    select = if desc {
        select.order_by_desc(col)
    } else {
        select.order_by_asc(col)
    };

    let total = select.clone().count(db).await?;
    if let Some(s) = skip {
        select = select.offset(s);
    }
    if let Some(l) = limit {
        select = select.limit(l);
    }
    Ok((select.all(db).await?, total))
}

pub async fn update_user_by_id(
    db: &DatabaseConnection,
    id: &str,
    patch: UserPatch,
) -> Result<Option<User>> {
    let Some(existing) = get_user_by_id(db, id).await? else {
        return Ok(None);
    };
    let mut am = existing.into_active_model();
    if let Some(v) = patch.role {
        am.role = Set(Some(v));
    }
    if let Some(v) = patch.name {
        am.name = Set(v);
    }
    if let Some(v) = patch.email {
        am.email = Set(Some(v));
    }
    if let Some(v) = patch.profile_image_url {
        am.profile_image_url = Set(Some(sanitize_profile_image_url(&v)));
    }
    if let Some(v) = patch.bio {
        am.bio = Set(Some(v));
    }
    if let Some(v) = patch.gender {
        am.gender = Set(Some(v));
    }
    if let Some(v) = patch.timezone {
        am.timezone = Set(Some(v));
    }
    if let Some(v) = patch.settings {
        am.settings = Set(Some(v));
    }
    if let Some(v) = patch.variables {
        am.variables = Set(Some(v));
    }
    if let Some(v) = patch.info {
        am.info = Set(Some(v));
    }
    if let Some(v) = patch.last_active_at {
        am.last_active_at = Set(Some(v.as_i64()));
    }
    am.updated_at = Set(Some(Secs::now().as_i64()));
    let updated = am.update(db).await?;
    Ok(Some(updated))
}

pub async fn delete_user_by_id(db: &DatabaseConnection, id: &str) -> Result<bool> {
    // api_key rows cascade via FK (both dialects; sqlx enables foreign_keys).
    let res = user::Entity::delete_by_id(id).exec(db).await?;
    Ok(res.rows_affected > 0)
}
