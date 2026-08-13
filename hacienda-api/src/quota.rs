//! Per-tenant request-rate quota (S1 spec §7) — middleware applied before body decode.
//!
//! The S1 spec names four quota dimensions: documents/month, bytes ingested,
//! requests/minute, and vector-corpus size. This implements the one dimension that is
//! purely enforceable at the transport layer, before any handler or store is reached —
//! the other three need counters threaded through document processing and RAG ingestion,
//! a deeper integration tracked separately. [`QuotaLimits`] is a struct rather than a bare
//! `u32` so those can be added as fields later without another signature change at every
//! call site.
//!
//! [`InMemoryQuotaStore`] is process-local and not shared across replicas — the same scope
//! every other in-memory store in this crate operates at until S2 wires a shared backend
//! (see that spec's `verify_passes_per_tenant_after_replica_failover` exit criterion, which
//! this does not meet).

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::{
    extract::{Request, State},
    http::HeaderValue,
    middleware::Next,
    response::{IntoResponse, Response},
};

use hacienda_core::auth::AuthExtension;
use hacienda_core::tenancy::TenantId;

use crate::error::ApiError;

/// Per-tenant limits enforced by [`quota_middleware`].
#[derive(Debug, Clone, Copy)]
pub struct QuotaLimits {
    /// Maximum requests a single tenant may make within any rolling one-minute window.
    pub requests_per_minute: u32,
}

/// One tenant's request timestamps within the trailing window, oldest first.
///
/// A `VecDeque` rather than a count: a true rolling window (matching
/// [`QuotaLimits::requests_per_minute`]'s own doc comment) has to know *when* each
/// request in the window happened, not just how many — a count-and-reset-at-a-fixed-
/// boundary scheme lets a tenant burst up to 2x the limit across a window edge (all of
/// its old window's allowance just before the reset, all of the new window's allowance
/// just after).
struct Window {
    timestamps: VecDeque<Instant>,
}

/// In-memory, single-process, per-tenant request-rate limiter.
///
/// A rolling one-minute window per tenant: every check prunes timestamps older than
/// the window before counting, so the limit holds over *any* trailing 60 seconds, not
/// just calendar-aligned ones — no background sweep, since a tenant with no traffic
/// needs no state maintained on its behalf, and a pruned-empty entry costs the same as
/// none.
pub struct InMemoryQuotaStore {
    limits: QuotaLimits,
    windows: Mutex<HashMap<String, Window>>,
}

/// Window over which [`QuotaLimits::requests_per_minute`] is enforced.
const WINDOW: Duration = Duration::from_secs(60);

/// Never advertise a shorter wait than this — even a request that would be accepted a
/// microsecond from now must not tell a client `Retry-After: 0`.
const MIN_RETRY_AFTER: Duration = Duration::from_secs(1);

impl InMemoryQuotaStore {
    /// Build a process-local limiter enforcing `limits` — see the type's own doc
    /// comment for what "process-local" implies (no cross-replica sharing until S2).
    pub fn new(limits: QuotaLimits) -> Self {
        Self {
            limits,
            windows: Mutex::new(HashMap::new()),
        }
    }

    /// Record one request for `tenant`. `Ok(())` if the tenant is within quota; otherwise
    /// `Err(retry_after)` naming how long the caller must wait before its next request
    /// would be accepted.
    fn check_and_record(&self, tenant: &str) -> Result<(), Duration> {
        let now = Instant::now();

        let mut windows = self
            .windows
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());

        let window = windows.entry(tenant.to_string()).or_insert_with(|| Window {
            timestamps: VecDeque::new(),
        });

        // Prune timestamps that have aged out of the trailing window before counting.
        while let Some(&oldest) = window.timestamps.front() {
            if now.duration_since(oldest) >= WINDOW {
                window.timestamps.pop_front();
            } else {
                break;
            }
        }

        if window.timestamps.len() as u32 >= self.limits.requests_per_minute {
            let retry_after = match window.timestamps.front() {
                Some(&oldest) => WINDOW.saturating_sub(now.duration_since(oldest)),
                // requests_per_minute == 0: nothing recorded to wait out, but the
                // tenant must never be let through either.
                None => WINDOW,
            };
            return Err(retry_after.max(MIN_RETRY_AFTER));
        }

        window.timestamps.push_back(now);
        Ok(())
    }
}

