//! Disposable-Postgres test fixture (Phase 9 Task 1 Step 5).
//!
//! Spins up a real, throwaway Postgres container via the local Docker daemon and runs
//! the embedded migrations against it, so integration tests for the `postgres` feature
//! no longer need a human to export `DATABASE_URL` before running them. The container
//! and its `PgPool` are returned together: the pool must not outlive the container (the
//! container stops on drop, which then refuses new connections), so callers keep both
//! alive for the duration of the test rather than discarding the container handle.
//!
//! `#[cfg(test)]`-only: this module requires a local Docker CLI and daemon for
//! `cargo test`, never for a published build.

use super::connection;
use crate::tenancy::TenantId;
use sqlx::PgPool;
use std::{
    process::{Command, Stdio},
    sync::OnceLock,
    time::Duration,
};
use tokio::sync::OnceCell;

/// Owns the container from successful `docker run` until fixture setup fails or the
/// fixture is dropped. In both cases, cleanup runs even during a panic.
struct DockerContainer {
    id: String,
}

impl DockerContainer {
    fn start() -> Self {
        let container = Self {
            id: format!("hacienda-core-pg-{}", uuid::Uuid::new_v4()),
        };
        let output = Command::new("docker")
            .args([
                "run",
                "--detach",
                "--name",
                &container.id,
                "--publish",
                "127.0.0.1::5432",
                "--env",
                "POSTGRES_PASSWORD=postgres",
                "postgres:16-alpine",
            ])
            .output()
            .expect("failed to execute Docker CLI; install Docker and start its daemon");
        assert!(
            output.status.success(),
            "failed to start disposable Postgres 16 container {}: {}",
            container.id,
            String::from_utf8_lossy(&output.stderr).trim()
        );
        assert!(
            !output.stdout.is_empty(),
            "docker run returned no container ID for {}",
            container.id
        );
        // `SHARED` is static and Rust does not drop statics at process exit.
        // A detached child removes the container once this test process exits;
        // `Drop` still handles failures and short-lived fixtures immediately.
        let parent_pid = std::process::id().to_string();
        Command::new("sh")
            .args([
                "-c",
                "while kill -0 \"$1\" 2>/dev/null; do sleep 2; done; docker rm --force \"$2\" >/dev/null 2>&1",
                "hacienda-postgres-reaper",
                &parent_pid,
                &container.id,
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("failed to start Postgres test container cleanup process");
        container
    }

    fn command(&self, args: &[&str]) -> std::process::Output {
        Command::new("docker")
            .args(args)
            .arg(&self.id)
            .output()
            .expect("Docker CLI stopped working while preparing the Postgres fixture")
    }

    fn host_port(&self) -> u16 {
        let output = self.command(&["port"]);
        assert!(
            output.status.success(),
            "failed to read Postgres container port: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
        let ports = String::from_utf8_lossy(&output.stdout);
        let mapping = ports
            .lines()
            .find_map(|line| line.strip_prefix("5432/tcp -> 127.0.0.1:"))
            .unwrap_or_else(|| panic!("Docker did not publish Postgres on loopback: {ports}"));
        mapping.parse().expect("invalid mapped Postgres port")
    }

    fn logs(&self) -> String {
        let output = self.command(&["logs"]);
        format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    }

    async fn wait_ready(&self) {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(60);
        loop {
            let output = Command::new("docker")
                .args([
                    "exec",
                    &self.id,
                    "pg_isready",
                    "-h",
                    "127.0.0.1",
                    "-U",
                    "postgres",
                    "-d",
                    "postgres",
                ])
                .output()
                .expect("Docker CLI stopped working while waiting for Postgres");
            if output.status.success() {
                return;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "Postgres 16 container {} did not become ready within 60s; pg_isready: {}; logs:\n{}",
                self.id,
                String::from_utf8_lossy(&output.stderr).trim(),
                self.logs()
            );
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    }
}

impl Drop for DockerContainer {
    fn drop(&mut self) {
        match Command::new("docker")
            .args(["rm", "--force", &self.id])
            .output()
        {
            Ok(output) if output.status.success() => {}
            Ok(output) => eprintln!(
                "failed to remove Postgres test container {}: {}",
                self.id,
                String::from_utf8_lossy(&output.stderr).trim()
            ),
            Err(error) => eprintln!(
                "failed to remove Postgres test container {}: {error}",
                self.id
            ),
        }
    }
}

/// A running disposable Postgres container plus a migrated pool connected to it.
///
/// Keep this value alive for as long as the pool is used — dropping it stops the
/// container, and any pool connection acquired afterwards fails.
pub struct PostgresFixture {
    database_url: String,
    pool: PgPool,
    /// Dropped after the pool; the pool and URL are the actual test surface.
    _container: DockerContainer,
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
        // Match the Postgres 16 major used by the local/CI pgvector service.
        let container = DockerContainer::start();
        let port = container.host_port();
        container.wait_ready().await;
        let database_url = format!("postgres://postgres:postgres@127.0.0.1:{port}/postgres");

        let pool = connection::connect(&database_url)
            .await
            .expect("failed to connect to disposable Postgres container");
        connection::migrate_from_embedded(&pool)
            .await
            .expect("failed to run migrations against disposable Postgres container");

        Self {
            database_url,
            pool,
            _container: container,
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
    sqlx::query(
        "INSERT INTO tenants (id, display_name) VALUES ($1, NULL) ON CONFLICT (id) DO NOTHING",
    )
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
