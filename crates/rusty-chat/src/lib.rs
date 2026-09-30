//! rusty-chat server library — router assembly shared between the binary
//! and contract tests (which drive the app in-process via tower oneshot).

pub mod defaults;
pub mod extract;
pub mod routes;
pub mod settings;
pub mod state;

use anyhow::Result;
use settings::Settings;
use state::AppState;

/// Full HTTP router: API routes + SPA fallback (when the frontend build
/// exists). Separated from `serve` so tests can exercise it without ports.
#[cfg_attr(feature = "embed-frontend", allow(unused_variables))]
pub fn build_router(app_state: AppState, frontend_dist: &std::path::Path) -> axum::Router {
    let mut router = axum::Router::new()
        .route(
            "/api/config",
            axum::routing::get(routes::config::get_app_config),
        )
        .route(
            "/api/v1/auths/signin",
            axum::routing::post(routes::auths::signin),
        )
        .route(
            "/api/v1/auths/signup",
            axum::routing::post(routes::auths::signup),
        )
        .route(
            "/api/v1/auths/signout",
            axum::routing::post(routes::auths::signout),
        )
        .route(
            "/api/v1/auths/",
            axum::routing::get(routes::auths::session_user),
        )
        .route(
            "/api/v1/auths/update/password",
            axum::routing::post(routes::auths::update_password),
        )
        .route(
            "/api/v1/auths/api_key",
            axum::routing::get(routes::auths::get_api_key),
        )
        .route(
            "/api/v1/auths/api_key",
            axum::routing::post(routes::auths::create_api_key),
        )
        .route(
            "/api/v1/auths/api_key",
            axum::routing::delete(routes::auths::delete_api_key),
        )
        .route(
            "/api/models",
            axum::routing::get(routes::models::get_models),
        )
        .route(
            "/api/v1/models",
            axum::routing::get(routes::models::get_models),
        )
        .route(
            "/api/chat/completions",
            axum::routing::post(routes::chat::chat_completion),
        )
        .route("/ws", axum::routing::get(routes::chat::ws_handler))
        .route(
            "/api/v1/tasks/title/completions",
            axum::routing::post(routes::tasks::generate_title),
        )
        .route(
            "/api/v1/utils/highlight",
            axum::routing::post(routes::utils::highlight),
        )
        .route(
            "/api/v1/utils/highlight/languages",
            axum::routing::get(routes::utils::languages),
        )
        .route(
            "/api/v1/chats/new",
            axum::routing::post(routes::chats::create_new_chat),
        )
        .route(
            "/api/v1/chats/",
            axum::routing::get(routes::chats::list_chats),
        )
        .route(
            "/api/v1/chats/list",
            axum::routing::get(routes::chats::list_chats),
        )
        .route(
            "/api/v1/chats/search",
            axum::routing::get(routes::chats::search_chats),
        )
        .route(
            "/api/v1/chats/pinned",
            axum::routing::get(routes::chats::pinned_chats),
        )
        .route(
            "/api/v1/chats/archived",
            axum::routing::get(routes::chats::archived_chats),
        )
        .route(
            "/api/v1/chats/archive/all",
            axum::routing::post(routes::chats::archive_all),
        )
        .route(
            "/api/v1/chats/unarchive/all",
            axum::routing::post(routes::chats::unarchive_all),
        )
        .route(
            "/api/v1/chats/shared",
            axum::routing::get(routes::chats::shared_chats),
        )
        .route(
            "/api/v1/chats/share/{share_id}",
            axum::routing::get(routes::chats::get_shared_chat),
        )
        .route(
            "/api/v1/chats/{id}",
            axum::routing::get(routes::chats::get_chat),
        )
        .route(
            "/api/v1/chats/{id}",
            axum::routing::post(routes::chats::update_chat),
        )
        .route(
            "/api/v1/chats/{id}",
            axum::routing::delete(routes::chats::delete_chat),
        )
        .route(
            "/api/v1/chats/{id}/pin",
            axum::routing::post(routes::chats::pin_chat),
        )
        .route(
            "/api/v1/chats/{id}/archive",
            axum::routing::post(routes::chats::archive_chat),
        )
        .route(
            "/api/v1/chats/{id}/share",
            axum::routing::post(routes::chats::share_chat),
        )
        .route(
            "/api/v1/chats/{id}/share",
            axum::routing::delete(routes::chats::delete_share),
        )
        .route(
            "/api/v1/chats/{id}/tags",
            axum::routing::get(routes::chats::get_chat_tags),
        )
        .route(
            "/api/v1/chats/{id}/tags",
            axum::routing::post(routes::chats::update_chat_tags),
        )
        .route(
            "/api/v1/chats",
            axum::routing::delete(routes::chats::delete_all_chats),
        )
        .route(
            "/ollama/{*path}",
            axum::routing::any(routes::models::ollama_proxy),
        )
        .route("/health", axum::routing::get(|| async { "OK" }))
        .with_state(app_state);

    #[cfg(feature = "embed-frontend")]
    {
        router = router.fallback(static_handler);
    }
    #[cfg(not(feature = "embed-frontend"))]
    if frontend_dist.join("index.html").exists() {
        router = router.fallback_service(
            tower_http::services::ServeDir::new(frontend_dist)
                .append_index_html_on_directories(true),
        );
    }
    router
}

