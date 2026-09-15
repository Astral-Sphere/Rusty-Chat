//! SeaORM entities — hand-written column-for-column against the open-webui
//! 0.11.3 schema (see docs/COMPATIBILITY.md). Nullable columns are `Option<T>`;
//! epoch timestamp columns are plain `i64` in entities and converted to
//! `rc_core::timestamp::{Secs, Nanos}` at repository boundaries.

pub mod api_key;
pub mod auth;
pub mod chat;
pub mod chat_message;
pub mod config;
pub mod folder;
pub mod shared_chat;
pub mod tag;
pub mod user;
