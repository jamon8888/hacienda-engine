//! Built-in [`RagStore`](crate::RagStore) backends.
//!
//! Recovered from `77e2fd3d71^:crates/xberg-rag/src/backends/mod.rs`
//! (xberg, MIT).
//!
//! - [`memory`] — pure-Rust brute-force store (the reference backend; see
//!   the crate-level doc comment).
//! - [`pgvector`] — durable Postgres backend using the `pgvector` extension
//!   (Phase 12 Task 2, `superpowers/plans/2026-08-01-platform-parity-and-scale-implementation.md`).
//!   Gated behind the `postgres` feature and only compiles on native targets,
//!   mirroring `hacienda-core`'s `store::postgres` gating.

pub mod memory;

#[cfg(feature = "postgres")]
pub mod pgvector;
