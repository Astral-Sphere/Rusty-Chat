//! Shared error type for Rusty-Chat internals.

use thiserror::Error as ThisError;

#[derive(Debug, ThisError)]
pub enum Error {
    #[error("database incompatible: {0}")]
    DatabaseIncompatible(String),

    #[error("not found: {0}")]
    NotFound(String),

    #[error("unauthorized: {0}")]
    Unauthorized(String),

    #[error("forbidden: {0}")]
    Forbidden(String),

    #[error("bad request: {0}")]
    BadRequest(String),

    #[error("internal: {0}")]
    Internal(String),
}

#[cfg(feature = "sea-orm")]
impl From<sea_orm::DbErr> for Error {
    fn from(e: sea_orm::DbErr) -> Self {
        Error::Internal(e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // 覆盖矩阵：
    // ✅ Display 文案（错误串会进 API detail/日志，格式是契约）
    // ✅ From<sea_orm::DbErr> → Internal（feature 门控与实现一致）

    #[test]
    fn display_strings_are_stable() {
        assert_eq!(
            Error::DatabaseIncompatible("bad head".into()).to_string(),
            "database incompatible: bad head"
        );
        assert_eq!(Error::NotFound("x".into()).to_string(), "not found: x");
        assert_eq!(
            Error::Unauthorized("no token".into()).to_string(),
            "unauthorized: no token"
        );
        assert_eq!(Error::Forbidden("r".into()).to_string(), "forbidden: r");
        assert_eq!(Error::BadRequest("p".into()).to_string(), "bad request: p");
        assert_eq!(Error::Internal("boom".into()).to_string(), "internal: boom");
    }

    #[cfg(feature = "sea-orm")]
    #[test]
    fn db_err_maps_to_internal() {
        let err: Error = sea_orm::DbErr::RecordNotFound("row".into()).into();
        assert!(matches!(err, Error::Internal(_)));
        assert!(err.to_string().contains("row"));
    }
}
