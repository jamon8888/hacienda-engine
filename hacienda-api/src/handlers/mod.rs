//! Request handlers.
//!
//! Handlers must not derive `Caller` themselves; they call [`extract_auth_context`] to
//! retrieve the `Arc<AuthContext>` the middleware installed, then pass it to
//! [`caller_from_arc`] to get a `Caller<'_>`. These two functions are the one place
//! where the extension-to-Caller mapping lives, and they carry a safety comment.

pub mod audit_review;
pub mod auth;
pub mod documents;
pub mod info;
pub mod jobs;
pub mod openapi;
pub mod pii;
pub mod presets;
pub mod rag;
pub mod uploads;
pub mod usage;
pub mod versions;

use axum::http::request::Parts;
use hacienda_core::auth::{AuthContext, AuthExtension, Caller};
use std::sync::Arc;

/// Extract the authenticated context from request extensions.
///
/// Returns `None` when auth is disabled (no extension installed).
///
/// # Safety invariant
///
/// This is sound **only because the auth middleware has already enforced access before
/// the handler runs.** The middleware either refused the request or inserted the
/// extension. Absent extension therefore means auth is disabled, and `Caller::Trusted`
/// is the correct interpretation: the process boundary is the trust boundary.
///
/// If the middleware is bypassed, this function cannot detect it. The test
/// `guarded_handler_always_observes_auth_extension` in `routes.rs` is the regression
/// guard: it asserts that a guarded route without a token is refused, so a bypass
/// would make that test fail before any handler runs with incorrect trust.
pub fn extract_auth_context(parts: &Parts) -> Option<Arc<AuthContext>> {
    parts
        .extensions
        .get::<AuthExtension>()
        // AuthExtension derefs to Arc<AuthContext>; clone is Arc refcount bump.
        .map(|ext| Arc::clone(&**ext))
}

/// Derive a `Caller` from an optional `Arc<AuthContext>`.
///
/// `None` → `Caller::Trusted` (auth disabled; see `extract_auth_context` safety note).
/// `Some` → `Caller::Principal` borrowing the `AuthContext` inside the Arc.
pub fn caller_from_arc<'a>(ctx: &'a Option<Arc<AuthContext>>) -> Caller<'a> {
    match ctx {
        Some(arc) => Caller::Principal(arc.as_ref()),
        None => Caller::Trusted,
    }
}
