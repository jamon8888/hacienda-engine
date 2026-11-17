//! Trust-boundary contracts for the two supported Hacienda installation profiles.
//!
//! The types themselves — `InstallationConfig`, `LocalFullConfig`, `TeamOnPremiseConfig`,
//! their topology enums, their validation, and the shared resolution entry points — live
//! in [`hacienda_installation`], which both this crate and `hacienda-core` depend on.
//! They are re-exported here so every existing `hacienda_client_core::…` path keeps
//! working and so the client's own seam (`provider` below) stays next to them.
//!
//! This crate adds the *network* half of the client contract: a provider gateway
//! transport that only ever carries token-only text. It does not implement encryption,
//! a vault, a gateway server, or `LanceDB`. Secret material is represented by references
//! to an external provider and is never a field in these contracts.

mod provider;

pub use hacienda_installation::*;
pub use provider::*;
