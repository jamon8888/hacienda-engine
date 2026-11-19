//! The unified rehydration contract: [`RevealTicket`] (issue #147, wayfinder dossier #122,
//! Option 1 — « ticket de divulgation scopé, sur un contrat unique »).
//!
//! Before this module there were two parallel, unrelated rehydration contracts:
//!
//! - **Contract A** — the facade/REST/CLI token reveal (`pii:reveal` over any token of the
//!   tenant, no document, no justification, no TTL, no one-shot).
//! - **Contract B** — the installation-profile client (`RehydrationRequest` /
//!   `RehydrationCapability` in `hacienda-client-core`, executed by `hacienda-backend`),
//!   which never returns plaintext but had no justification, TTL, or named principal in
//!   its audit entries either.
//!
//! [`RevealTicket`] is now the single object both surfaces converge on:
//!
//! - **Minting** a ticket requires `Capability::PiiReveal` **and** dossier membership
//!   (the [`DossierAcl`]); an in-process [`Caller::Trusted`](crate::auth::Caller) can
//!   never mint one — the agent/MCP surface is closed by construction (the MCP manifest
//!   exposes no reveal tool at all).
//! - **Resolving** a ticket (getting the plaintext) is reserved to the ticket's `subject`,
//!   once if `one_shot`, before `expires_at`, and only for tokens/documents inside the
//!   ticket's scope. A ticket can be **revoked** at any time.
//! - TODO(#147): the `Reveal` audit entry currently carries a `principal` that is never
//!   `None` on this human path (the resolver must be a [`Caller::Principal`]), but the
//!   enrichment with `ticket_id`, `document_id`, `token_id`, `justification` and
//!   `approvers` requires extending `AuditEntryInput`/`AuditEntry` (and the audit chain +
//!   storage schema with them) — deferred so the core contract lands first.
//!
//! # Decisions on the open questions of issue #147 / dossier #122 §4
//!
//! - **TTL** : default *and* maximum of [`MAX_TICKET_TTL_SECS`] (10 minutes — the maximum
//!   dossier #122 §4 Q6 proposes). A requester may ask for a *shorter* TTL
//!   (`ttl_secs`, clamped to `[MIN_TICKET_TTL_SECS, MAX_TICKET_TTL_SECS]`), never a longer
//!   one. 10 minutes bounds how long a captured reveal response stays valid.
//! - **One-shot** : usage unique is non-negotiable on the human path (dossier #122 cible:
//!   « une seule fois »), so the facade always mints `one_shot: true` tickets. The field
//!   stays on the contract so exports and future non-human executors can see it.
//! - **Double approval** : the "high sensitivity" threshold is the *category set*
//!   [`HIGH_SENSITIVITY_CATEGORIES`] (identity documents, payment instruments, health
//!   data, credentials), derived from the token header `[CATEGORY:key_id:body]` or from
//!   `field_scope`. When any in-scope category is high-sensitivity, `approvers` must hold
//!   at least two **distinct** principals, none of them the `subject` (separation of
//!   duties). TODO(#147): the mint path stores `approvers` but does **not** enforce this
//!   yet — `RevealTicketError::InsufficientApprovers` exists and is reserved for it.
//! - **Unknown token** : stays fail-closed — resolution surfaces the same coarse
//!   `PseudonymError` shape as today, which the HTTP layer collapses to one 400.
//! - **Non-reversible mode (#17)** : a ticket can only be minted when the tenant's redaction
//!   mode is `RedactionMode::Pseudonymize`. Mask/Hash/Remove/Custom — and any future
//!   short non-reversible pseudonym mode from #17, which will by definition not be
//!   `Pseudonymize` — can never produce a ticket.
//! - **Xberg document-local map** (dossier #131 §5.3): a `RehydrationMap` is bound to the
//!   document it was minted for. The enforcement point is the ticket's `document_ids`: an
//!   executor (backend rehydration, vault adapter) must check that the document being
//!   rehydrated is **in** the ticket's `document_ids` (and that the ticket's `dossier_id`
//!   matches the document's dossier). Cross-document use of a document-local map is
//!   refused there, because the ticket — not the raw token — is what authorizes the call.
//!   TODO(#147): wire `Backend::rehydrate`/`VaultRehydrationAdapter` to accept a ticket and
//!   enforce this check (today only the facade's `resolve_reveal_ticket_with_auth` checks
//!   `document_ids`); TODO(#147): where the map itself lives (facade store vs backend
//!   store) is still open, dossier #131 §5.3.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, PoisonError};

