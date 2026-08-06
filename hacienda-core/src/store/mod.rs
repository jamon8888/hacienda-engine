//! Durable store backends beyond the in-memory/file defaults.
//!
//! `postgres` gates `sqlx`, `s3` gates `reqwest`/`rusty-s3` — each submodule is only
//! compiled when its own feature is on, so wasm32 and consumers who want neither never
//! pull either dependency graph in (see `hacienda-core/Cargo.toml`'s feature comments).
//! The crate-root `#[cfg]` on `pub mod store;` in `lib.rs` requires at least one of the
//! two before this module compiles at all.

#[cfg(feature = "postgres")]
pub mod postgres;

#[cfg(feature = "s3")]
pub mod object;
