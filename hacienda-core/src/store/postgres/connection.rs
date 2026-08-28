//! Postgres connection and migration utilities.
//!
//! Provides a single `PgPool` constructor and embedded migrations.
//! Migrations run explicitly via `--migrate` flag — never implicitly in a library
//! constructor (see Design Decision D4 in the Phase 8-15 plan).

use sqlx::{postgres::PgPoolOptions, PgPool};
use std::time::Duration;

/// Error type for store connection/migration failures.
#[derive(Debug, thiserror::Error)]
/// StoreError enum
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
/// `set_ignore_missing(true)`: sqlx's `_sqlx_migrations` table is one per physical
/// database, not one per crate (see `hacienda-core/migrations/README.md`). Without
/// this, `Migrator::run` rejects the whole shared table as soon as it contains any
/// applied-migration row outside this crate's own resolved list (`VersionMissing`) —
/// which every `crates/hacienda-rag` migration row is, from this migrator's point of
/// view. `ignore_missing` relaxes that check to "every version *this* migrator knows
/// about must match its checksum", which is what the shared-database deployment
/// actually needs.
///
/// # Errors
/// Returns `StoreError::Migrate` if any migration fails.
pub async fn migrate(pool: &PgPool) -> Result<(), StoreError> {
    let mut migrator = sqlx::migrate!("./migrations");
    migrator.set_ignore_missing(true);
    migrator.run(pool).await.map_err(StoreError::Migrate)
}

/// Run migrations from an embedded source (for testcontainers where the
/// migrations directory isn't on the filesystem).
///
/// # Errors
/// Returns `StoreError::Migrate` if any migration fails.
pub async fn migrate_from_embedded(pool: &PgPool) -> Result<(), StoreError> {
    // sqlx::migrate! macro embeds the migrations at compile time.
    // This is the same as `migrate()` but makes the intent explicit for tests.
    // See `migrate()`'s doc comment for why `set_ignore_missing(true)` is required.
    let mut migrator = sqlx::migrate!("./migrations");
    migrator.set_ignore_missing(true);
    migrator.run(pool).await.map_err(StoreError::Migrate)
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
        let pool = connect(fixture.database_url())
            .await
            .expect("connect failed");
        migrate(&pool).await.expect("migrate failed");
        // If we get here, the schema is ready.
    }

    /// A fresh database must have the `default` tenant admitted (0004_tenants.sql's
    /// seed) — every other table's `tenant_id DEFAULT 'default'` backfill in
    /// migrations 0005-0008 depends on this row existing to satisfy its FK constraint.
    #[tokio::test]
    async fn migrating_a_fresh_database_seeds_the_default_tenant() {
        let fixture = PostgresFixture::start().await;
        let pool = connect(fixture.database_url())
            .await
            .expect("connect failed");
        migrate(&pool).await.expect("migrate failed");

        let id: String = sqlx::query_scalar("SELECT id FROM tenants WHERE id = 'default'")
            .fetch_one(&pool)
            .await
            .expect("the default tenant must be seeded by migration 0004");
        assert_eq!(id, "default");
    }

    /// Every tenant-scoped table added in S1 (migrations 0005-0008) must reject a
    /// `tenant_id` that names no admitted tenant — otherwise the database can persist
    /// an orphan tenant scope no `tenants` row ever admitted.
    #[tokio::test]
    async fn tenant_scoped_tables_reject_a_tenant_id_with_no_admitted_tenant() {
        let fixture = PostgresFixture::start().await;
        let pool = connect(fixture.database_url())
            .await
            .expect("connect failed");
        migrate(&pool).await.expect("migrate failed");

        let err = sqlx::query(
            "INSERT INTO jobs (id, status, tenant_id) VALUES ('test-job-id', 'queued', 'ghost-tenant')",
        )
        .execute(&pool)
        .await
        .expect_err("a job for an unadmitted tenant must be rejected by the FK constraint");
        assert!(
            err.as_database_error()
                .is_some_and(|e| e.is_foreign_key_violation()),
            "expected a foreign key violation, got {err:?}"
        );
    }
}