use async_trait::async_trait;
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::tenancy::TenantId;

/// Default ticket TTL, in seconds: 10 minutes.
pub const DEFAULT_TICKET_TTL_SECS: u64 = 600;
/// Maximum ticket TTL, in seconds: 10 minutes (dossier #122 §4 Q6 proposes 10 min as the
/// maximum expiration). A shorter TTL may be requested; a longer one is clamped down.
pub const MAX_TICKET_TTL_SECS: u64 = 600;
/// Minimum ticket TTL, in seconds. 1 s keeps tests fast without allowing a zero-lived
/// ticket (which would expire between mint and resolve under any clock jitter).
pub const MIN_TICKET_TTL_SECS: u64 = 1;
/// How long a *terminal* ticket (resolved / revoked / expired) is retained past its
/// `expires_at` before being purged from [`RevealTicketStore`], in seconds — a short
/// forensic window ("which ticket was resolved, and when"), never a durability
/// promise: a purge simply makes an old ticket `NotFound`, which is the same
/// client-facing answer it already produced while expiring.
pub const TICKET_RETENTION_SECS: i64 = 600;
/// Ceiling on how many tickets a single [`RevealTicketStore`] keeps in memory. Only
/// already-dead tickets are ever dropped to get back under this bound; an `Active`
/// ticket is never evicted, so a valid resolution always still succeeds.
pub const MAX_RETAINED_TICKETS: usize = 4096;

/// Category labels (the `CATEGORY` header of a `[CATEGORY:key_id:body]` token) that make a
/// reveal scope **high-sensitivity**, requiring at least two approvers (dossier #120's
/// double-approval rule, applied per dossier #122 Option 1). Identity documents, payment
/// instruments, health data, and credentials — the categories whose disclosure is
/// irreversible harm rather than inconvenience.
pub const HIGH_SENSITIVITY_CATEGORIES: &[&str] = &[
    "SSN",
    "NATIONALID",
    "TAXID",
    "PASSPORTNUMBER",
    "DRIVERSLICENSE",
    "GOVERNMENTID",
    "IBAN",
    "BANKACCOUNT",
    "ROUTINGNUMBER",
    "SWIFTBIC",
    "CREDITCARD",
    "PAYMENTCARD",
    "CARDEXPIRY",
    "CARDCVV",
    "CRYPTOWALLET",
    "MEDICALRECORDNUMBER",
    "HEALTHPLANNUMBER",
    "DIAGNOSIS",
    "MEDICATION",
    "DATEOFBIRTH",
    "PASSWORD",
    "APIKEY",
    "SECRETTOKEN",
    "JWTTOKEN",
];

/// Whether a token category label (e.g. `"SSN"`) is high-sensitivity.
pub fn is_high_sensitivity_category(label: &str) -> bool {
    HIGH_SENSITIVITY_CATEGORIES.contains(&label.to_ascii_uppercase().as_str())
}

/// Whether a request's scope touches at least one high-sensitivity category — the trigger
/// for the double-approval rule (dossier #120): either a listed token's category header,
/// or a `field_scope` category label.
pub fn is_high_sensitivity_scope(token_ids: &[String], field_scope: &[String]) -> bool {
    token_ids
        .iter()
        .filter_map(|t| token_header(t))
        .any(|(category, _)| is_high_sensitivity_category(category))
        || field_scope
            .iter()
            .any(|label| is_high_sensitivity_category(label))
}

