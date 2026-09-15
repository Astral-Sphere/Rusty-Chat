//! Repositories — data access aligned with open-webui's `models/*Table`
//! semantics. All async, all taking `&DatabaseConnection`.

pub mod auths;
pub mod chat_messages;
pub mod chats;
pub mod config;
pub mod shared_chats;
pub mod tags;
pub mod users;

/// Cross-dialect case-insensitive `LIKE` for search filters.
/// SQLite `LIKE` is ASCII-case-insensitive by default (open-webui additionally
/// registers a custom `like()` function achieving the same); Postgres needs
/// real `ILIKE`.
pub(crate) fn ci_like(
    col: impl sea_orm::sea_query::IntoColumnRef,
    pattern: &str,
    backend: sea_orm::DbBackend,
) -> sea_orm::sea_query::SimpleExpr {
    use sea_orm::sea_query::extension::postgres::PgExpr;
    use sea_orm::sea_query::{Expr, ExprTrait};
    match backend {
        sea_orm::DbBackend::Sqlite => Expr::col(col).like(format!("%{pattern}%")),
        _ => Expr::col(col).ilike(format!("%{pattern}%")),
    }
}

/// Condition excluding `meta.internal == true` rows (null-safe), matching
/// `Chat.meta['internal'].as_boolean().is_not(True)`.
pub(crate) fn not_internal(backend: sea_orm::DbBackend) -> sea_orm::sea_query::SimpleExpr {
    use sea_orm::sea_query::Expr;
    match backend {
        sea_orm::DbBackend::Sqlite => {
            // json_extract yields 1/0/NULL; NULL IS NOT 1 → include.
            Expr::cust("json_extract(meta, '$.internal') IS NOT 1")
        }
        _ => Expr::cust("(meta ->> 'internal')::boolean IS NOT TRUE"),
    }
}
