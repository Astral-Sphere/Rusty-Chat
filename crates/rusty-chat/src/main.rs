use anyhow::Result;
use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(
    name = "rusty-chat",
    version,
    about = "Rusty-Chat server (open-webui compatible)"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Run the HTTP server
    Serve,
    /// Verify database compatibility without starting the server
    MigrateCheck,
    /// Create (or promote) the first admin account
    CreateAdmin {
        #[arg(long)]
        email: String,
        #[arg(long)]
        password: String,
        #[arg(long, default_value = "User")]
        name: String,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let cli = Cli::parse();
    match cli.command {
        Command::Serve => serve().await,
        Command::MigrateCheck => migrate_check().await,
        Command::CreateAdmin {
            email,
            password,
            name,
        } => create_admin(email, password, name).await,
    }
}

async fn serve() -> Result<()> {
    let (app_state, settings) = rusty_chat::open_state().await?;
    let router = rusty_chat::build_router(app_state, &settings.frontend_dist);

    let addr = format!("{}:{}", settings.host, settings.port);
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    tracing::info!(
        "rusty-chat v{} listening on http://{addr}",
        env!("CARGO_PKG_VERSION")
    );
    axum::serve(listener, router).await?;
    Ok(())
}

async fn migrate_check() -> Result<()> {
    rc_db::install_drivers();
    let settings = rusty_chat::settings::Settings::load()?;
    let mut conn = rusty_chat::raw_any_conn(&settings.database_url).await?;
    match rc_db::bootstrap::inspect_database(&mut conn).await? {
        rc_db::bootstrap::DatabaseState::Fresh => {
            println!(
                "FRESH: no schema — `serve` will create it at head {}",
                rc_db::COMPATIBLE_ALEMBIC_HEAD
            );
        }
        rc_db::bootstrap::DatabaseState::Compatible => {
            println!(
                "OK: database is at compatible head {}",
                rc_db::COMPATIBLE_ALEMBIC_HEAD
            );
        }
        rc_db::bootstrap::DatabaseState::WrongRevision { found } => {
            anyhow::bail!(
                "INCOMPATIBLE: alembic revision {found} != {} — migrate with open-webui first",
                rc_db::COMPATIBLE_ALEMBIC_HEAD
            );
        }
        rc_db::bootstrap::DatabaseState::LegacyUnstamped => {
            anyhow::bail!(
                "LEGACY: pre-0.6 schema without alembic_version — open once with open-webui >= 0.6"
            );
        }
    }
    Ok(())
}

async fn create_admin(email: String, password: String, name: String) -> Result<()> {
    let (app, _settings) = rusty_chat::open_state().await?;
    if rc_db::repo::users::get_num_users(&app.db).await? > 0 {
        anyhow::bail!("users already exist — create-admin only works on a fresh database");
    }
    let hashed = rc_auth::hash_password(&password, rc_auth::HashAlgorithm::Bcrypt)?;
    let user = rc_db::repo::auths::insert_new_auth(
        &app.db,
        rc_db::repo::auths::SignupParams {
            email: &email.to_lowercase(),
            password_hash: &hashed,
            name: &name,
            profile_image_url: None,
            role: Some("pending"),
            oauth: None,
        },
    )
    .await?;
    let Some(user) = user else {
        anyhow::bail!("user creation failed");
    };
    rc_db::repo::users::update_user_by_id(
        &app.db,
        &user.id,
        rc_db::repo::users::UserPatch {
            role: Some("admin".to_string()),
            ..Default::default()
        },
    )
    .await?;
    println!(
        "created admin {} ({})",
        user.email.unwrap_or_default(),
        user.id
    );
    Ok(())
}