/// Parse the `[CATEGORY:key_id:body]` header of a pseudonym token.
///
/// Returns `None` for anything not shaped like a token. Shape only — this never touches
/// key material, so a malformed token is distinguishable from an unreadable one here, but
/// both collapse to the same client-facing 400 at the transport layer (no error oracle).
pub fn token_header(token: &str) -> Option<(&str, &str)> {
    let inner = token.strip_prefix('[')?.strip_suffix(']')?;
    let mut fields = inner.splitn(3, ':');
    let (category, key_id, body) = (fields.next()?, fields.next()?, fields.next()?);
    if category.is_empty() || key_id.is_empty() || body.is_empty() || fields.next().is_some() {
        None
    } else {
        Some((category, key_id))
    }
}

/// Lifecycle of a [`RevealTicket`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RevealTicketStatus {
    /// Minted, not yet resolved, not revoked, not expired.
    Active,
    /// A one-shot ticket was resolved once. Resolving again fails.
    Resolved,
    /// Explicitly revoked (by the subject or an approver). Never resolves again.
    Revoked,
    /// Past `expires_at` without being resolved. Never resolves again.
    Expired,
}

/// The unified rehydration contract (issue #147): the single object authorizing a
/// disclosure, on every surface (REST, CLI viewer, Tauri viewer, vault, backend).
///
/// Carries no secret: it is safe to serialize to the API response and to the audit log.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RevealTicket {
    /// Server-allocated opaque id (UUID). Never derived from caller input.
    pub ticket_id: String,
    /// The principal the ticket was minted for — the only caller allowed to resolve it.
    pub subject: String,
    /// Tenant the ticket is scoped to (S1). Tickets never cross tenants.
    pub tenant: TenantId,
    /// The dossier the subject was checked against at mint time (the ACL gate).
    pub dossier_id: String,
    /// Documents in scope. Executors must refuse any document not in this list — this is
    /// what keeps a document-local Xberg `RehydrationMap` from being used cross-document.
    pub document_ids: Vec<String>,
    /// Explicit token ids in scope. May be empty when `field_scope` is used instead.
    pub token_ids: Vec<String>,
    /// Category labels in scope (e.g. `["EMAIL"]`) when the request named fields rather
    /// than individual tokens. A token whose category label is in this list is in scope.
    pub field_scope: Vec<String>,
    /// Free-text justification, required at mint time and written to the audit entry.
    pub justification: String,
    /// RFC 3339 mint time.
    pub requested_at: String,
    /// RFC 3339 expiration time (`requested_at + ttl`, clamped to the max TTL).
    pub expires_at: String,
    /// Usage-unique flag. The facade always mints `one_shot: true` (see module doc).
    pub one_shot: bool,
    /// Principals that approved the disclosure. High-sensitivity scopes require >= 2.
    pub approvers: Vec<String>,
    pub status: RevealTicketStatus,
}

impl RevealTicket {
    /// Whether the ticket can still be resolved at `now`.
    pub fn is_live_at(&self, now: DateTime<Utc>) -> bool {
        if self.status != RevealTicketStatus::Active {
            return false;
        }
        match DateTime::parse_from_rfc3339(&self.expires_at) {
            Ok(expires) => now < expires.with_timezone(&Utc),
            // An unparsable stored timestamp is fail-closed: the ticket is not live.
            Err(_) => false,
        }
    }

    /// Whether `document_id` is inside the ticket's document scope.
    pub fn covers_document(&self, document_id: &str) -> bool {
        self.document_ids.iter().any(|id| id == document_id)
    }

    /// Whether `token` is inside the ticket's scope: either listed verbatim in
    /// `token_ids`, or its category header is in `field_scope`.
    pub fn covers_token(&self, token: &str) -> bool {
        if self.token_ids.iter().any(|t| t == token) {
            return true;
        }
        match token_header(token) {
            Some((category, _)) => self.field_scope.iter().any(|c| c == category),
            None => false,
        }
    }
}

