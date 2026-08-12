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

use std::collections::HashMap;
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

/// One tenant's request count within the current one-minute window.
struct Window {
    started_at: Instant,
    count: u32,
}

/// In-memory, single-process, per-tenant request-rate limiter.
///
/// A fixed one-minute window per tenant, reset lazily on the first request to observe
/// it has expired — no background sweep, since a tenant with no traffic needs no state
/// maintained on its behalf.
pub struct InMemoryQuotaStore {
    limits: QuotaLimits,
    windows: Mutex<HashMap<String, Window>>,
}

impl InMemoryQuotaStore {
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
        const WINDOW: Duration = Duration::from_secs(60);
        let now = Instant::now();

        let mut windows = self
            .windows
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());

        let window = windows.entry(tenant.to_string()).or_insert_with(|| Window {
            started_at: now,
            count: 0,
        });

        if now.duration_since(window.started_at) >= WINDOW {
            window.started_at = now;
            window.count = 0;
        }

        if window.count >= self.limits.requests_per_minute {
            let retry_after = WINDOW.saturating_sub(now.duration_since(window.started_at));
            // Never advertise a zero-second wait: the window has not rolled over yet, or
            // this call would not be rejected.
            return Err(retry_after.max(Duration::from_secs(1)));
        }

        window.count += 1;
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
    let secs = retry_after.as_secs().max(1);
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
}
