//! Disposable-Postgres test fixture (Phase 9 Task 1 Step 5).
//!
//! Spins up a real, throwaway Postgres container via the local Docker daemon and runs
//! the embedded migrations against it, so integration tests for the `postgres` feature
//! no longer need a human to export `DATABASE_URL` before running them. The container
//! and its `PgPool` are returned together: the pool must not outlive the container (the
//! container stops on drop, which then refuses new connections), so callers keep both
//! alive for the duration of the test rather than discarding the `ContainerAsync` handle.
//!
//! `#[cfg(test)]`-only: this module — and the `testcontainers`/`testcontainers-modules`
//! dev-dependencies it needs — only exist for `cargo test`, never for a published build.

use super::connection;
use sqlx::PgPool;
use testcontainers::{runners::AsyncRunner, ContainerAsync, ImageExt};
use testcontainers_modules::postgres::Postgres as PostgresImage;

/// A running disposable Postgres container plus a migrated pool connected to it.
///
/// Keep this value alive for as long as the pool is used — dropping it stops the
/// container, and any pool connection acquired afterwards fails.
pub struct PostgresFixture {
    /// Held only to keep the container alive; the pool and URL are the actual test
    /// surface.
    _container: ContainerAsync<PostgresImage>,
    database_url: String,
    pool: PgPool,
}

impl PostgresFixture {
    /// Start a fresh disposable Postgres container and run migrations against it.
    ///
    /// # Panics
    /// Panics (via `expect`) on container startup, connection, or migration failure.
    /// A fixture that can silently continue on a broken container would make every test
    /// built on top of it fail with a misleading assertion instead of the real cause, so
    /// this fails loudly at setup time instead.
    pub async fn start() -> Self {
        // `testcontainers-modules`' default Postgres tag is `11-alpine` (pre-PG13), which
        // predates `gen_random_uuid()` becoming a core built-in — the migrations in
        // `hacienda-core/migrations/0001_init.sql` use it unqualified, with no `pgcrypto`
        // extension, matching PG13+. Pin `16-alpine` to track the same major version as
        // the `pgvector/pgvector:pg16` container used for local/CI Postgres testing
        // elsewhere in this repo.
        let container = PostgresImage::default()
            .with_tag("16-alpine")
            .start()
            .await
            .expect("failed to start disposable Postgres container");
        let host = container
            .get_host()
            .await
            .expect("failed to resolve container host");
        let port = container
            .get_host_port_ipv4(5432)
            .await
            .expect("failed to resolve mapped Postgres port");
        let database_url = format!("postgres://postgres:postgres@{host}:{port}/postgres");

        let pool = connection::connect(&database_url)
            .await
            .expect("failed to connect to disposable Postgres container");
        connection::migrate_from_embedded(&pool)
            .await
            .expect("failed to run migrations against disposable Postgres container");

        Self {
            _container: container,
            database_url,
            pool,
        }
    }

    /// The migrated pool. Cloning is cheap (`PgPool` is an `Arc` internally).
    pub fn pool(&self) -> PgPool {
        self.pool.clone()
    }

    /// The connection string for this fixture's container. Restart tests use this to
    /// open a second, independent pool against the same database — simulating a process
    /// restart requires a genuinely new connection, not a clone of the existing pool.
    pub fn database_url(&self) -> &str {
        &self.database_url
    }
}