/// A request to mint a [`RevealTicket`]. This is what contract B's `RehydrationRequest`
/// becomes at the authorization boundary: the *demand*, before the ticket is granted.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RevealTicketRequest {
    /// Dossier the subject must belong to (ACL gate at mint time).
    pub dossier_id: String,
    /// Documents in scope — must be non-empty and contain no empty id.
    pub document_ids: Vec<String>,
    /// Explicit token ids in scope. Shape-checked at mint; unknown keys stay fail-closed
    /// (400) at resolve time, never at mint time, so minting is not a token oracle.
    #[serde(default)]
    pub token_ids: Vec<String>,
    /// Category labels in scope, as an alternative to `token_ids`. At least one of
    /// `token_ids` / `field_scope` must be non-empty.
    #[serde(default)]
    pub field_scope: Vec<String>,
    /// Required, non-blank justification. Written verbatim to the audit entry.
    pub justification: String,
    /// Approvers. High-sensitivity scopes require >= 2 distinct approvers != subject.
    #[serde(default)]
    pub approvers: Vec<String>,
    /// Optional TTL in seconds; clamped to `[MIN_TICKET_TTL_SECS, MAX_TICKET_TTL_SECS]`,
    /// defaulting to [`DEFAULT_TICKET_TTL_SECS`].
    #[serde(default)]
    pub ttl_secs: Option<u64>,
}

/// Failures of the reveal-ticket contract.
///
/// The variants are distinguishable to an **operator** (logs, tests, in-crate callers);
/// the HTTP layer deliberately collapses them to a single generic 400 so a client cannot
/// use error shape to probe tokens, tickets, or dossiers (no error oracle).
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum RevealTicketError {
    #[error("in-process callers cannot mint or resolve reveal tickets")]
    TrustedCaller,
    #[error("reveal ticket not found")]
    NotFound,
    #[error("caller is not the ticket holder or an approver")]
    NotHolder,
    #[error("reveal ticket is revoked")]
    Revoked,
    #[error("reveal ticket is expired")]
    Expired,
    #[error("reveal ticket is already consumed")]
    Consumed,
    #[error("token or document is outside the ticket scope")]
    ScopeViolation,
    #[error("subject is not a member of the dossier (or no dossier ACL is configured)")]
    AclDenied,
    #[error("redaction mode is not reversible: no reveal ticket can be minted")]
    NonReversibleMode,
    #[error("high-sensitivity scope requires at least two distinct approvers")]
    InsufficientApprovers,
    #[error("invalid reveal ticket request: {reason}")]
    InvalidRequest { reason: String },
    /// No document→dossier index is configured, so document ownership cannot be proven.
    /// Fail-closed: without the index a ticket cannot be minted (reveal is opt-in).
    #[error("no document-to-dossier index is configured: document ownership cannot be proven")]
    DocumentOwnershipUnknown,
    /// A requested document does not belong to the ticket's dossier.
    #[error("a requested document does not belong to the ticket dossier")]
    DocumentNotInDossier,
}

impl RevealTicketError {
    /// A malformed request, naming **which field** failed and how to correct it, without
    /// exposing any secret (never the token body, key id, or dossier payload). Operators,
    /// logs and tests see the reason; the transport layer still collapses every one of
    /// these to a single generic 400, so a client gains no oracle from it.
    pub(crate) fn invalid_request(reason: impl Into<String>) -> Self {
        Self::InvalidRequest {
            reason: reason.into(),
        }
    }
}

/// Dossier membership (the business ACL behind ticket minting: `pii:reveal` + belonging
/// to the dossier).
///
/// Implementations must **fail closed**: any internal error is reported as `false`
/// (not a member), never as `true` and never as a panic.
#[async_trait]
pub trait DossierAcl: Send + Sync {
    /// Whether `principal` is a member of `dossier_id` in `tenant`.
    async fn is_member(&self, tenant: &TenantId, dossier_id: &str, principal: &str) -> bool;
}

/// In-memory [`DossierAcl`] — the default for single-process deployments and tests.
#[derive(Default)]
pub struct InMemoryDossierAcl {
    members: Mutex<HashMap<(TenantId, String), HashSet<String>>>,
}

impl InMemoryDossierAcl {
    pub fn new() -> Self {
        Self::default()
    }

