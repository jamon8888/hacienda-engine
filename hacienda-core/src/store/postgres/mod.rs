//! Postgres store backends for Phase 9 durable persistence.
//!
//! This module is gated behind the `postgres` feature and only compiles on
//! native targets (not wasm32).

pub mod connection;

/// Disposable-Postgres test fixture (Phase 9 Task 1 Step 5). `cfg(test)` — the
/// `testcontainers`/`testcontainers-modules` dev-dependencies it needs are never part of
/// a published build.
#[cfg(test)]
pub(crate) mod test_support;

#[cfg(feature = "postgres")]
pub mod audit;

#[cfg(feature = "postgres")]
pub mod review;

#[cfg(all(feature = "postgres", feature = "jobs"))]
pub mod jobs;

#[cfg(feature = "postgres")]
pub mod versions;

#[cfg(feature = "postgres")]
pub mod presets;

#[cfg(feature = "postgres")]
pub mod api_keys;

#[cfg(feature = "postgres")]
pub mod usage;
