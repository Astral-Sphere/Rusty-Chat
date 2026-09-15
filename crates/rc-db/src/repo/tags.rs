//! `Tags` — mirrors open-webui `models/tags.py` (M1 subset).

use crate::entity::tag;
use rc_core::Result;
use sea_orm::{
    ActiveValue::Set, ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, QueryOrder,
};

pub use tag::Model as Tag;

/// Slug derivation: `name.replace(' ', '_').lower()`.
pub fn tag_id_from_name(name: &str) -> String {
    name.replace(' ', "_").to_lowercase()
}

pub async fn insert_new_tag(
    db: &DatabaseConnection,
    name: &str,
    user_id: &str,
) -> Result<Option<Tag>> {
    let am = tag::ActiveModel {
        id: Set(tag_id_from_name(name)),
        user_id: Set(user_id.to_string()),
        name: Set(Some(name.to_string())),
        meta: Set(None),
    };
    match tag::Entity::insert(am).exec(db).await {
        Ok(_) => Ok(
            tag::Entity::find_by_id((tag_id_from_name(name), user_id.to_string()))
                .one(db)
                .await?,
        ),
        Err(sea_orm::DbErr::RecordNotInserted) => Ok(None),
        Err(e) => Err(rc_core::Error::Internal(format!("insert_new_tag: {e}"))),
    }
}

/// `Tags.ensure_tags_exist`.
pub async fn ensure_tags_exist(
    db: &DatabaseConnection,
    tag_names: &[&str],
    user_id: &str,
) -> Result<()> {
    for name in tag_names {
        let id = tag_id_from_name(name);
        if tag::Entity::find_by_id((id, user_id.to_string()))
            .one(db)
            .await?
            .is_none()
        {
            insert_new_tag(db, name, user_id).await?;
        }
    }
    Ok(())
}

pub async fn get_tags_by_user_id(db: &DatabaseConnection, user_id: &str) -> Result<Vec<Tag>> {
    Ok(tag::Entity::find()
        .filter(tag::Column::UserId.eq(user_id))
        .order_by_asc(tag::Column::Name)
        .all(db)
        .await?)
}

pub async fn delete_tag_by_id_and_user_id(
    db: &DatabaseConnection,
    id: &str,
    user_id: &str,
) -> Result<bool> {
    let res = tag::Entity::delete_by_id((id.to_string(), user_id.to_string()))
        .exec(db)
        .await?;
    Ok(res.rows_affected > 0)
}

/// `Chats.delete_orphan_tags_for_user`: delete tags whose id no longer
/// appears in any chat row's meta.tags for this user. M1 simplification:
/// callers pass the ids still in use; we delete the given ids that are NOT
/// referenced by any chat. (The full scan query lives in chats.rs.)
pub async fn delete_orphan_tags_for_user(
    db: &DatabaseConnection,
    candidate_ids: &[String],
    user_id: &str,
    chat_tag_ids_in_use: &[String],
) -> Result<usize> {
    let mut deleted = 0;
    for id in candidate_ids {
        if chat_tag_ids_in_use.contains(id) {
            continue;
        }
        if delete_tag_by_id_and_user_id(db, id, user_id).await? {
            deleted += 1;
        }
    }
    Ok(deleted)
}

/// Distinct tag ids referenced by a user's chat rows' meta.tags
/// (`delete_orphan_tags_for_user`'s source of truth).
pub async fn collect_chat_tag_ids_in_use(
    db: &DatabaseConnection,
    user_id: &str,
) -> Result<Vec<String>> {
    use sea_orm::{DbBackend, Value};

    let backend = db.get_database_backend();
    let (sql, val) = match backend {
        DbBackend::Sqlite => (
            "SELECT DISTINCT je.value AS tag_id FROM chat c, json_each(json_extract(c.meta, '$.tags')) je \
             WHERE c.user_id = ? AND json_valid(json_extract(c.meta, '$.tags'))",
            Value::from(user_id.to_string()),
        ),
        _ => (
            // meta is `json` (not jsonb); guard non-array values via json_typeof.
            "SELECT DISTINCT t::text AS tag_id FROM chat c, \
             json_array_elements_text(CASE WHEN json_typeof(c.meta->'tags') = 'array' THEN c.meta->'tags' END) t \
             WHERE c.user_id = $1",
            Value::from(user_id.to_string()),
        ),
    };
    let _ = val;
    let rows: Vec<String> = match backend {
        DbBackend::Sqlite => sqlx::query_scalar(sql)
            .bind(user_id)
            .fetch_all(db.get_sqlite_connection_pool())
            .await
            .map_err(|e| rc_core::Error::Internal(format!("collect_chat_tag_ids_in_use: {e}")))?,
        _ => sqlx::query_scalar(sql)
            .bind(user_id)
            .fetch_all(db.get_postgres_connection_pool())
            .await
            .map_err(|e| rc_core::Error::Internal(format!("collect_chat_tag_ids_in_use: {e}")))?,
    };
    Ok(rows)
}
