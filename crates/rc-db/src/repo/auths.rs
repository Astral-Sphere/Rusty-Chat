//! `Auths` — mirrors open-webui `models/auths.py::AuthsTable`.

use crate::entity::auth;
use crate::repo::users::User;
use crate::repo::{users, users::UserPatch};
use rc_core::Result;
use rc_core::timestamp::Secs;
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter,
};

pub use auth::Model as Auth;

/// What to do about password verification — provided by `rc-auth` so hashing
/// stays out of the data layer.
// Crate-internal trait (object safety / auto traits not needed).
#[allow(async_fn_in_trait)]
pub trait VerifyPassword {
    /// Returns true if the hash matches the candidate password.
    async fn verify(&self, hash: &str) -> bool;
}

#[derive(Debug, Clone)]
pub struct SignupParams<'a> {
    pub email: &'a str,
    pub password_hash: &'a str,
    pub name: &'a str,
    pub profile_image_url: Option<&'a str>,
    pub role: Option<&'a str>,
    pub oauth: Option<serde_json::Value>,
}

/// `Auths.insert_new_auth`: create auth+user pair. Returns the created user.
///
/// NOTE (parity): the caller performs the "first user becomes admin"
/// post-check exactly like open-webui's signup router (see
/// `repo::users::get_num_users`); this function does not do it itself.
pub async fn insert_new_auth(
    db: &DatabaseConnection,
    params: SignupParams<'_>,
) -> Result<Option<User>> {
    let id = uuid::Uuid::new_v4().to_string();
    let am = auth::ActiveModel {
        id: Set(id.clone()),
        email: Set(Some(params.email.to_string())),
        password: Set(Some(params.password_hash.to_string())),
        active: Set(Some(true)),
    };
    // Match open-webui behavior: a duplicate email aborts the whole insert.
    let res = auth::Entity::insert(am).exec(db).await;
    if let Err(e) = res {
        return Err(rc_core::Error::BadRequest(match e {
            sea_orm::DbErr::Exec(source) => {
                format!("credential insert failed (duplicate email?): {source}")
            }
            other => format!("credential insert failed: {other}"),
        }));
    }

    let user = users::insert_new_user(
        db,
        users::NewUserParams {
            id: &id,
            name: params.name,
            email: params.email,
            profile_image_url: params.profile_image_url,
            role: params.role,
            username: None,
            oauth: params.oauth,
        },
    )
    .await?;
    Ok(user)
}

/// `Auths.authenticate_user`: resolve by (lowercased) email, verify against
/// the credential row; unknown user / inactive account still burn a bcrypt
/// verify against `placeholder_hash` so timing cannot reveal existence.
pub async fn authenticate_user(
    db: &DatabaseConnection,
    email: &str,
    _password: &str,
    verifier: &impl VerifyPassword,
    placeholder_hash: &str,
) -> Result<Option<User>> {
    let Some(resolved) = users::get_user_by_email(db, email).await? else {
        verifier.verify(placeholder_hash).await;
        return Ok(None);
    };
    let Some(credential) = auth::Entity::find_by_id(&resolved.id).one(db).await? else {
        verifier.verify(placeholder_hash).await;
        return Ok(None);
    };
    if !credential.active.unwrap_or(false) {
        verifier.verify(placeholder_hash).await;
        return Ok(None);
    }
    let Some(hash) = credential.password.as_deref() else {
        return Ok(None);
    };
    if !verifier.verify(hash).await {
        return Ok(None);
    }
    Ok(Some(resolved))
}

/// `Auths.authenticate_user_by_api_key`.
pub async fn authenticate_user_by_api_key(
    db: &DatabaseConnection,
    api_key: &str,
) -> Result<Option<User>> {
    users::get_user_by_api_key(db, api_key).await
}

/// `Auths.update_email_by_id`: update auth row then mirror to user row.
pub async fn update_email_by_id(
    db: &DatabaseConnection,
    user_id: &str,
    email: &str,
) -> Result<bool> {
    let Some(row) = auth::Entity::find_by_id(user_id).one(db).await? else {
        return Ok(false);
    };
    let mut am: auth::ActiveModel = row.into();
    am.email = Set(Some(email.to_string()));
    am.update(db).await?;
    users::update_user_by_id(
        db,
        user_id,
        UserPatch {
            email: Some(email.to_string()),
            ..Default::default()
        },
    )
    .await?;
    Ok(true)
}

/// `Auths.update_user_password_by_id` — takes an already-hashed password.
pub async fn update_user_password_by_id(
    db: &DatabaseConnection,
    user_id: &str,
    password_hash: &str,
) -> Result<bool> {
    let Some(row) = auth::Entity::find_by_id(user_id).one(db).await? else {
        return Ok(false);
    };
    let mut am: auth::ActiveModel = row.into();
    am.password = Set(Some(password_hash.to_string()));
    am.update(db).await?;
    Ok(true)
}

/// `Auths.delete_auth_by_id`: remove user row then auth row.
pub async fn delete_auth_by_id(db: &DatabaseConnection, id: &str) -> Result<bool> {
    if !users::delete_user_by_id(db, id).await? {
        return Ok(false);
    }
    auth::Entity::delete_by_id(id).exec(db).await?;
    Ok(true)
}

/// API key lifecycle lives with users in open-webui (`Auths` router); keep
/// the data access here so rc-auth needs no SQL.
pub mod api_keys {
    use super::*;
    use crate::entity::api_key;

    pub async fn create(
        db: &DatabaseConnection,
        user_id: &str,
        key: &str,
    ) -> Result<api_key::Model> {
        let now = Secs::now().as_i64();
        let am = api_key::ActiveModel {
            id: Set(uuid::Uuid::new_v4().to_string()),
            user_id: Set(Some(user_id.to_string())),
            key: Set(Some(key.to_string())),
            data: Set(None),
            expires_at: Set(None),
            last_used_at: Set(None),
            created_at: Set(Some(now)),
            updated_at: Set(Some(now)),
        };
        Ok(am.insert(db).await?)
    }

    pub async fn get_by_user_id(
        db: &DatabaseConnection,
        user_id: &str,
    ) -> Result<Option<api_key::Model>> {
        Ok(api_key::Entity::find()
            .filter(api_key::Column::UserId.eq(user_id))
            .one(db)
            .await?)
    }

    pub async fn delete_by_user_id(db: &DatabaseConnection, user_id: &str) -> Result<bool> {
        let res = api_key::Entity::delete_many()
            .filter(api_key::Column::UserId.eq(user_id))
            .exec(db)
            .await?;
        Ok(res.rows_affected > 0)
    }

    pub async fn touch_last_used(db: &DatabaseConnection, key: &str) -> Result<()> {
        if let Some(row) = api_key::Entity::find()
            .filter(api_key::Column::Key.eq(key))
            .one(db)
            .await?
        {
            let mut am: api_key::ActiveModel = row.into();
            am.last_used_at = Set(Some(Secs::now().as_i64()));
            am.update(db).await?;
        }
        Ok(())
    }
}
