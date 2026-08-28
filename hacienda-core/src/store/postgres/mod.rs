//! Postgres store backends for Phase 9 durable persistence.
//!
//! This module is gated behind the `postgres` feature and only compiles on
//! native targets (not wasm32).

/// Module connection
pub mod connection;

/// Disposable-Postgres test fixture (Phase 9 Task 1 Step 5). `cfg(test)` — the
/// `testcontainers`/`testcontainers-modules` dev-dependencies it needs are never part of
/// a published build.
#[cfg(test)]
pub(crate) mod test_support;

#[cfg(feature = "postgres")]
/// Module audit
pub mod audit;

#[cfg(feature = "postgres")]
/// Module review
pub mod review;

#[cfg(all(feature = "postgres", feature = "jobs"))]
/// Module jobs
pub mod jobs;

#[cfg(feature = "postgres")]
/// Module versions
pub mod versions;

#[cfg(feature = "postgres")]
/// Module presets
pub mod presets;

#[cfg(feature = "postgres")]
/// Module api_keys
pub mod api_keys;

#[cfg(feature = "postgres")]
/// Module usage
pub mod usage;

#[cfg(feature = "postgres")]
/// Module tenants
pub mod tenants;