    /// Grant `principal` membership of `dossier_id` in `tenant`.
    pub fn grant(
        &self,
        tenant: &TenantId,
        dossier_id: impl Into<String>,
        principal: impl Into<String>,
    ) {
        self.members
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .entry((tenant.clone(), dossier_id.into()))
            .or_default()
            .insert(principal.into());
    }
}

#[async_trait]
impl DossierAcl for InMemoryDossierAcl {
    async fn is_member(&self, tenant: &TenantId, dossier_id: &str, principal: &str) -> bool {
        self.members
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(&(tenant.clone(), dossier_id.to_owned()))
            .is_some_and(|members| members.contains(principal))
    }
}

/// Authoritative mapping `document_id -> dossier_id` within a tenant.
///
/// This is what closes the ownership gap the [`DossierAcl`] cannot: membership proves the
/// *subject* belongs to the dossier, never that a caller-supplied `document_id` does. A
/// member of `case-1` could otherwise name a document of `case-2` and have it minted into
/// the ticket's scope.
///
/// Implementations must **fail closed**: an unknown document or any internal error is
/// reported as `false` (not in this dossier), never as `true` and never as a panic.
#[async_trait]
pub trait DocumentDossierIndex: Send + Sync {
    /// Whether `document_id` belongs to `dossier_id` in `tenant`.
    async fn belongs_to(&self, tenant: &TenantId, document_id: &str, dossier_id: &str) -> bool;
}

/// In-memory [`DocumentDossierIndex`] — the default for single-process deployments and
/// tests. An index that knows nothing answers `false` for every lookup, which is the
/// fail-closed direction (no ownership proof ⇒ no ticket).
#[derive(Default)]
pub struct InMemoryDocumentDossierIndex {
    owner: Mutex<HashMap<(TenantId, String), String>>,
}

impl InMemoryDocumentDossierIndex {
    pub fn new() -> Self {
        Self::default()
    }

    /// Record that `document_id` belongs to `dossier_id` in `tenant`.
    pub fn index(
        &self,
        tenant: &TenantId,
        document_id: impl Into<String>,
        dossier_id: impl Into<String>,
    ) {
        self.owner
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert((tenant.clone(), document_id.into()), dossier_id.into());
    }
}

#[async_trait]
impl DocumentDossierIndex for InMemoryDocumentDossierIndex {
    async fn belongs_to(&self, tenant: &TenantId, document_id: &str, dossier_id: &str) -> bool {
        self.owner
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(&(tenant.clone(), document_id.to_owned()))
            .is_some_and(|owner| owner == dossier_id)
    }
}

/// In-memory store of live [`RevealTicket`]s, keyed by `(tenant, ticket_id)` — tickets
/// never cross tenants.
///
/// The clock is injectable ([`Self::with_clock`]) so expiration is testable without
/// sleeping; production uses [`Self::new`] (the real clock).
///
/// Durability (surviving a restart) is deliberately out of scope here, same as key
/// rotation is today: a restart expires every outstanding ticket, which is the
/// fail-closed direction.
pub struct RevealTicketStore {
    tickets: Mutex<HashMap<(TenantId, String), RevealTicket>>,
    now: Arc<dyn Fn() -> DateTime<Utc> + Send + Sync>,
}

impl Default for RevealTicketStore {
    fn default() -> Self {
        Self::new()
    }
}

impl RevealTicketStore {
    pub fn new() -> Self {
        Self::with_clock(Arc::new(Utc::now))
    }

    /// A store reading time from `now` — for tests.
    pub fn with_clock(now: Arc<dyn Fn() -> DateTime<Utc> + Send + Sync>) -> Self {
        Self {
            tickets: Mutex::new(HashMap::new()),
            now,
        }
    }

    pub fn now(&self) -> DateTime<Utc> {
        (self.now)()
    }

