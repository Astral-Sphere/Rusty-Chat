//! Smoke tests for schema bootstrap and the alembic-version guard.
//!
//! These are the M0 gate for database compatibility: if we can bootstrap a
//! fresh database and then read/write it with plain SQL, and open-webui's own
//! fixture opens as `Compatible`, the foundation is sound.

use rc_core::Error;
use rc_db::bootstrap::{DatabaseState, Dialect, bootstrap, inspect_database};

/// Acquires a single connection from a pool over `url`.
/// Returns the pool too — the connection guard keeps it alive.
async fn any_conn(url: &str) -> (sqlx::AnyPool, sqlx::pool::PoolConnection<sqlx::Any>) {
    rc_db::install_drivers();
    let pool = sqlx::AnyPool::connect(url).await.unwrap();
    let conn = pool.acquire().await.unwrap();
    (pool, conn)
}

#[tokio::test]
async fn fresh_sqlite_bootstraps_to_compatible() {
    let dir = tempfile::tempdir().unwrap();
    let url = format!("sqlite://{}?mode=rwc", dir.path().join("test.db").display());
    let (_pool, mut conn) = any_conn(&url).await;

    assert_eq!(
        inspect_database(&mut conn).await.unwrap(),
        DatabaseState::Fresh
    );
    bootstrap(&mut conn, Dialect::Sqlite).await.unwrap();
    assert_eq!(
        inspect_database(&mut conn).await.unwrap(),
        DatabaseState::Compatible
    );
}

#[tokio::test]
async fn bootstrapped_schema_has_expected_tables() {
    let dir = tempfile::tempdir().unwrap();
    let url = format!("sqlite://{}?mode=rwc", dir.path().join("test.db").display());
    let (_pool, mut conn) = any_conn(&url).await;
    bootstrap(&mut conn, Dialect::Sqlite).await.unwrap();

    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%'",
    )
    .fetch_one(&mut *conn)
    .await
    .unwrap();
    // Ground truth from open-webui 0.11.3 at head: 43 tables including
    // legacy artifacts (document, chatidtag, config_old).
    assert_eq!(
        count, 43,
        "table count diverges from open-webui head schema"
    );

    // Core tables exist and are writable.
    let user_id = "test-user-0001";
    let now = rc_core::timestamp::Secs::now().as_i64();
    sqlx::query(
        "INSERT INTO user (id, email, name, role, created_at, updated_at, last_active_at) \
                 VALUES (?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(user_id)
    .bind("smoke@example.com")
    .bind("Smoke Tester")
    .bind("admin")
    .bind(now)
    .bind(now)
    .bind(now)
    .execute(&mut *conn)
    .await
    .unwrap();

    let (name, role): (String, String) = sqlx::query_as("SELECT name, role FROM user WHERE id = ?")
        .bind(user_id)
        .fetch_one(&mut *conn)
        .await
        .unwrap();
    assert_eq!((name.as_str(), role.as_str()), ("Smoke Tester", "admin"));
}

#[tokio::test]
async fn bootstrap_is_idempotent() {
    let dir = tempfile::tempdir().unwrap();
    let url = format!("sqlite://{}?mode=rwc", dir.path().join("test.db").display());
    let (_pool, mut conn) = any_conn(&url).await;
    bootstrap(&mut conn, Dialect::Sqlite).await.unwrap();
    // Second call sees Compatible and must not re-run DDL (which would fail
    // on existing tables).
    bootstrap(&mut conn, Dialect::Sqlite).await.unwrap();
    assert_eq!(
        inspect_database(&mut conn).await.unwrap(),
        DatabaseState::Compatible
    );
}

#[tokio::test]
async fn refuses_older_revision() {
    let dir = tempfile::tempdir().unwrap();
    let url = format!("sqlite://{}?mode=rwc", dir.path().join("test.db").display());
    let (_pool, mut conn) = any_conn(&url).await;
    bootstrap(&mut conn, Dialect::Sqlite).await.unwrap();
    sqlx::query("UPDATE alembic_version SET version_num = 'ca81bd47c050'")
        .execute(&mut *conn)
        .await
        .unwrap();

    match inspect_database(&mut conn).await.unwrap() {
        DatabaseState::WrongRevision { found } => assert_eq!(found, "ca81bd47c050"),
        other => panic!("expected WrongRevision, got {other:?}"),
    }
    let err = bootstrap(&mut conn, Dialect::Sqlite).await.unwrap_err();
    assert!(matches!(err, Error::DatabaseIncompatible(_)));
}

