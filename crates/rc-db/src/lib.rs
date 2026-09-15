//! # rc-db
//!
//! Database layer for Rusty-Chat: SeaORM entities and repositories that are
//! byte-compatible with open-webui 0.11.3 databases (alembic head
//! `d4c1a8e37b62`), DDL bootstrap for fresh databases, and the per-key
//! `config` table engine.
//!
//! Layout:
//! - [`bootstrap`] — schema creation + alembic-version guard
//! - `entity::` — SeaORM entities (added per milestone)
//! - `repo::` — repositories (added per milestone)

pub mod bootstrap;

pub use rc_core::{COMPATIBLE_ALEMBIC_HEAD, Error, Result};

/// Installs sqlx Any-driver backends (SQLite + Postgres).
///
/// Required once per process before any `sqlx::Any*` type is used; safe to
/// call repeatedly. Server startup (`rusty-chat`) and tests both call this.
pub fn install_drivers() {
    sqlx::any::install_default_drivers();
}
