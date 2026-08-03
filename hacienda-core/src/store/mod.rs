//! Durable store backends beyond the in-memory/file defaults.
//!
//! Gated behind the `postgres` feature so wasm32 and non-Postgres consumers of
//! `hacienda-core` never pull in `sqlx` (see `hacienda-core/Cargo.toml`'s `postgres`
//! feature comment).

pub mod postgres;
