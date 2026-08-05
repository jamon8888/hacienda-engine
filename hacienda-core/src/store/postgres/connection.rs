//! Postgres connection and migration utilities.
//!
//! Provides a single `PgPool` constructor and embedded migrations.
//! Migrations run explicitly via `--migrate` flag — never implicitly in a library
//! constructor (see Design Decision D4 in the Phase 8-15 plan).

use sqlx::{postgres::PgPoolOptions, PgPool};
use std::time::Duration;

/// Error type for store connection/migration failures.
#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("failed to connect to Postgres: {0}")]
    Connect(#[source] sqlx::Error),

    #[error("failed to run migrations: {0}")]
    Migrate(#[source] sqlx::migrate::MigrateError),

    #[error("migration source not found: {0}")]
    MigrationSource(String),
}

/// Connect to Postgres and return a pool.
///
/// Pool is configured with sensible defaults for hacienda workloads:
/// - max 10 connections (tunable via env if needed)
/// - 30s acquire timeout
/// - 5min idle timeout
/// - 30s max lifetime
///
/// # Errors
/// Returns `StoreError::Connect` if the pool cannot be established.
pub async fn connect(database_url: &str) -> Result<PgPool, StoreError> {
    let pool = PgPoolOptions::new()
        .max_connections(10)
        .acquire_timeout(Duration::from_secs(30))
        .idle_timeout(Duration::from_secs(5 * 60))
        .max_lifetime(Duration::from_secs(30 * 60))
        .connect(database_url)
        .await
        .map_err(StoreError::Connect)?;
    Ok(pool)
}

/// Run embedded migrations against the given pool.
///
/// Uses `sqlx::migrate!` with the `migrations` directory.
/// Call this explicitly from the process entry point (CLI `serve` or test harness)
/// behind a `--migrate` flag — never from a library constructor.
///
/// # Errors
/// Returns `StoreError::Migrate` if any migration fails.
pub async fn migrate(pool: &PgPool) -> Result<(), StoreError> {
    sqlx::migrate!("./migrations")
        .run(pool)
        .await
        .map_err(StoreError::Migrate)
}

/// Run migrations from an embedded source (for testcontainers where the
/// migrations directory isn't on the filesystem).
///
/// # Errors
/// Returns `StoreError::Migrate` if any migration fails.
pub async fn migrate_from_embedded(pool: &PgPool) -> Result<(), StoreError> {
    // sqlx::migrate! macro embeds the migrations at compile time.
    // This is the same as `migrate()` but makes the intent explicit for tests.
    sqlx::migrate!("./migrations")
        .run(pool)
        .await
        .map_err(StoreError::Migrate)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::postgres::test_support::PostgresFixture;

    /// No `#[ignore]`/`DATABASE_URL` needed (Phase 9 Task 1 Step 5): `PostgresFixture`
    /// spins up its own disposable container via the local Docker daemon, so this runs
    /// under a plain `cargo test -p hacienda-core --features postgres`.
    #[tokio::test]
    async fn connect_and_migrate() {
        let fixture = PostgresFixture::start().await;
        let pool = connect(fixture.database_url()).await.expect("connect failed");
        migrate(&pool).await.expect("migrate failed");
        // If we get here, the schema is ready.
    }
}