    /// Drop tickets that can never be resolved again, so a long-running facade does not
    /// retain every ticket (and its scope payload) forever. Bounds the map's size, not its
    /// security posture.
    ///
    /// Never removes an `Active` ticket, so a valid resolution always still succeeds.
    pub(crate) fn purge_obsolete(&self) {
        let now = self.now();
        let mut tickets = self.tickets.lock().unwrap_or_else(PoisonError::into_inner);
        let retention = Duration::seconds(TICKET_RETENTION_SECS);
        tickets.retain(|_, ticket| match ticket.status {
            RevealTicketStatus::Active => true,
            // Terminal tickets are kept a short while past expiry, then dropped.
            _ => match DateTime::parse_from_rfc3339(&ticket.expires_at) {
                Ok(expires) => now < expires.with_timezone(&Utc) + retention,
                // An unparsable timestamp is fail-closed and never resolvable again.
                Err(_) => false,
            },
        });
        // Capacity bound: shed the oldest already-dead tickets first, never a live one.
        if tickets.len() > MAX_RETAINED_TICKETS {
            let mut dead: Vec<(DateTime<Utc>, (TenantId, String))> = tickets
                .iter()
                .filter(|(_, t)| t.status != RevealTicketStatus::Active)
                .filter_map(|(key, t)| {
                    DateTime::parse_from_rfc3339(&t.expires_at)
                        .ok()
                        .map(|expires| (expires.with_timezone(&Utc), key.clone()))
                })
                .collect();
            // Sort by expiry only: `TenantId` does not implement `Ord`, and the key's
            // tuple would otherwise require it. Ties are harmless for a bounded purge.
            dead.sort_by_key(|(expires, _)| *expires);
            for (_, key) in dead.into_iter().take(tickets.len() - MAX_RETAINED_TICKETS) {
                tickets.remove(&key);
            }
        }
    }

    pub fn insert(&self, ticket: RevealTicket) {
        self.purge_obsolete();
        self.tickets
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert((ticket.tenant.clone(), ticket.ticket_id.clone()), ticket);
    }

    pub fn get(&self, tenant: &TenantId, ticket_id: &str) -> Option<RevealTicket> {
        self.tickets
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(&(tenant.clone(), ticket_id.to_owned()))
            .cloned()
    }

    /// Transition `Active -> Revoked`. Idempotent on an already-revoked ticket; fails on a
    /// consumed (resolved) ticket.
    pub fn revoke(
        &self,
        tenant: &TenantId,
        ticket_id: &str,
    ) -> Result<RevealTicket, RevealTicketError> {
        let mut tickets = self.tickets.lock().unwrap_or_else(PoisonError::into_inner);
        let ticket = tickets
            .get_mut(&(tenant.clone(), ticket_id.to_owned()))
            .ok_or(RevealTicketError::NotFound)?;
        match ticket.status {
            RevealTicketStatus::Revoked => Ok(ticket.clone()),
            RevealTicketStatus::Resolved => Err(RevealTicketError::Consumed),
            RevealTicketStatus::Active | RevealTicketStatus::Expired => {
                ticket.status = RevealTicketStatus::Revoked;
                Ok(ticket.clone())
            }
        }
    }

    /// Transition `Active -> Resolved` for a one-shot ticket. Fails if the ticket is not
    /// live at `now` (revoked / expired / already consumed).
    pub fn consume(
        &self,
        tenant: &TenantId,
        ticket_id: &str,
        now: DateTime<Utc>,
    ) -> Result<(), RevealTicketError> {
        let mut tickets = self.tickets.lock().unwrap_or_else(PoisonError::into_inner);
        let ticket = tickets
            .get_mut(&(tenant.clone(), ticket_id.to_owned()))
            .ok_or(RevealTicketError::NotFound)?;
        if !ticket.is_live_at(now) {
            if ticket.status == RevealTicketStatus::Active {
                ticket.status = RevealTicketStatus::Expired;
            }
            return Err(match ticket.status {
                RevealTicketStatus::Revoked => RevealTicketError::Revoked,
                RevealTicketStatus::Resolved => RevealTicketError::Consumed,
                RevealTicketStatus::Expired => RevealTicketError::Expired,
                RevealTicketStatus::Active => RevealTicketError::Expired,
            });
        }
        ticket.status = RevealTicketStatus::Resolved;
        Ok(())
    }
}

/// Mint a ticket id. Kept here so every id is allocated the same way.
pub(crate) fn new_ticket_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

