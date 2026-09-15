//! Schema bootstrap and alembic-version guard.
//!
//! Rusty-Chat does not run its own migration system. Compatibility contract:
//! - Fresh database → create the exact schema open-webui 0.11.3 produces at
//!   alembic head `d4c1a8e37b62`, and stamp `alembic_version` with that
//!   revision, so open-webui itself can open the database afterwards.
//! - Existing database → verify `alembic_version == head`; refuse older
//!   schemas with instructions to migrate via open-webui first.
//!
//! The DDL files are dumps of real databases created by running open-webui's
//! own alembic chain (SQLite + Postgres), see `docs/COMPATIBILITY.md`.

use rc_core::{COMPATIBLE_ALEMBIC_HEAD, Error, Result};

/// SQL that creates the full head schema and stamps `alembic_version`.
pub const SQLITE_DDL: &str = include_str!("ddl/sqlite.sql");
pub const POSTGRES_DDL: &str = include_str!("ddl/postgres.sql");

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dialect {
    Sqlite,
    Postgres,
}

impl Dialect {
    pub fn ddl(self) -> &'static str {
        match self {
            Dialect::Sqlite => SQLITE_DDL,
            Dialect::Postgres => POSTGRES_DDL,
        }
    }
}

/// What [`inspect_database`] decided about the database it looked at.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DatabaseState {
    /// Empty (no tables at all) — safe to bootstrap.
    Fresh,
    /// Schema exists and is stamped with exactly the compatible head revision.
    Compatible,
    /// Schema exists but stamped with a different alembic revision.
    WrongRevision { found: String },
    /// Legacy pre-alembic open-webui (< 0.6) — has our tables but no stamp.
    LegacyUnstamped,
}

/// Reads `alembic_version` if present.
///
/// `conn` is raw sqlx rather than SeaORM because this runs before any entity
/// machinery and must work identically on both dialects.
pub async fn inspect_database(conn: &mut sqlx::AnyConnection) -> Result<DatabaseState> {
    let has_alembic: bool = table_exists(conn, "alembic_version").await?;
    if !has_alembic {
        let has_user = table_exists(conn, "user").await?;
        let has_auth = table_exists(conn, "auth").await?;
        if has_user || has_auth {
            return Ok(DatabaseState::LegacyUnstamped);
        }
        return Ok(DatabaseState::Fresh);
    }

    let revision: Option<String> = sqlx::query_scalar("SELECT version_num FROM alembic_version")
        .fetch_optional(conn)
        .await
        .map_err(|e| Error::Internal(format!("failed to read alembic_version: {e}")))?;

    match revision {
        Some(r) if r == COMPATIBLE_ALEMBIC_HEAD => Ok(DatabaseState::Compatible),
        Some(r) => Ok(DatabaseState::WrongRevision { found: r }),
        None => Ok(DatabaseState::LegacyUnstamped),
    }
}

/// Bootstraps a fresh database by executing the dialect DDL.
///
/// Callers must have checked [`inspect_database`] first; this function
/// refuses to touch anything that is not [`DatabaseState::Fresh`].
pub async fn bootstrap(conn: &mut sqlx::AnyConnection, dialect: Dialect) -> Result<()> {
    match inspect_database(conn).await? {
        DatabaseState::Fresh => {}
        DatabaseState::Compatible => return Ok(()),
        DatabaseState::WrongRevision { found } => {
            return Err(Error::DatabaseIncompatible(format!(
                "database is at alembic revision {found}, this build requires \
                 {COMPATIBLE_ALEMBIC_HEAD} (open-webui 0.11.3). Migrate with \
                 open-webui first, then retry."
            )));
        }
        DatabaseState::LegacyUnstamped => {
            return Err(Error::DatabaseIncompatible(
                "database has open-webui tables but no alembic_version stamp \
                 (pre-0.6 layout). Open it once with open-webui >= 0.6 to \
                 migrate, then retry."
                    .to_string(),
            ));
        }
    }

    // The DDL is our own embedded file (not user input), and raw_sql
    // executes multi-statement scripts on both dialects.
    sqlx::raw_sql(dialect.ddl())
        .execute(conn)
        .await
        .map_err(|e| Error::Internal(format!("bootstrap DDL failed: {e}")))?;
    Ok(())
}

async fn table_exists(conn: &mut sqlx::AnyConnection, table: &str) -> Result<bool> {
    // Detect dialect by capability: SQLite answers sqlite_master, Postgres
    // (and everything else) answers information_schema. Avoids depending on
    // backend_name() spelling.
    #[allow(clippy::explicit_auto_deref)] // reborrow is required: `conn` feeds two probes
    let sqlite: std::result::Result<i64, _> =
        sqlx::query_scalar("SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = ?")
            .bind(table)
            .fetch_one(&mut *conn)
            .await;
    if let Ok(n) = sqlite {
        return Ok(n > 0);
    }
    #[allow(clippy::explicit_auto_deref)] // second probe consumes the reborrow
    let n: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM information_schema.tables \
         WHERE table_schema = current_schema() AND table_name = $1",
    )
    .bind(table)
    .fetch_one(&mut *conn)
    .await
    .map_err(|e| Error::Internal(format!("failed to probe tables: {e}")))?;
    Ok(n > 0)
}
