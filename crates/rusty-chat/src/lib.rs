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
            "/ollama/{*path}",
            axum::routing::any(routes::models::ollama_proxy),
        )
        .route("/health", axum::routing::get(|| async { "OK" }))
        .with_state(app_state);

    if frontend_dist.join("index.html").exists() {
        router = router.fallback_service(
            tower_http::services::ServeDir::new(frontend_dist)
                .append_index_html_on_directories(true),
        );
    }
    router
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