/// Compute `(requested_at, expires_at)` RFC 3339 timestamps for a `ttl_secs` request,
/// clamped to `[MIN_TICKET_TTL_SECS, MAX_TICKET_TTL_SECS]`.
pub(crate) fn ticket_window(ttl_secs: Option<u64>, now: DateTime<Utc>) -> (String, String) {
    let ttl = ttl_secs
        .unwrap_or(DEFAULT_TICKET_TTL_SECS)
        .clamp(MIN_TICKET_TTL_SECS, MAX_TICKET_TTL_SECS);
    let expires = now + Duration::seconds(ttl as i64);
    (now.to_rfc3339(), expires.to_rfc3339())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ticket(tenant: &str, id: &str) -> RevealTicket {
        let now = Utc::now();
        let (requested_at, expires_at) = ticket_window(None, now);
        RevealTicket {
            ticket_id: id.to_owned(),
            subject: "alice".to_owned(),
            tenant: TenantId::new(tenant),
            dossier_id: "case-1".to_owned(),
            document_ids: vec!["doc:v1".to_owned()],
            token_ids: vec!["[EMAIL:k1:AAAA]".to_owned()],
            field_scope: vec![],
            justification: "legal hold".to_owned(),
            requested_at,
            expires_at,
            one_shot: true,
            approvers: vec![],
            status: RevealTicketStatus::Active,
        }
    }

    #[test]
    fn token_header_parses_category_and_key_id() {
        assert_eq!(token_header("[EMAIL:k1:ABC]"), Some(("EMAIL", "k1")));
        assert_eq!(token_header("not-a-token"), None);
        assert_eq!(token_header("[EMAIL:k1:]"), None);
        assert_eq!(token_header("[:k1:ABC]"), None);
    }

    #[test]
    fn high_sensitivity_categories_are_case_insensitive() {
        assert!(is_high_sensitivity_category("SSN"));
        assert!(is_high_sensitivity_category("iban"));
        assert!(!is_high_sensitivity_category("EMAIL"));
    }

    #[test]
    fn scope_covers_listed_tokens_and_field_scope_categories() {
        let mut t = ticket("default", "t1");
        assert!(t.covers_token("[EMAIL:k1:AAAA]"));
        assert!(!t.covers_token("[EMAIL:k1:BBBB]"));
        t.field_scope = vec!["EMAIL".to_owned()];
        assert!(t.covers_token("[EMAIL:k1:BBBB]"));
        assert!(!t.covers_token("[SSN:k1:CCCC]"));
        assert!(t.covers_document("doc:v1"));
        assert!(!t.covers_document("other:v1"));
    }

    #[test]
    fn ttl_is_clamped_to_the_ten_minute_maximum() {
        let now = Utc::now();
        let (_, expires) = ticket_window(Some(60 * 60), now);
        assert_eq!(
            DateTime::parse_from_rfc3339(&expires)
                .unwrap()
                .with_timezone(&Utc),
            now + Duration::seconds(MAX_TICKET_TTL_SECS as i64)
        );
        let (_, expires) = ticket_window(Some(30), now);
        assert_eq!(
            DateTime::parse_from_rfc3339(&expires)
                .unwrap()
                .with_timezone(&Utc),
            now + Duration::seconds(30)
        );
    }

    #[test]
    fn consume_is_one_shot_and_expired_tickets_never_resolve() {
        let store = RevealTicketStore::new();
        let now = store.now();
        store.insert(ticket("default", "t1"));
        assert!(store.consume(&TenantId::new("default"), "t1", now).is_ok());
        assert_eq!(
            store.consume(&TenantId::new("default"), "t1", now),
            Err(RevealTicketError::Consumed)
        );
        // Tickets are tenant-scoped: another tenant sees NotFound, not the ticket.
        assert_eq!(
            store.consume(&TenantId::new("acme"), "t1", now),
            Err(RevealTicketError::NotFound)
        );
    }

    #[test]
    fn revoke_blocks_later_consumption() {
        let store = RevealTicketStore::new();
        let now = store.now();
        store.insert(ticket("default", "t2"));
        store.revoke(&TenantId::new("default"), "t2").unwrap();
        assert_eq!(
            store.consume(&TenantId::new("default"), "t2", now),
            Err(RevealTicketError::Revoked)
        );
    }

    #[tokio::test]
    async fn in_memory_acl_is_fail_closed_by_default() {
        let acl = InMemoryDossierAcl::new();
        let tenant = TenantId::new("default");
        assert!(!acl.is_member(&tenant, "case-1", "alice").await);
        acl.grant(&tenant, "case-1", "alice");
        assert!(acl.is_member(&tenant, "case-1", "alice").await);
        assert!(!acl.is_member(&tenant, "case-2", "alice").await);
    }

    #[tokio::test]
    async fn in_memory_document_index_is_fail_closed_by_default() {
        let index = InMemoryDocumentDossierIndex::new();
        let tenant = TenantId::new("default");
        // Unknown everywhere until indexed — never a yes from ignorance.
        assert!(!index.belongs_to(&tenant, "doc:v1", "case-1").await);
        index.index(&tenant, "doc:v1", "case-1");
        assert!(index.belongs_to(&tenant, "doc:v1", "case-1").await);
        // Indexed in case-1: another dossier (case-2) is refused, and so is another
        // tenant that happens to use the same document id.
        assert!(!index.belongs_to(&tenant, "doc:v1", "case-2").await);
        let other = TenantId::new("acme");
        assert!(!index.belongs_to(&other, "doc:v1", "case-1").await);
    }

    fn ticket_with_status(tenant: &str, id: &str, status: RevealTicketStatus) -> RevealTicket {
        let now = Utc::now();
        let mut t = ticket(tenant, id);
        let (_, expires_at) = ticket_window(Some(60), now);
        t.expires_at = expires_at;
        t.status = status;
        t
    }

    #[test]
    fn purge_drops_terminal_tickets_but_never_a_live_one() {
        let store = RevealTicketStore::new();
        store.insert(ticket_with_status(
            "default",
            "live",
            RevealTicketStatus::Active,
        ));
        store.insert(ticket_with_status(
            "default",
            "resolved",
            RevealTicketStatus::Resolved,
        ));
        store.insert(ticket_with_status(
            "default",
            "revoked",
            RevealTicketStatus::Revoked,
        ));
        // Within the retention window: terminal tickets are kept, none lost early.
        store.purge_obsolete();
        assert!(store.get(&TenantId::new("default"), "live").is_some());
        assert!(store.get(&TenantId::new("default"), "resolved").is_some());

        // Past the retention window: terminal tickets are gone, the live one stays.
        // A store-level clock lets us move time instead of sleeping.
        let now = Utc::now();
        let shifted = Arc::new(Mutex::new(now));
        let clock = shifted.clone();
        let store = RevealTicketStore::with_clock(Arc::new(move || *clock.lock().unwrap()));
        store.insert(ticket_with_status(
            "default",
            "live",
            RevealTicketStatus::Active,
        ));
        let mut stale = ticket_with_status("default", "resolved", RevealTicketStatus::Resolved);
        let (_, long_ago) = ticket_window(
            Some(60),
            now - Duration::seconds(TICKET_RETENTION_SECS + 120),
        );
        stale.expires_at = long_ago;
        store.insert(stale);
        store.purge_obsolete();
        assert!(store.get(&TenantId::new("default"), "live").is_some());
        assert!(
            store.get(&TenantId::new("default"), "resolved").is_none(),
            "a terminal ticket long past expiry must be purged"
        );
    }

    #[test]
    fn high_sensitivity_scope_recognises_tokens_and_field_scope() {
        assert!(is_high_sensitivity_scope(&["[EMAIL:k1:AAAA]".to_owned()], &[]) == false);
        assert!(is_high_sensitivity_scope(
            &["[SSN:k1:AAAA]".to_owned()],
            &[]
        ));
        assert!(is_high_sensitivity_scope(&[], &["DIAGNOSIS".to_owned()]));
        assert!(!is_high_sensitivity_scope(&[], &["EMAIL".to_owned()]));
    }
}
