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
use crate::tenancy::TenantId;
use sqlx::PgPool;
use std::sync::OnceLock;
use testcontainers::{runners::AsyncRunner, ContainerAsync, ImageExt};
use testcontainers_modules::postgres::Postgres as PostgresImage;
use tokio::sync::OnceCell;

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

static SHARED: OnceCell<PostgresFixture> = OnceCell::const_new();

/// A `PostgresFixture` shared by every caller within this test binary, started once on
/// first use and reused for the rest of the run.
///
/// `audit.rs`/`review.rs`/`jobs.rs`'s Postgres-backed test suites are written against a
/// single shared Postgres instance on purpose — mirroring the production single-writer
/// design where `AuditStore::entries`/`tip` read whatever segment is open store-wide —
/// not one throwaway container per test. Use this instead of calling
/// [`PostgresFixture::start`] directly so those tests keep sharing one database instead
/// of each racing to seed its own. Callers still need `--test-threads=1` for the reasons
/// documented on each of those test modules; this only removes the `DATABASE_URL`
/// requirement, it does not change that constraint.
pub async fn shared() -> &'static PostgresFixture {
    SHARED.get_or_init(PostgresFixture::start).await
}

/// Admits `tenant` into the `tenants` table if it isn't already there.
///
/// Migration `0004_tenants.sql` seeds only the `default` tenant; every other
/// tenant-scoped table gained a `fk_*_tenant` constraint in migrations 0005-0010
/// referencing `tenants(id)`, so any store test that fabricates a non-`default`
/// `TenantId` must admit it first or its very first insert against a scoped table fails
/// with a foreign-key violation. `ON CONFLICT DO NOTHING` makes this safe to call
/// repeatedly for the same fixed tenant literal a module's tests share across many
/// invocations against the same long-lived [`shared`] instance.
pub async fn ensure_tenant(pool: &PgPool, tenant: &TenantId) {
    sqlx::query("INSERT INTO tenants (id, display_name) VALUES ($1, NULL) ON CONFLICT (id) DO NOTHING")
        .bind(tenant.as_str())
        .execute(pool)
        .await
        .expect("failed to admit test tenant");
}

static RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();

/// Run `fut` on the single Tokio runtime that owns the [`shared`] fixture's pool for the
/// whole test binary's lifetime.
///
/// `#[tokio::test]` builds and tears down a brand-new runtime per test. sqlx ties each
/// connection's socket to the runtime that established it, so a connection opened while
/// servicing one `#[tokio::test]` becomes unusable the moment that test's runtime is
/// dropped — the next test, running under its own separate runtime, hands the pool a
/// connection whose return-to-pool bookkeeping never completes, permanently shrinking the
/// shared pool's usable capacity by one. Across `--test-threads=1` sequential tests this
/// compounds until the pool has nothing left to hand out and every later test times out on
/// `pool.acquire()` regardless of how simple its query is.
///
/// Every test that touches [`shared`] (or [`PostgresFixture::start`]'s pool) must drive its
/// async body through this function — with `#[test]` instead of `#[tokio::test]` — so the
/// pool is created and used by the same runtime for the test binary's entire run.
pub fn block_on_shared<F: std::future::Future>(fut: F) -> F::Output {
    let runtime = RUNTIME.get_or_init(|| {
        tokio::runtime::Runtime::new().expect("failed to build shared Postgres test runtime")
    });
    runtime.block_on(fut)
}
