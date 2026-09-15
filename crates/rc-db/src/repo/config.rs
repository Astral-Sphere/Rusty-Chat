//! Per-key `config` table engine — open-webui compat: dotted keys, JSON
//! values, DB wins over env after first seed.

use crate::entity::config;
use rc_core::Result;
use rc_core::timestamp::Secs;
use sea_orm::{ActiveModelTrait, ActiveValue::Set, DatabaseConnection, EntityTrait};
use std::collections::BTreeMap;

pub struct ConfigEngine {
    db: DatabaseConnection,
}

impl ConfigEngine {
    pub fn new(db: DatabaseConnection) -> Self {
        Self { db }
    }

    pub async fn get(&self, key: &str) -> Result<Option<serde_json::Value>> {
        Ok(config::Entity::find_by_id(key)
            .one(&self.db)
            .await?
            .map(|row| row.value))
    }

    /// `get` with a fallback value (returned untouched when the key is absent).
    pub async fn get_or(&self, key: &str, default: serde_json::Value) -> Result<serde_json::Value> {
        Ok(self.get(key).await?.unwrap_or(default))
    }

    pub async fn upsert(&self, key: &str, value: &serde_json::Value) -> Result<()> {
        let now = Secs::now().as_i64();
        let existing = config::Entity::find_by_id(key).one(&self.db).await?;
        match existing {
            Some(row) => {
                let mut am: config::ActiveModel = row.into();
                am.value = Set(value.clone());
                am.updated_at = Set(Some(now));
                am.update(&self.db).await?;
            }
            None => {
                let am = config::ActiveModel {
                    key: Set(key.to_string()),
                    value: Set(value.clone()),
                    updated_at: Set(Some(now)),
                };
                am.insert(&self.db).await?;
            }
        }
        Ok(())
    }

    /// Seed missing keys from the DEFAULT_CONFIG registry (env-derived
    /// defaults). Existing rows are never overwritten. Returns how many
    /// keys were inserted.
    pub async fn seed_defaults(
        &self,
        defaults: &BTreeMap<String, serde_json::Value>,
    ) -> Result<usize> {
        let now = Secs::now().as_i64();
        let mut seeded = 0;
        for (key, value) in defaults {
            if config::Entity::find_by_id(key)
                .one(&self.db)
                .await?
                .is_some()
            {
                continue;
            }
            let am = config::ActiveModel {
                key: Set(key.clone()),
                value: Set(value.clone()),
                updated_at: Set(Some(now)),
            };
            am.insert(&self.db).await?;
            seeded += 1;
        }
        Ok(seeded)
    }

    /// All key/value rows (admin export).
    pub async fn all(&self) -> Result<BTreeMap<String, serde_json::Value>> {
        let rows = config::Entity::find().all(&self.db).await?;
        Ok(rows.into_iter().map(|r| (r.key, r.value)).collect())
    }

    pub async fn delete(&self, key: &str) -> Result<bool> {
        let res = config::Entity::delete_by_id(key).exec(&self.db).await?;
        Ok(res.rows_affected > 0)
    }
}