/// Embedded frontend assets (feature `embed-frontend`). The folder is
/// relative to this crate's manifest: `web/dist` at the repo root, produced
/// by `just web-build` (dx build --release + copy).
#[cfg(feature = "embed-frontend")]
#[derive(rust_embed::RustEmbed)]
#[folder = "../../web/dist"]
struct FrontendAssets;

/// Serves embedded frontend files with an SPA-style index fallback. API-ish
/// prefixes never fall back to index.html — unmatched API routes 404.
#[cfg(feature = "embed-frontend")]
async fn static_handler(uri: axum::http::Uri) -> axum::response::Response {
    use axum::http::{StatusCode, header};
    use axum::response::IntoResponse;

    let path = uri.path().trim_start_matches('/');
    if path.starts_with("api/")
        || path.starts_with("ollama/")
        || path.starts_with("openai/")
        || path == "ws"
    {
        return (StatusCode::NOT_FOUND, "not found").into_response();
    }
    let file_path = if path.is_empty() { "index.html" } else { path };
    let not_found = || (StatusCode::NOT_FOUND, "not found").into_response();
    let Some(file) = FrontendAssets::get(file_path).or_else(|| FrontendAssets::get("index.html"))
    else {
        return not_found();
    };
    let mime = mime_guess::from_path(file_path).first_or_octet_stream();
    ([(header::CONTENT_TYPE, mime.as_ref())], file.data).into_response()
}

/// Startup sequence shared by `serve` / `create-admin`: connect, guard the
/// schema, seed config defaults.
pub async fn open_state() -> Result<(AppState, Settings)> {
    rc_db::install_drivers();
    let settings = Settings::load()?;
    let db = sea_orm::Database::connect(&settings.database_url).await?;

    let mut conn = raw_any_conn(&settings.database_url).await?;
    match rc_db::bootstrap::inspect_database(&mut conn).await? {
        rc_db::bootstrap::DatabaseState::Fresh => {
            tracing::info!(
                "fresh database → creating schema at head {}",
                rc_db::COMPATIBLE_ALEMBIC_HEAD
            );
            let dialect = dialect_of(&settings.database_url);
            rc_db::bootstrap::bootstrap(&mut conn, dialect).await?;
        }
        rc_db::bootstrap::DatabaseState::Compatible => {}
        other => anyhow::bail!("database incompatible: {other:?} — see docs/COMPATIBILITY.md"),
    }
    drop(conn);

    let config = std::sync::Arc::new(rc_db::repo::config::ConfigEngine::new(db.clone()));
    let seeded = config.seed_defaults(&defaults::default_config()).await?;
    if seeded > 0 {
        tracing::info!("seeded {seeded} missing config keys");
    }

    let app = AppState {
        db,
        config,
        secret_key: settings.secret_key.clone(),
        webui_name: settings.webui_name.clone(),
        version: env!("CARGO_PKG_VERSION"),
        placeholder_hash: std::sync::Arc::new(rc_auth::placeholder_hash()),
        webui_auth: settings.webui_auth,
        hub: std::sync::Arc::new(rc_realtime::Hub::new()),
    };
    Ok((app, settings))
}

use sqlx::Pool;

pub async fn raw_any_conn(database_url: &str) -> Result<sqlx::pool::PoolConnection<sqlx::Any>> {
    let pool = Pool::connect(database_url).await?;
    Ok(pool.acquire().await?)
}

fn dialect_of(url: &str) -> rc_db::bootstrap::Dialect {
    if url.starts_with("postgres") {
        rc_db::bootstrap::Dialect::Postgres
    } else {
        rc_db::bootstrap::Dialect::Sqlite
    }
}