#[tokio::test]
async fn refuses_legacy_unstamped_database() {
    let dir = tempfile::tempdir().unwrap();
    let url = format!("sqlite://{}?mode=rwc", dir.path().join("test.db").display());
    let (_pool, mut conn) = any_conn(&url).await;
    sqlx::raw_sql(
        "CREATE TABLE user (id VARCHAR PRIMARY KEY, email VARCHAR); \
         CREATE TABLE auth (id VARCHAR PRIMARY KEY, password TEXT);",
    )
    .execute(&mut *conn)
    .await
    .unwrap();

    assert_eq!(
        inspect_database(&mut conn).await.unwrap(),
        DatabaseState::LegacyUnstamped
    );
    let err = bootstrap(&mut conn, Dialect::Sqlite).await.unwrap_err();
    assert!(matches!(err, Error::DatabaseIncompatible(_)));
}

#[tokio::test]
async fn opens_real_open_webui_fixture() {
    // Fixture: real database migrated to head by open-webui's own alembic
    // chain. Copy it (SQLite needs write access for WAL even on reads).
    let src = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../docs/fixtures/webui-head.db"
    );
    let dir = tempfile::tempdir().unwrap();
    let dst = dir.path().join("fixture.db");
    std::fs::copy(src, &dst).unwrap();

    let (_pool, mut conn) = any_conn(&format!("sqlite://{}", dst.display())).await;
    assert_eq!(
        inspect_database(&mut conn).await.unwrap(),
        DatabaseState::Compatible
    );

    // Round-trip one row through the schema open-webui itself created.
    let now = rc_core::timestamp::Secs::now().as_i64();
    sqlx::query(
        "INSERT INTO chat (id, user_id, title, chat, created_at, updated_at) \
                 VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind("chat-fixture-smoke")
    .bind("someone")
    .bind("Fixture Smoke")
    .bind(r#"{"history":{"messages":{},"currentId":null}}"#)
    .bind(now)
    .bind(now)
    .execute(&mut *conn)
    .await
    .unwrap();
    let title: String =
        sqlx::query_scalar("SELECT title FROM chat WHERE id = 'chat-fixture-smoke'")
            .fetch_one(&mut *conn)
            .await
            .unwrap();
    assert_eq!(title, "Fixture Smoke");
}

/// Postgres bootstrap requires a live server; enable by setting
/// `RC_TEST_PG_URL` (e.g. `postgres://postgres:fixture@localhost:5433/postgres`).
/// CI and developers with the podman fixture run this:
///   podman run -d --name rc-pg-fixture -e POSTGRES_PASSWORD=fixture -p 127.0.0.1:5433:5432 postgres:17-alpine
#[tokio::test]
async fn postgres_bootstrap_matches_ground_truth() {
    let Ok(url) = std::env::var("RC_TEST_PG_URL") else {
        eprintln!("skipping: RC_TEST_PG_URL not set");
        return;
    };
    // RC_TEST_PG_URL may point at any admin database; derive the server base.
    // `postgres://host:5433/postgres` -> `postgres://host:5433`
    let trimmed = url.trim_end_matches('/');
    let cut = trimmed.rfind('/').filter(|i| !trimmed[..*i].ends_with(':'));
    let base = match cut {
        Some(i) => &trimmed[..i],
        None => trimmed,
    };
    const TEST_DB: &str = "rc_db_bootstrap_test";

    // Recreate a pristine database (fixed name; this test is the only user).
    // WITH (FORCE) reaps lingering sessions from earlier aborted runs.
    let (pool, mut admin) = any_conn(&format!("{base}/postgres")).await;
    sqlx::raw_sql("DROP DATABASE IF EXISTS rc_db_bootstrap_test WITH (FORCE);")
        .execute(&mut *admin)
        .await
        .unwrap();
    sqlx::raw_sql("CREATE DATABASE rc_db_bootstrap_test;")
        .execute(&mut *admin)
        .await
        .unwrap();
    drop(admin);
    drop(pool);

    let (_pool2, mut conn) = any_conn(&format!("{base}/{TEST_DB}")).await;
    assert_eq!(
        inspect_database(&mut conn).await.unwrap(),
        DatabaseState::Fresh
    );
    bootstrap(&mut conn, Dialect::Postgres).await.unwrap();
    assert_eq!(
        inspect_database(&mut conn).await.unwrap(),
        DatabaseState::Compatible
    );

    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM information_schema.tables WHERE table_schema = 'public'",
    )
    .fetch_one(&mut *conn)
    .await
    .unwrap();
    assert_eq!(
        count, 43,
        "postgres table count diverges from open-webui head"
    );
}
