//! The stored tenant record — distinct from [`super::TenantCtx`], which is a
//! per-request scoping value, not a row.

use super::TenantId;
use serde::{Deserialize, Serialize};

/// A registered tenant.
///
/// A [`TenantCtx`](super::TenantCtx) can name any [`TenantId`] a caller likes; a `Tenant`
/// row is what makes that id *real* — admitted, with a resolvable pseudonymization key
/// (spec §4) and, eventually, quotas (spec §7). Every store-level check that matters for
/// security uses `TenantCtx`/`TenantId` directly, not this type: `Tenant` exists for
/// administration (create/list a tenant), not for authorization.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
/// Tenant struct
pub struct Tenant {
    pub id: TenantId,
    /// Human-readable label. Not unique, not used for lookup — `id` is the only key.
    pub display_name: Option<String>,
    /// RFC 3339 timestamp of admission.
    pub created_at: String,
}