/// Axum middleware enforcing [`InMemoryQuotaStore`] per tenant.
///
/// Reads the tenant from the [`AuthExtension`] `auth_middleware` attaches earlier in the
/// chain, falling back to the default tenant when none is present (auth disabled, or the
/// route is public — both cases `auth_middleware` never attaches one). Must run *after*
/// `auth_middleware` (so the extension exists) and *before* `DefaultBodyLimit`/any
/// handler extractor (so a rejected request never pays the cost of reading its body) —
/// see the ordering comment on `routes::build_router`.
///
/// `State` is `Option<Arc<InMemoryQuotaStore>>` rather than a required `Arc` so an
/// embedder that never calls `ApiState::with_quota_store` gets exactly today's
/// behaviour: quotas are opt-in, matching every other optional store in [`crate::state`].
pub async fn quota_middleware(
    State(quota): State<Option<Arc<InMemoryQuotaStore>>>,
    request: Request,
    next: Next,
) -> Response {
    let Some(quota) = quota else {
        return next.run(request).await;
    };

    let tenant = request
        .extensions()
        .get::<AuthExtension>()
        .map(|ext| ext.tenant.to_string())
        .unwrap_or_else(|| TenantId::default_tenant().to_string());

    match quota.check_and_record(&tenant) {
        Ok(()) => next.run(request).await,
        Err(retry_after) => rate_limited_response(retry_after),
    }
}

/// Build the `429` response: the same error envelope every other handler uses, plus a
/// `Retry-After` header — the one piece of information a well-behaved client needs to
/// back off correctly, which the JSON body alone does not give it in a machine-readable
/// form.
fn rate_limited_response(retry_after: Duration) -> Response {
    let secs = retry_after.as_secs().max(MIN_RETRY_AFTER.as_secs());
    let mut response = ApiError::rate_limited(format!(
        "Request quota exceeded for this tenant. Retry after {secs} seconds."
    ))
    .into_response();
    if let Ok(value) = HeaderValue::from_str(&secs.to_string()) {
        response.headers_mut().insert("retry-after", value);
    }
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allows_requests_up_to_the_limit_then_rejects() {
        let store = InMemoryQuotaStore::new(QuotaLimits {
            requests_per_minute: 3,
        });
        assert!(store.check_and_record("acme").is_ok());
        assert!(store.check_and_record("acme").is_ok());
        assert!(store.check_and_record("acme").is_ok());
        assert!(
            store.check_and_record("acme").is_err(),
            "a fourth request within the same window must be rejected"
        );
    }

    #[test]
    fn tenants_have_independent_windows() {
        let store = InMemoryQuotaStore::new(QuotaLimits {
            requests_per_minute: 1,
        });
        assert!(store.check_and_record("acme").is_ok());
        assert!(
            store.check_and_record("acme").is_err(),
            "acme's second request in the same window must be rejected"
        );
        assert!(
            store.check_and_record("globex").is_ok(),
            "globex must have its own quota, unaffected by acme's usage"
        );
    }

    #[test]
    fn retry_after_is_never_reported_as_zero() {
        let store = InMemoryQuotaStore::new(QuotaLimits {
            requests_per_minute: 0,
        });
        let retry_after = store
            .check_and_record("acme")
            .expect_err("zero-quota tenant must be rejected immediately");
        assert!(retry_after >= Duration::from_secs(1));
    }

    /// A fixed window resetting at a calendar boundary lets a tenant burst up to 2x
    /// its limit across the reset (its full old-window allowance immediately followed
    /// by its full new-window allowance). A true rolling window must not allow this:
    /// once the limit is used, the *oldest* timestamp — not a fixed clock boundary —
    /// is what has to age out before another request is accepted.
    #[test]
    fn a_burst_at_a_fixed_window_boundary_does_not_double_the_effective_limit() {
        let store = InMemoryQuotaStore::new(QuotaLimits {
            requests_per_minute: 3,
        });

        // Exhaust the limit "just before" a would-be fixed-window boundary.
        assert!(store.check_and_record("acme").is_ok());
        assert!(store.check_and_record("acme").is_ok());
        assert!(store.check_and_record("acme").is_ok());
        assert!(
            store.check_and_record("acme").is_err(),
            "a fourth request must still be rejected"
        );

        // Directly exercise that the oldest timestamp, not a reset counter, is what
        // gates the next acceptance: manually age the oldest entry out of the window
        // and confirm exactly one more request is then accepted (not a fresh
        // allowance of three).
        {
            let mut windows = store.windows.lock().unwrap();
            let window = windows.get_mut("acme").unwrap();
            let oldest = window.timestamps.pop_front().unwrap();
            window.timestamps.push_back(oldest - WINDOW);
            // Restore insertion order (oldest-first) after the manual edit above.
            let mut ordered: Vec<_> = window.timestamps.drain(..).collect();
            ordered.sort();
            window.timestamps.extend(ordered);
        }

        assert!(
            store.check_and_record("acme").is_ok(),
            "exactly one slot must free up once its timestamp ages out"
        );
        assert!(
            store.check_and_record("acme").is_err(),
            "the freed slot must not reopen the full original allowance"
        );
    }
}
