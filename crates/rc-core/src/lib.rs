//! # rc-core
//!
//! Shared types for Rusty-Chat: DTOs, the realtime event protocol, error
//! types, and the timestamp policy that keeps us byte-compatible with
//! open-webui 0.11.3 databases.
//!
//! See `docs/COMPATIBILITY.md` for the authoritative table-by-table
//! timestamp conventions.

pub mod error;
pub mod timestamp;

pub use error::Error;
pub type Result<T, E = Error> = std::result::Result<T, E>;

/// The open-webui schema version this build is compatible with.
///
/// This is the alembic head revision of open-webui 0.11.3. `rc-db` refuses to
/// boot against a database stamped with a different revision.
pub const COMPATIBLE_ALEMBIC_HEAD: &str = "d4c1a8e37b62";
