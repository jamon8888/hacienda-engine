use thiserror::Error;

#[derive(Debug, Error)]
/// AuditError enum
pub enum AuditError {
    /// I/O error writing audit log
    #[error("writing audit log to {path}: {source}")]
    Io {
        /// Path
        path: String,
        /// Source error
        #[source]
        source: std::io::Error,
    },

    /// JSON serialization error
    #[error("serializing audit entry: {0}")]
    Json(#[from] serde_json::Error),

    /// Chain integrity error
    #[error("audit chain broken at entry {index}: expected hash '{expected}', found '{actual}'")]
    ChainIntegrity {
        /// Index
        index: u64,
        /// Expected hash
        expected: String,
        /// Actual hash
        actual: String,
    },

    /// Config mismatch
    #[error("audit entry was minted under config '{actual}' but this chain uses '{expected}'")]
    ConfigMismatch { 
        /// Expected config
        expected: String, 
        /// Actual config
        actual: String 
    },

    /// A segment's recorded `seal_hash` does not match the hash recomputed from its
    /// own fields. This fires when any field of the seal (including `entry_count` and
    /// `prev_seal_hash`) has been altered after sealing — see Design Decision D2.
    #[error(
        "segment '{segment_id}' seal hash is corrupt: expected '{expected}', found '{actual}'"
    )]
    SegmentIntegrity {
        /// Segment ID
        segment_id: String,
        /// Expected hash
        expected: String,
        /// Actual hash
        actual: String,
    },

    /// The `prev_seal_hash` of a segment does not match the `seal_hash` of the
    /// immediately preceding seal in the chain, meaning a segment was inserted,
    /// deleted, or reordered. This catches attacks that `SegmentIntegrity` cannot
    /// catch on its own — an attacker who recomputes seal hashes after deletion would
    /// still break the link to the now-absent predecessor.
    #[error(
        "segment '{segment_id}' prev_seal_hash is '{actual}', expected '{expected}' \
         from the preceding seal"
    )]
    SegmentLink {
        /// Segment ID
        segment_id: String,
        /// Expected hash
        expected: String,
        /// Actual hash
        actual: String,
    },

    /// A sealed segment holds a different number of entries than its seal recorded.
    ///
    /// Distinct from [`AuditError::ChainIntegrity`] so the operator is told what is
    /// actually wrong. A hash mismatch and "three records are missing" call for very
    /// different responses, and collapsing them into one error would hide that.
    #[error("segment '{segment_id}' holds {actual} entries but its seal records {expected}")]
    SegmentEntryCount {
        /// Segment ID
        segment_id: String,
        /// Expected count
        expected: u64,
        /// Actual count
        actual: u64,
    },

    /// An operation that needs an open segment was called on a closed store.
    ///
    /// Deliberately *not* a [`AuditError::ChainIntegrity`]: nothing is corrupt, the caller
    /// simply used the store after shutting it down. Reporting a closed store as a broken
    /// chain would raise a tamper alarm for what is only a lifecycle mistake.
    #[error("the audit store is closed and cannot accept '{operation}'")]
    StoreClosed { 
        /// Operation name
        operation: &'static str 
    },

    /// A pagination cursor could not be resolved to a position in this history.
    ///
    /// Covers both a cursor that does not parse and one that parses but names a segment
    /// or an index this store does not hold. Both are reported rather than quietly
    /// restarting from the beginning of the history: a caller that trusted the cursor
    /// would then record every entry it already had a second time, and could not tell the
    /// duplicate run from new activity.
    #[error("audit cursor '{cursor}' is not a position in this history: {reason}")]
    UnresolvableCursor { 
        /// Cursor value
        cursor: String, 
        /// Reason
        reason: String 
    },

    /// A non-file persistence backend failed. `Io` is file-specific (it carries a
    /// `std::io::Error`); this is the equivalent for backends like
    /// [`IndexedDbAuditStore`](crate::audit::IndexedDbAuditStore) (Track L5) whose
    /// underlying errors are JS/DOM exceptions, not `std::io::Error`.
    #[error("audit backend error: {0}")]
    Backend(String),

    /// An invariant this store maintains internally did not hold — a bug in this
    /// crate, not a caller mistake or an environment problem. Every tenant-scoped
    /// method calls `ensure_tenant_loaded` before looking its tenant up in the
    /// in-memory map; nothing removes an entry once inserted, so the lookup
    /// immediately after should never fail. Reported rather than asserted with
    /// `expect` — a panic in a library takes the whole process down with it, and an
    /// audit write is exactly the operation that must not do that.
    ///
    /// Structured (rather than a bare `String`) so every call site is forced to name
    /// the operation and tenant it was serving, matching this enum's other variants —
    /// an operator debugging "audit store invariant violated" alone has no way to tell
    /// which tenant's data is affected or which store method to look at first.
    #[error("audit store invariant violated during '{operation}' for tenant '{tenant}': {cause}")]
    Internal {
        /// The [`AuditStore`](crate::audit::AuditStore) method that discovered the
        /// broken invariant, e.g. `"append"` or `"history"`.
        operation: &'static str,
        /// The tenant whose state the invariant check was for.
        tenant: String,
        /// What was expected to hold and did not.
        cause: String,
    },
}

/// Lets Postgres backend code use `?` directly on `sqlx::Error` instead of a
/// `.map_err(...)` at every call site. Gated behind the `postgres` feature so this
/// crate's non-Postgres consumers never pull in `sqlx` just for this impl.
#[cfg(feature = "postgres")]
impl From<sqlx::Error> for AuditError {
    fn from(e: sqlx::Error) -> Self {
        AuditError::Backend(e.to_string())
    }
}
