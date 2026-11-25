//! Loopback-only projection channel for the Claude Code Mod renderer.
//!
//! This is a *dedicated* store and route set, deliberately separate from the
//! Codex viewer (`codex_viewer.rs`). The Codex viewer is a browser page whose
//! session is minted with an HttpOnly cookie and whose originals are published
//! to one globally active tab. A Mod is a non-browser consumer: it has no cookie
//! jar, needs an out-of-band authenticated bootstrap, and must never receive
//! another principal's originals just because that principal opened a session.
//!
//! Security invariants, all enforced here:
//!
//! * Loopback only — locality is decided by the **actual TCP peer**
//!   (`ConnectInfo<SocketAddr>`), never by the client-supplied Host/Origin. The
//!   header check remains as a same-origin/CSRF guard and to pin the event
//!   origin, but it can never make a remote caller "local". `hacienda serve`
//!   installs `into_make_service_with_connect_info` and refuses a non-loopback
//!   bind while this surface is mounted, so an extractor failure fails closed.
//! * Every session is scoped to the authenticated tenant **and** principal; the
//!   gateway bearer is checked by the route table's `documents:process` guard and
//!   the `AuthContext` is read from request extensions.
//! * The caller-supplied correlation value is a non-secret hint from
//!   `ANTHROPIC_CUSTOM_HEADERS`, never an authenticator. It is validated for shape
//!   and must match on bootstrap, subscribe, publish and poll. Two live sessions
//!   with the same correlation but different scopes make activation fail closed.
//! * The projection target is keyed by `(tenant, principal, correlation)` with a
//!   generation counter. A caller can only ever activate its own scope, so it
//!   cannot evict another tenant's or principal's projection.
//! * Originals live only in bounded, in-memory queues (an SSE channel and a
//!   one-shot poll queue), each capped at 32 events with each original capped at
//!   4096 bytes. Full queues drop and audit `dropped_backpressure`. Revoke and
//!   expiry clear the queues, drop the sender and invalidate live streams so
//!   queued originals are discarded rather than drained.
//! * Audit records are content-free (hashed refs only), bounded, and cover the
//!   full capability lifecycle: issuance, activation, subscription, polling,
//!   project, revocation, expiry and every denial.

use std::{
    collections::{HashMap, VecDeque},
    convert::Infallible,
    net::SocketAddr,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};

use axum::{
    extract::{ConnectInfo, Query, State},
    http::{header, HeaderMap, HeaderValue},
    response::{
        sse::{Event, KeepAlive, Sse},
        IntoResponse, Response,
    },
    Json,
};
use futures::stream;
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;
use uuid::Uuid;

use crate::{
    error::ApiError,
    handlers::{codex_viewer::is_loopback_request, extract_auth_context},
    state::ApiState,
};

/// A projection session is shorter than a Claude Code working session: a stale
/// Mod must not be able to replay a mapping after this window.
pub const MOD_SESSION_TTL: Duration = Duration::from_secs(10 * 60);

/// Non-secret correlation header, set by `hacienda claude install` through
/// `ANTHROPIC_CUSTOM_HEADERS`. It is a routing *hint* only, never a credential.
pub const CORRELATION_HEADER: &str = "x-hacienda-projection-correlation";
/// Short-lived bootstrap capability, sent by the Mod adapter on its own loopback
/// requests. It is not the gateway bearer and is never forwarded upstream.
pub const CAPABILITY_HEADER: &str = "x-hacienda-projection-capability";

const SESSION_PREFIX: &str = "cmods_";
const CAPABILITY_PREFIX: &str = "cmcap_";
const MAX_TOKEN_LEN: usize = 256;
const MAX_AUDIT_ENTRIES: usize = 2048;
/// Both the SSE channel and the one-shot poll queue hold at most this many
/// events per session. This is the hard memory ceiling for queued originals.
const EVENT_CAPACITY: usize = 32;
/// An original larger than this is dropped before it enters any queue. The
/// client keeps its pseudonym, which is the fail-closed behaviour.
const MAX_ORIGINAL_BYTES: usize = 4096;
/// Hard ceiling on a single `/mod/poll` serialized response body.
const MAX_POLL_BYTES: usize = 256 * 1024;
/// Maximum (and default) number of events one `/mod/poll` call drains.
const MAX_POLL_EVENTS: usize = 32;
/// Bounded memory of recently closed sessions, so a poller can be told that
/// its session closed without keeping the session (and its PII) alive.
const MAX_RECENTLY_CLOSED: usize = 256;
const MAX_LIVE_SESSIONS: usize = 256;

/// One event on the local-only Mod projection channel.
///
/// `Mapping` mirrors the Mod contract exactly: `type`, `origin`, `sessionId`,
/// `token`, `original`. `capability` lets the renderer's `verifyEvent` reject a
/// frame that did not come from the bootstrapped session.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "type")]
pub enum ModProjectionEvent {
    #[serde(rename = "Mapping")]
    Mapping {
        origin: String,
        #[serde(rename = "sessionId")]
        session_id: String,
        token: String,
        original: String,
        capability: String,
    },
    #[serde(rename = "End")]
    End {
        origin: String,
        #[serde(rename = "sessionId")]
        session_id: String,
        capability: String,
    },
}

/// Body of the one-shot, capability-authenticated `/mod/poll` transport.
///
/// `expired` reports that the session's TTL elapsed; `closed` reports that the
/// session was revoked/expired and will never produce further events. Both are
/// `false` for a live session. Poll is fail-closed: an unknown session or a
/// wrong capability/correlation returns 401 with no events at all.
#[derive(Clone, Debug, Serialize)]
pub struct ModPollResponse {
    pub events: Vec<ModProjectionEvent>,
    pub expired: bool,
    pub closed: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct ModAuditRecord {
    pub at: String,
    pub action: String,
    pub tenant: String,
    pub principal: String,
    pub session_ref: String,
    pub token_ref: String,
    pub outcome: String,
}

struct ModSession {
    tenant: String,
    principal: String,
    correlation: String,
    /// Plaintext is retained only to stamp the short-lived channel frames; the
    /// digest is what authorization checks compare.
    capability: String,
    capability_digest: [u8; 32],
    origin: String,
    expires: Instant,
    /// Bounded server-side queue drained atomically by `/mod/poll`. Cleared on
    /// revoke and expiry so queued originals never outlive the session.
    pending: VecDeque<ModProjectionEvent>,
    /// Bounded SSE channel, present only while a subscriber is attached. The
    /// receiver lives with the SSE stream; the sender is dropped on revoke.
    tx: Option<mpsc::Sender<ModProjectionEvent>>,
    /// Flipped on revoke/expiry. A live SSE stream checks it before emitting any
    /// frame and terminates, dropping its receiver and any queued originals.
    invalidated: Arc<AtomicBool>,
    /// Activation epoch of this session. A re-activation rotates the invalidation
    /// flag and drops the old sender so a displaced subscriber cannot be reused.
    generation: u64,
}

/// Scope of one projection target. Activation and publishing use this key, not
/// a single global slot, so one principal cannot displace another's target.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct ProjectionKey {
    tenant: String,
    principal: String,
    correlation: String,
}

impl ProjectionKey {
    fn new(tenant: &str, principal: &str, correlation: &str) -> Self {
        Self {
            tenant: tenant.to_owned(),
            principal: principal.to_owned(),
            correlation: correlation.to_owned(),
        }
    }
}

#[derive(Clone, Debug)]
struct ActiveProjection {
    session_id: String,
    generation: u64,
}

#[derive(Default)]
struct ModInner {
    sessions: HashMap<String, ModSession>,
    /// One projection target per `(tenant, principal, correlation)` scope.
    active: HashMap<ProjectionKey, ActiveProjection>,
    /// Bounded, content-free lifecycle audit records (newest last).
    audit: Vec<ModAuditRecord>,
    /// Monotonic counter so a displaced subscriber cannot be silently reused.
    generation: u64,
    /// `(session id, expired)` tombstones, bounded, so a poller learns that its
    /// session closed without the store retaining the session's originals.
    recently_closed: VecDeque<(String, bool)>,
}

impl ModInner {
    /// Drop expired sessions and their queued originals. Every state-touching
    /// operation calls this, so expiry is enforced even if no publisher runs.
    fn prune(&mut self, now: Instant) {
        let expired: Vec<String> = self
            .sessions
            .iter()
            .filter(|(_, session)| session.expires <= now)
            .map(|(id, _)| id.clone())
            .collect();
        for id in expired {
            if let Some(mut session) = self.sessions.remove(&id) {
                close_session(&mut session);
                push_audit(
                    &mut self.audit,
                    "expiry",
                    &session.tenant,
                    &session.principal,
                    &digest_ref(SESSION_PREFIX, &id),
                    &digest_ref("cap", &session.capability),
                    "expired",
                );
                self.remember_closed(&id, true);
            }
        }
        self.active
            .retain(|_, active| self.sessions.contains_key(&active.session_id));
    }

    fn remember_closed(&mut self, session_id: &str, expired: bool) {
        self.recently_closed
            .push_back((session_id.to_owned(), expired));
        if self.recently_closed.len() > MAX_RECENTLY_CLOSED {
            self.recently_closed.pop_front();
        }
    }

    fn closed_state(&self, session_id: &str) -> Option<bool> {
        self.recently_closed
            .iter()
            .find(|(id, _)| id == session_id)
            .map(|(_, expired)| *expired)
    }
}

/// Release every resource a session owns: queued originals, SSE sender and the
/// flag that makes a live stream terminate rather than drain stale frames.
fn close_session(session: &mut ModSession) {
    session.invalidated.store(true, Ordering::Release);
    session.pending.clear();
    session.tx.take();
}

/// Process-local Mod projection sessions. `Clone` shares the same store, so
/// every router clone sees the same sessions.
#[derive(Clone)]
pub struct ModSessionStore {
    inner: Arc<Mutex<ModInner>>,
    ttl: Duration,
}

impl Default for ModSessionStore {
    fn default() -> Self {
        Self::new(MOD_SESSION_TTL)
    }
}

/// What the bootstrap route returns to the host adapter. The capability is a
/// secret in transit only; the store retains just its digest.
#[derive(Clone, Debug, Serialize)]
pub struct ModBootstrap {
    pub session_id: String,
    pub origin: String,
    pub expires_at: String,
    pub capability: String,
    pub correlation: String,
}

/// Receiver half handed to the SSE stream. Checking `invalidated` before and
/// after `recv` keeps a revoked session from emitting a queued original.
pub struct ModEventStream {
    rx: mpsc::Receiver<ModProjectionEvent>,
    invalidated: Arc<AtomicBool>,
}

impl ModEventStream {
    pub async fn recv(&mut self) -> Option<ModProjectionEvent> {
        if self.invalidated.load(Ordering::Acquire) {
            return None;
        }
        self.rx
            .recv()
            .await
            .filter(|_| !self.invalidated.load(Ordering::Acquire))
    }

    pub fn is_invalidated(&self) -> bool {
        self.invalidated.load(Ordering::Acquire)
    }
}

impl ModSessionStore {
    pub fn new(ttl: Duration) -> Self {
        Self {
            inner: Arc::new(Mutex::new(ModInner::default())),
            ttl,
        }
    }

    /// Mint a session bound to the authenticated gateway caller's scope. The
    /// id and capability are two UUIDs behind typed prefixes: neither carries
    /// tenant, path, or PII information and neither is derivable from input.
    pub fn create_session(
        &self,
        tenant: &str,
        principal: &str,
        correlation: &str,
        origin: &str,
    ) -> Result<ModBootstrap, ApiError> {
        let session_id = format!(
            "{SESSION_PREFIX}{}{}",
            Uuid::new_v4().simple(),
            Uuid::new_v4().simple()
        );
        let capability = format!(
            "{CAPABILITY_PREFIX}{}{}",
            Uuid::new_v4().simple(),
            Uuid::new_v4().simple()
        );
        let expires_at = (chrono::Utc::now()
            + chrono::Duration::from_std(self.ttl).unwrap_or_default())
        .to_rfc3339();
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let now = Instant::now();
        inner.prune(now);
        if inner.sessions.len() >= MAX_LIVE_SESSIONS {
            return Err(ApiError::rate_limited(
                "Too many active local projection sessions.",
            ));
        }
        inner.sessions.insert(
            session_id.clone(),
            ModSession {
                tenant: tenant.to_owned(),
                principal: principal.to_owned(),
                correlation: correlation.to_owned(),
                capability: capability.clone(),
                capability_digest: *blake3::hash(capability.as_bytes()).as_bytes(),
                origin: origin.to_owned(),
                expires: now + self.ttl,
                pending: VecDeque::with_capacity(EVENT_CAPACITY),
                tx: None,
                invalidated: Arc::new(AtomicBool::new(false)),
                generation: 0,
            },
        );
        push_audit(
            &mut inner.audit,
            "issue",
            tenant,
            principal,
            &digest_ref(SESSION_PREFIX, &session_id),
            &digest_ref("cap", &capability),
            "issued",
        );
        Ok(ModBootstrap {
            session_id,
            origin: origin.to_owned(),
            expires_at,
            capability,
            correlation: correlation.to_owned(),
        })
    }

    /// Bind subsequent mapping events to one already-valid session. Every
    /// scope must match; a second live session with the same correlation but a
    /// different scope makes activation fail closed rather than pick a target.
    /// Activation only ever writes the key for the authenticated scope, so it
    /// cannot evict a different tenant's or principal's projection.
    pub fn activate(
        &self,
        session_id: &str,
        capability: &str,
        tenant: &str,
        principal: &str,
        correlation: &str,
    ) -> Result<(), ApiError> {
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let now = Instant::now();
        inner.prune(now);
        let allowed = activation_allowed(
            &inner,
            session_id,
            capability,
            tenant,
            principal,
            correlation,
            now,
        );
        push_audit(
            &mut inner.audit,
            "activate",
            tenant,
            principal,
            &digest_ref(SESSION_PREFIX, session_id),
            &digest_ref("corr", correlation),
            if allowed { "activated" } else { "denied" },
        );
        if !allowed {
            return Err(ApiError::unauthenticated());
        }
        let generation = inner.generation.wrapping_add(1);
        inner.generation = generation;
        if let Some(session) = inner.sessions.get_mut(session_id) {
            if session.generation != generation {
                // A previous subscriber to this session must go away when the
                // session is (re)activated; only a fresh subscribe may receive
                // events for this epoch.
                session.invalidated.store(true, Ordering::Release);
                session.invalidated = Arc::new(AtomicBool::new(false));
                session.tx = None;
                session.generation = generation;
            }
        }
        inner.active.insert(
            ProjectionKey::new(tenant, principal, correlation),
            ActiveProjection {
                session_id: session_id.to_owned(),
                generation,
            },
        );
        Ok(())
    }

    /// Validate a presented session/capability/correlation triple and replace
    /// its event sender. A replayed stream cannot keep an old receiver alive.
    pub fn subscribe(
        &self,
        session_id: &str,
        capability: &str,
        correlation: &str,
    ) -> Result<ModEventStream, ApiError> {
        let valid_shape = is_session_token(session_id) && is_capability(capability);
        let (tx, rx) = mpsc::channel(EVENT_CAPACITY);
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let now = Instant::now();
        inner.prune(now);
        let valid = valid_shape
            && inner.sessions.get(session_id).is_some_and(|session| {
                session.expires > now
                    && !session.invalidated.load(Ordering::Acquire)
                    && session.capability_digest == *blake3::hash(capability.as_bytes()).as_bytes()
                    && session.correlation == correlation
            });
        let (tenant, principal) = inner
            .sessions
            .get(session_id)
            .map(|s| (s.tenant.clone(), s.principal.clone()))
            .unwrap_or_default();
        push_audit(
            &mut inner.audit,
            "subscribe",
            &tenant,
            &principal,
            &digest_ref(SESSION_PREFIX, session_id),
            &digest_ref("corr", correlation),
            if valid { "subscribed" } else { "denied" },
        );
        if !valid {
            drop(tx);
            drop(rx);
            return Err(ApiError::unauthenticated());
        }
        let invalidated = inner.sessions[session_id].invalidated.clone();
        inner
            .sessions
            .get_mut(session_id)
            .expect("checked above")
            .tx = Some(tx);
        Ok(ModEventStream { rx, invalidated })
    }

    /// Atomically drain up to `limit` (capped at 32) queued mappings for a
    /// validated session. The response body is capped at 256 KiB. This is the
    /// bounded one-shot transport for hosts whose `$.http.fetch` cannot consume
    /// a long-lived SSE stream.
    /// Check that this exact scope has a live projection target before callers
    /// resolve pseudonyms into original values for delivery.
    pub fn has_active_projection(&self, tenant: &str, principal: &str, correlation: &str) -> bool {
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        inner.prune(Instant::now());
        let key = ProjectionKey::new(tenant, principal, correlation);
        let Some(projection) = inner.active.get(&key) else {
            return false;
        };
        inner
            .sessions
            .get(&projection.session_id)
            .is_some_and(|session| {
                session.tenant == tenant
                    && session.principal == principal
                    && session.correlation == correlation
                    && session.generation == projection.generation
                    && !session.invalidated.load(Ordering::Acquire)
                    && session.expires > Instant::now()
            })
    }

    pub fn poll(
        &self,
        session_id: &str,
        capability: &str,
        correlation: &str,
        limit: usize,
    ) -> Result<ModPollResponse, ApiError> {
        if !is_session_token(session_id) || !is_capability(capability) {
            return Err(ApiError::unauthenticated());
        }
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let now = Instant::now();
        let digest = blake3::hash(capability.as_bytes());
        // Observe expiry before pruning so the caller gets an explicit `expired`
        // signal instead of an opaque 401. A wrong capability never sees it.
        let expired = inner.sessions.get(session_id).is_some_and(|session| {
            session.expires <= now
                && session.capability_digest == *digest.as_bytes()
                && session.correlation == correlation
        });
        inner.prune(now);

        if expired {
            push_audit(
                &mut inner.audit,
                "poll",
                "",
                "",
                &digest_ref(SESSION_PREFIX, session_id),
                &digest_ref("cap", capability),
                "expired",
            );
            return Ok(ModPollResponse {
                events: Vec::new(),
                expired: true,
                closed: true,
            });
        }

        // A revoked/expired session we still remember: report closed, never
        // replay any original. The session id is a 256-bit secret, so this does
        // not give an attacker a usable oracle.
        if let Some(was_expired) = inner.closed_state(session_id) {
            push_audit(
                &mut inner.audit,
                "poll",
                "",
                "",
                &digest_ref(SESSION_PREFIX, session_id),
                &digest_ref("cap", capability),
                "closed",
            );
            return Ok(ModPollResponse {
                events: Vec::new(),
                expired: was_expired,
                closed: true,
            });
        }

        let Some(session) = inner.sessions.get_mut(session_id) else {
            push_audit(
                &mut inner.audit,
                "poll",
                "",
                "",
                &digest_ref(SESSION_PREFIX, session_id),
                &digest_ref("cap", capability),
                "denied",
            );
            return Err(ApiError::unauthenticated());
        };
        if session.expires <= now
            || session.invalidated.load(Ordering::Acquire)
            || session.capability_digest != *digest.as_bytes()
            || session.correlation != correlation
        {
            let (tenant, principal) = (session.tenant.clone(), session.principal.clone());
            push_audit(
                &mut inner.audit,
                "poll",
                &tenant,
                &principal,
                &digest_ref(SESSION_PREFIX, session_id),
                &digest_ref("cap", capability),
                "denied",
            );
            return Err(ApiError::unauthenticated());
        }

        let count = limit.clamp(1, MAX_POLL_EVENTS);
        let mut events = Vec::with_capacity(count);
        let mut bytes = 0usize;
        while events.len() < count {
            let Some(front) = session.pending.front() else {
                break;
            };
            let encoded = serde_json::to_vec(front)
                .map(|v| v.len())
                .unwrap_or(usize::MAX);
            if bytes.saturating_add(encoded) > MAX_POLL_BYTES {
                break;
            }
            let event = session.pending.pop_front().expect("front checked");
            bytes += encoded;
            events.push(event);
        }
        let (tenant, principal) = (session.tenant.clone(), session.principal.clone());
        push_audit(
            &mut inner.audit,
            "poll",
            &tenant,
            &principal,
            &digest_ref(SESSION_PREFIX, session_id),
            &digest_ref("cap", capability),
            if events.is_empty() {
                "drained_empty"
            } else {
                "drained"
            },
        );
        Ok(ModPollResponse {
            events,
            expired: false,
            closed: false,
        })
    }

    /// Publish to the one active session for this exact scope, carrying a
    /// content-free audit record. Oversize originals and full queues drop the
    /// event (never grow without bound) and are audited. The original is only
    /// sent over the bounded subscriber channel and the bounded poll queue.
    pub fn publish(
        &self,
        tenant: &str,
        principal: &str,
        correlation: &str,
        token: &str,
        original: String,
    ) -> bool {
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let now = Instant::now();
        inner.prune(now);
        // Split the guard into disjoint field borrows: `audit` is mutated while
        // `sessions` is held mutably for the whole delivery path.
        let ModInner {
            sessions,
            active,
            audit,
            ..
        } = &mut *inner;
        let key = ProjectionKey::new(tenant, principal, correlation);
        let unknown_ref = digest_ref(SESSION_PREFIX, "");
        let token_ref = digest_ref("tok", token);
        let Some(projection) = active.get(&key).cloned() else {
            push_audit(
                audit,
                "project",
                tenant,
                principal,
                &unknown_ref,
                &token_ref,
                "denied_no_active",
            );
            return false;
        };
        let Some(session) = sessions.get_mut(&projection.session_id) else {
            active.remove(&key);
            push_audit(
                audit,
                "project",
                tenant,
                principal,
                &unknown_ref,
                &token_ref,
                "denied_no_session",
            );
            return false;
        };
        let session_ref = digest_ref(SESSION_PREFIX, &projection.session_id);
        if session.expires <= now
            || session.invalidated.load(Ordering::Acquire)
            || session.generation != projection.generation
        {
            push_audit(
                audit,
                "project",
                tenant,
                principal,
                &session_ref,
                &token_ref,
                "denied_expired",
            );
            return false;
        }
        if original.len() > MAX_ORIGINAL_BYTES {
            push_audit(
                audit,
                "project",
                tenant,
                principal,
                &session_ref,
                &token_ref,
                "dropped_oversize",
            );
            return false;
        }
        let event = ModProjectionEvent::Mapping {
            origin: session.origin.clone(),
            session_id: projection.session_id.clone(),
            token: token.to_owned(),
            original,
            capability: session.capability.clone(),
        };

        let mut sse_delivered = false;
        if let Some(tx) = session.tx.as_ref() {
            match tx.try_send(event.clone()) {
                Ok(()) => sse_delivered = true,
                Err(mpsc::error::TrySendError::Full(dropped)) => {
                    drop(dropped);
                    push_audit(
                        audit,
                        "project",
                        tenant,
                        principal,
                        &session_ref,
                        &token_ref,
                        "dropped_backpressure",
                    );
                }
                Err(mpsc::error::TrySendError::Closed(dropped)) => {
                    drop(dropped);
                    session.tx = None;
                    push_audit(
                        audit,
                        "project",
                        tenant,
                        principal,
                        &session_ref,
                        &token_ref,
                        "audience_closed",
                    );
                }
            }
        }

        let queued = if session.pending.len() < EVENT_CAPACITY {
            session.pending.push_back(event);
            true
        } else {
            push_audit(
                audit,
                "project",
                tenant,
                principal,
                &session_ref,
                &token_ref,
                "dropped_backpressure",
            );
            false
        };
        let outcome = if sse_delivered || queued {
            "delivered_without_persistence"
        } else {
            "audience_closed"
        };
        push_audit(
            audit,
            "project",
            tenant,
            principal,
            &session_ref,
            &token_ref,
            outcome,
        );
        sse_delivered || queued
    }

    /// Drop a session, its poll queue and its SSE sender. The caller must
    /// present the bootstrap capability; a different session cannot revoke its
    /// peer. Queued originals are discarded, not drained.
    pub fn revoke(&self, session_id: &str, capability: &str) -> Result<(), ApiError> {
        if !is_session_token(session_id) || !is_capability(capability) {
            return Err(ApiError::unauthenticated());
        }
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let digest = blake3::hash(capability.as_bytes());
        let valid = inner
            .sessions
            .get(session_id)
            .is_some_and(|session| session.capability_digest == *digest.as_bytes());
        let tenant = inner
            .sessions
            .get(session_id)
            .map(|s| s.tenant.clone())
            .unwrap_or_default();
        let principal = inner
            .sessions
            .get(session_id)
            .map(|s| s.principal.clone())
            .unwrap_or_default();
        push_audit(
            &mut inner.audit,
            "revoke",
            &tenant,
            &principal,
            &digest_ref(SESSION_PREFIX, session_id),
            &digest_ref("cap", capability),
            if valid { "revoked" } else { "denied" },
        );
        if !valid {
            return Err(ApiError::unauthenticated());
        }
        let mut session = inner.sessions.remove(session_id).expect("validated");
        close_session(&mut session);
        inner.remember_closed(session_id, false);
        inner
            .active
            .retain(|_, active| active.session_id != session_id);
        Ok(())
    }

    /// Record a content-free denial raised by a handler (loopback/auth failure)
    /// where no authenticated scope is available.
    pub fn audit_access_denied(&self, reason: &str) {
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        push_audit(
            &mut inner.audit,
            "access",
            "unauthenticated",
            "unauthenticated",
            "",
            &digest_ref("reason", reason),
            "denied",
        );
    }

    /// Content-free audit snapshot, for tests and operators. Never includes a
    /// plaintext value or a token string.
    pub fn audit_for_tenant(&self, tenant: &str) -> Vec<ModAuditRecord> {
        let inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        inner
            .audit
            .iter()
            .filter(|record| record.tenant == tenant)
            .cloned()
            .collect()
    }

    /// Every audit record, including handler-level denials with no tenant.
    pub fn audit_all(&self) -> Vec<ModAuditRecord> {
        let inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        inner.audit.clone()
    }
}

/// All checks an activation must pass. Kept separate so the caller can audit
/// the outcome (including denial) without holding overlapping borrows.
fn activation_allowed(
    inner: &ModInner,
    session_id: &str,
    capability: &str,
    tenant: &str,
    principal: &str,
    correlation: &str,
    now: Instant,
) -> bool {
    if !is_session_token(session_id) || !is_capability(capability) {
        return false;
    }
    let Some(session) = inner.sessions.get(session_id) else {
        return false;
    };
    if session.expires <= now
        || session.invalidated.load(Ordering::Acquire)
        || session.capability_digest != *blake3::hash(capability.as_bytes()).as_bytes()
        || session.tenant != tenant
        || session.principal != principal
        || session.correlation != correlation
    {
        return false;
    }
    // Fail closed on ambiguity: a different live principal using the same
    // correlation would make published mappings ambiguous, regardless of
    // whether that session is currently active.
    !inner.sessions.values().any(|other| {
        other.correlation == correlation && (other.tenant != tenant || other.principal != principal)
    })
}

fn push_audit(
    audit: &mut Vec<ModAuditRecord>,
    action: &str,
    tenant: &str,
    principal: &str,
    session_ref: &str,
    token_ref: &str,
    outcome: &str,
) {
    audit.push(ModAuditRecord {
        at: chrono::Utc::now().to_rfc3339(),
        action: action.to_owned(),
        tenant: tenant.to_owned(),
        principal: principal.to_owned(),
        session_ref: session_ref.to_owned(),
        token_ref: token_ref.to_owned(),
        outcome: outcome.to_owned(),
    });
    if audit.len() > MAX_AUDIT_ENTRIES {
        audit.remove(0);
    }
}

fn digest_ref(prefix: &str, value: &str) -> String {
    let digest = blake3::hash(value.as_bytes());
    format!(
        "{prefix}_{}",
        data_encoding::HEXLOWER.encode(&digest.as_bytes()[..8])
    )
}

fn is_session_token(token: &str) -> bool {
    token.len() == SESSION_PREFIX.len() + 64
        && token.starts_with(SESSION_PREFIX)
        && token[SESSION_PREFIX.len()..]
            .chars()
            .all(|c| c.is_ascii_hexdigit())
}

fn is_capability(value: &str) -> bool {
    value.len() == CAPABILITY_PREFIX.len() + 64
        && value.starts_with(CAPABILITY_PREFIX)
        && value[CAPABILITY_PREFIX.len()..]
            .chars()
            .all(|c| c.is_ascii_hexdigit())
}

/// Validate and return the non-secret correlation hint. Anything outside a
/// narrow, opaque-token charset is refused rather than normalised: two spellings
/// of "the same" process would make correlation ambiguous.
pub fn correlation(headers: &HeaderMap) -> Option<&str> {
    let value = headers.get(CORRELATION_HEADER)?.to_str().ok()?.trim();
    (!value.is_empty()
        && value.len() <= MAX_TOKEN_LEN
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_')))
    .then_some(value)
}

fn capability(headers: &HeaderMap) -> Option<&str> {
    let value = headers.get(CAPABILITY_HEADER)?.to_str().ok()?.trim();
    is_capability(value).then_some(value)
}

fn no_store_headers() -> HeaderMap {
    let mut map = HeaderMap::new();
    map.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("no-store, private"),
    );
    map.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    map.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("no-referrer"),
    );
    map
}

/// Exact loopback origin for this request. Derived from the validated Host
/// header so the Mod's `createProjectionSession` can pin one origin and reject
/// every event that does not name it. This is an *origin pin*, not proof of
/// locality: locality comes from the actual peer below.
fn request_origin(headers: &HeaderMap) -> Result<String, ApiError> {
    let host = headers
        .get(header::HOST)
        .and_then(|value| value.to_str().ok())
        .ok_or_else(ApiError::unauthenticated)?;
    let host = host.to_ascii_lowercase();
    let authority = if let Some((name, port)) = host.rsplit_once(':') {
        if name.starts_with('[') {
            host.clone()
        } else if name == "localhost" || name == "127.0.0.1" || name == "::1" {
            format!("{name}:{port}")
        } else {
            return Err(ApiError::unauthenticated());
        }
    } else {
        host.clone()
    };
    let name = authority
        .rsplit_once(':')
        .map(|(name, _)| name)
        .unwrap_or(&authority);
    let loopback = matches!(name, "localhost" | "127.0.0.1" | "[::1]");
    if !loopback {
        return Err(ApiError::unauthenticated());
    }
    Ok(format!("http://{authority}"))
}

/// Locality is decided by the real TCP peer. Host/Origin are client-controlled
/// and are checked only as a same-origin guard; they can never make a remote
/// caller local. `ConnectInfo` extraction fails closed if `hacienda serve` was
/// not built with `into_make_service_with_connect_info`.
fn loopback_request(headers: &HeaderMap, peer: SocketAddr) -> bool {
    peer.ip().is_loopback() && is_loopback_request(headers)
}

/// `POST /mod/session` — authenticated, loopback-only bootstrap.
#[utoipa::path(
    post,
    path = "/mod/session",
    tag = "claude-mod",
    operation_id = "claudeModSession",
    security(("bearerAuth" = [])),
    responses(
        (status = 200, description = "Ephemeral projection session and capability"),
        (status = 401, description = "Missing or invalid credentials/correlation"),
        (status = 403, description = "Request is not from a loopback peer")
    )
)]
pub async fn session(
    State(state): State<ApiState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    parts: axum::http::request::Parts,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    if !loopback_request(&headers, peer) {
        state.mod_sessions.audit_access_denied("non_loopback_peer");
        return Err(ApiError::invalid_request(
            "Mod projection requests must originate from a loopback peer.",
        ));
    }
    let auth = match extract_auth_context(&parts) {
        Some(auth) => auth,
        None => {
            state.mod_sessions.audit_access_denied("unauthenticated");
            return Err(ApiError::unauthenticated());
        }
    };
    let correlation = match correlation(&headers) {
        Some(value) => value,
        None => {
            state.mod_sessions.audit_access_denied("bad_correlation");
            return Err(ApiError::unauthenticated());
        }
    };
    let origin = request_origin(&headers)?;
    let bootstrap = state.mod_sessions.create_session(
        auth.tenant.as_str(),
        &auth.principal_id,
        correlation,
        &origin,
    )?;
    // The freshly bootstrapped, authenticated session is the projection target
    // *for its own scope*. Activation cannot displace another scope's target.
    state.mod_sessions.activate(
        &bootstrap.session_id,
        &bootstrap.capability,
        auth.tenant.as_str(),
        &auth.principal_id,
        correlation,
    )?;
    let mut response = Json(serde_json::json!({
        "sessionId": bootstrap.session_id,
        "origin": bootstrap.origin,
        "expiresAt": bootstrap.expires_at,
        "capability": bootstrap.capability,
        "correlation": bootstrap.correlation,
    }))
    .into_response();
    response.headers_mut().extend(no_store_headers());
    Ok(response)
}

#[derive(Debug, Deserialize)]
pub struct ModEventsQuery {
    /// Opaque session id returned by `POST /mod/session`.
    pub session: String,
}

#[derive(Debug, Deserialize)]
pub struct ModPollQuery {
    /// Opaque session id returned by `POST /mod/session`.
    pub session: String,
    /// Optional per-call cap; clamped to [1, 32].
    pub limit: Option<usize>,
}

/// `GET /mod/events` — authenticated local-only SSE projection stream.
#[utoipa::path(
    get,
    path = "/mod/events",
    tag = "claude-mod",
    operation_id = "claudeModEvents",
    security(("bearerAuth" = [])),
    responses(
        (status = 200, description = "Mod projection SSE stream", content_type = "text/event-stream"),
        (status = 401, description = "Missing, expired or cross-scope session"),
        (status = 403, description = "Request is not from a loopback peer")
    )
)]
pub async fn events(
    State(state): State<ApiState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Query(query): Query<ModEventsQuery>,
) -> Result<impl IntoResponse, ApiError> {
    if !loopback_request(&headers, peer) {
        state.mod_sessions.audit_access_denied("non_loopback_peer");
        return Err(ApiError::invalid_request(
            "Mod projection requests must originate from a loopback peer.",
        ));
    }
    let correlation = correlation(&headers).ok_or_else(ApiError::unauthenticated)?;
    let capability = capability(&headers).ok_or_else(ApiError::unauthenticated)?;
    let origin = request_origin(&headers)?;
    let receiver = state
        .mod_sessions
        .subscribe(&query.session, capability, correlation)?;
    let stream = stream::unfold(
        (
            receiver,
            query.session.clone(),
            capability.to_owned(),
            origin,
        ),
        |(mut receiver, session_id, capability, origin)| async move {
            receiver.recv().await.map(|event| {
                let event = match event {
                    ModProjectionEvent::Mapping {
                        token, original, ..
                    } => ModProjectionEvent::Mapping {
                        origin: origin.clone(),
                        session_id: session_id.clone(),
                        token,
                        original,
                        capability: capability.clone(),
                    },
                    ModProjectionEvent::End { .. } => ModProjectionEvent::End {
                        origin: origin.clone(),
                        session_id: session_id.clone(),
                        capability: capability.clone(),
                    },
                };
                let data = serde_json::to_string(&event).unwrap_or_else(|_| "{}".to_owned());
                (
                    Ok::<Event, Infallible>(Event::default().data(data)),
                    (receiver, session_id, capability, origin),
                )
            })
        },
    );
    Ok((
        no_store_headers(),
        Sse::new(stream).keep_alive(KeepAlive::default()),
    ))
}

/// `GET /mod/poll` — bounded, capability-authenticated one-shot drain for hosts
/// whose `$.http.fetch` resolves only once the full response is available (it
/// cannot consume the long-lived SSE stream). The response is capped at 32
/// events / 256 KiB and never redirected.
#[utoipa::path(
    get,
    path = "/mod/poll",
    tag = "claude-mod",
    operation_id = "claudeModPoll",
    security(("bearerAuth" = [])),
    responses(
        (status = 200, description = "Bounded drain of queued Mod projection events"),
        (status = 401, description = "Missing, expired or cross-scope session"),
        (status = 403, description = "Request is not from a loopback peer")
    )
)]
pub async fn poll(
    State(state): State<ApiState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Query(query): Query<ModPollQuery>,
) -> Result<Json<ModPollResponse>, ApiError> {
    if !loopback_request(&headers, peer) {
        state.mod_sessions.audit_access_denied("non_loopback_peer");
        return Err(ApiError::invalid_request(
            "Mod projection requests must originate from a loopback peer.",
        ));
    }
    let correlation = correlation(&headers).ok_or_else(ApiError::unauthenticated)?;
    let capability = capability(&headers).ok_or_else(ApiError::unauthenticated)?;
    let response = state.mod_sessions.poll(
        &query.session,
        capability,
        correlation,
        query.limit.unwrap_or(MAX_POLL_EVENTS),
    )?;
    Ok(Json(response))
}

/// `POST /mod/revoke` — drop the session and its queued originals.
#[utoipa::path(
    post,
    path = "/mod/revoke",
    tag = "claude-mod",
    operation_id = "claudeModRevoke",
    security(("bearerAuth" = [])),
    responses(
        (status = 200, description = "Session revoked"),
        (status = 401, description = "Missing, expired or cross-scope session"),
        (status = 403, description = "Request is not from a loopback peer")
    )
)]
pub async fn revoke(
    State(state): State<ApiState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Query(query): Query<ModEventsQuery>,
) -> Result<Json<serde_json::Value>, ApiError> {
    if !loopback_request(&headers, peer) {
        state.mod_sessions.audit_access_denied("non_loopback_peer");
        return Err(ApiError::invalid_request(
            "Mod projection requests must originate from a loopback peer.",
        ));
    }
    let capability = capability(&headers).ok_or_else(ApiError::unauthenticated)?;
    state.mod_sessions.revoke(&query.session, capability)?;
    Ok(Json(serde_json::json!({"revoked": true})))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn loopback_headers(
        host: &str,
        correlation: Option<&str>,
        capability: Option<&str>,
    ) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(header::HOST, host.parse().unwrap());
        headers.insert(header::ORIGIN, format!("http://{host}").parse().unwrap());
        if let Some(value) = correlation {
            headers.insert(CORRELATION_HEADER, value.parse().unwrap());
        }
        if let Some(value) = capability {
            headers.insert(CAPABILITY_HEADER, value.parse().unwrap());
        }
        headers
    }

    #[test]
    fn session_tokens_capabilities_and_correlation_are_shape_checked() {
        let store = ModSessionStore::new(Duration::from_millis(1));
        let bootstrap = store
            .create_session("acme", "alice", "corr-1", "http://127.0.0.1:8787")
            .expect("session created");
        assert!(is_session_token(&bootstrap.session_id));
        assert!(is_capability(&bootstrap.capability));
        let headers = loopback_headers(
            "127.0.0.1:8787",
            Some("corr-1"),
            Some(&bootstrap.capability),
        );
        assert_eq!(correlation(&headers), Some("corr-1"));
        std::thread::sleep(Duration::from_millis(5));
        assert!(store
            .subscribe(&bootstrap.session_id, &bootstrap.capability, "corr-1")
            .is_err());
    }

    #[test]
    fn live_session_count_is_bounded() {
        let store = ModSessionStore::new(MOD_SESSION_TTL);
        for index in 0..MAX_LIVE_SESSIONS {
            store
                .create_session(
                    "acme",
                    &format!("principal-{index}"),
                    "corr",
                    "http://127.0.0.1:8787",
                )
                .expect("session remains within cap");
        }
        assert!(store
            .create_session("acme", "overflow", "corr", "http://127.0.0.1:8787")
            .is_err());
    }

    #[test]
    fn publish_is_scoped_to_the_active_session() {
        let store = ModSessionStore::new(MOD_SESSION_TTL);
        let first = store
            .create_session("acme", "alice", "corr-1", "http://127.0.0.1:8787")
            .expect("first session created");
        let second = store
            .create_session("acme", "bob", "corr-2", "http://127.0.0.1:8787")
            .expect("second session created");
        store
            .activate(
                &second.session_id,
                &second.capability,
                "acme",
                "bob",
                "corr-2",
            )
            .unwrap();
        assert!(!store.has_active_projection("acme", "alice", "corr-1"));
        assert!(store.has_active_projection("acme", "bob", "corr-2"));
        let mut first_rx = store
            .subscribe(&first.session_id, &first.capability, "corr-1")
            .unwrap();
        let mut second_rx = store
            .subscribe(&second.session_id, &second.capability, "corr-2")
            .unwrap();
        let original = "alice@example.com".to_owned();
        assert!(store.publish("acme", "bob", "corr-2", "[EMAIL:k1:abc]", original.clone()));
        // Wrong principal and wrong correlation must not deliver a mapping to
        // the wrong Mod process.
        assert!(!store.publish(
            "acme",
            "alice",
            "corr-1",
            "[EMAIL:k1:abc]",
            original.clone(),
        ));
        assert!(matches!(
            second_rx.rx.try_recv(),
            Ok(ModProjectionEvent::Mapping { .. })
        ));
        assert!(matches!(
            first_rx.rx.try_recv(),
            Err(tokio::sync::mpsc::error::TryRecvError::Empty)
        ));
        let audit = store.audit_for_tenant("acme");
        assert!(!format!("{:?}", audit).contains("alice@example.com"));
    }

    #[test]
    fn ambiguous_correlation_fails_closed() {
        let store = ModSessionStore::new(MOD_SESSION_TTL);
        let first = store
            .create_session("acme", "alice", "corr", "http://127.0.0.1:8787")
            .expect("first session created");
        store
            .activate(
                &first.session_id,
                &first.capability,
                "acme",
                "alice",
                "corr",
            )
            .unwrap();
        // Only once a *second* live session exists with the same correlation
        // but a different scope does activation become ambiguous.
        let second = store
            .create_session("acme", "bob", "corr", "http://127.0.0.1:8787")
            .expect("second session created");
        assert!(store
            .activate(
                &second.session_id,
                &second.capability,
                "acme",
                "bob",
                "corr"
            )
            .is_err());
    }

    #[test]
    fn cross_principal_activation_cannot_evict_another_scope() {
        let store = ModSessionStore::new(MOD_SESSION_TTL);
        let alice = store
            .create_session("acme", "alice", "corr-a", "http://127.0.0.1:8787")
            .expect("Alice session created");
        let bob = store
            .create_session("acme", "bob", "corr-b", "http://127.0.0.1:8787")
            .expect("Bob session created");
        store
            .activate(
                &alice.session_id,
                &alice.capability,
                "acme",
                "alice",
                "corr-a",
            )
            .unwrap();
        let mut alice_rx = store
            .subscribe(&alice.session_id, &alice.capability, "corr-a")
            .unwrap();
        // Bob activates his own, differently-correlated scope. This must not
        // replace Alice's projection target.
        store
            .activate(&bob.session_id, &bob.capability, "acme", "bob", "corr-b")
            .unwrap();
        assert!(store.publish("acme", "alice", "corr-a", "[EMAIL:k1:a]", "alice@x".into()));
        assert!(matches!(
            alice_rx.rx.try_recv(),
            Ok(ModProjectionEvent::Mapping { .. })
        ));
        // Bob publishes to his own target; he cannot publish into Alice's key.
        assert!(store.publish("acme", "bob", "corr-b", "[EMAIL:k1:b]", "bob@x".into()));
        assert!(!store.publish("acme", "bob", "corr-a", "[EMAIL:k1:b]", "bob@x".into()));
        // Even a re-bootstrap by another principal of the *same* correlation
        // fails closed rather than displacing Alice.
        let eve = store
            .create_session("acme", "eve", "corr-a", "http://127.0.0.1:8787")
            .expect("Eve session created");
        assert!(store
            .activate(&eve.session_id, &eve.capability, "acme", "eve", "corr-a")
            .is_err());
    }

    #[test]
    fn revoke_requires_the_bootstrapped_capability() {
        let store = ModSessionStore::new(MOD_SESSION_TTL);
        let first = store
            .create_session("acme", "alice", "corr", "http://127.0.0.1:8787")
            .expect("first session created");
        let second = store
            .create_session("acme", "alice", "corr", "http://127.0.0.1:8787")
            .expect("second session created");
        assert!(store.revoke(&first.session_id, &second.capability).is_err());
        assert!(store.revoke(&first.session_id, &first.capability).is_ok());
    }

    #[test]
    fn bounded_queues_drop_under_pressure_and_revoke_clears_them() {
        let store = ModSessionStore::new(MOD_SESSION_TTL);
        let bootstrap = store
            .create_session("acme", "alice", "corr", "http://127.0.0.1:8787")
            .expect("session created");
        store
            .activate(
                &bootstrap.session_id,
                &bootstrap.capability,
                "acme",
                "alice",
                "corr",
            )
            .unwrap();
        let stream = store
            .subscribe(&bootstrap.session_id, &bootstrap.capability, "corr")
            .unwrap();
        for i in 0..(EVENT_CAPACITY * 2) {
            store.publish(
                "acme",
                "alice",
                "corr",
                &format!("[EMAIL:k1:{i}]"),
                "alice@example.com".into(),
            );
        }
        let audit = store.audit_for_tenant("acme");
        assert!(
            audit
                .iter()
                .any(|record| record.outcome == "dropped_backpressure"),
            "full channels must drop with an audit record"
        );

        // Poll drains at most 32 events and caps the response body.
        let drained = store
            .poll(
                &bootstrap.session_id,
                &bootstrap.capability,
                "corr",
                MAX_POLL_EVENTS,
            )
            .unwrap();
        assert_eq!(drained.events.len(), EVENT_CAPACITY);
        assert!(!drained.expired && !drained.closed);
        // Poll atomically drains: a second poll has nothing left.
        assert_eq!(
            store
                .poll(&bootstrap.session_id, &bootstrap.capability, "corr", 1)
                .unwrap()
                .events
                .len(),
            0
        );

        store
            .revoke(&bootstrap.session_id, &bootstrap.capability)
            .unwrap();
        assert!(stream.is_invalidated());
        let closed = store
            .poll(&bootstrap.session_id, &bootstrap.capability, "corr", 32)
            .unwrap();
        assert!(closed.closed && !closed.expired && closed.events.is_empty());
        // A revoked session cannot be re-subscribed or published into.
        assert!(store
            .subscribe(&bootstrap.session_id, &bootstrap.capability, "corr")
            .is_err());
    }

    #[test]
    fn oversize_originals_are_dropped_before_queueing() {
        let store = ModSessionStore::new(MOD_SESSION_TTL);
        let bootstrap = store
            .create_session("acme", "alice", "corr", "http://127.0.0.1:8787")
            .expect("session created");
        store
            .activate(
                &bootstrap.session_id,
                &bootstrap.capability,
                "acme",
                "alice",
                "corr",
            )
            .unwrap();
        assert!(!store.publish(
            "acme",
            "alice",
            "corr",
            "[EMAIL:k1:big]",
            "x".repeat(MAX_ORIGINAL_BYTES + 1)
        ));
        let drained = store
            .poll(&bootstrap.session_id, &bootstrap.capability, "corr", 32)
            .unwrap();
        assert!(drained.events.is_empty());
        assert!(store
            .audit_for_tenant("acme")
            .iter()
            .any(|r| r.outcome == "dropped_oversize"));
    }

    #[test]
    fn remote_origin_and_host_are_refused() {
        let mut headers = loopback_headers("127.0.0.1:8787", None, None);
        assert!(is_loopback_request(&headers));
        headers.insert(header::ORIGIN, "https://evil.test".parse().unwrap());
        assert!(!is_loopback_request(&headers));
        headers = loopback_headers("evil.test:8787", None, None);
        assert!(!is_loopback_request(&headers));
        assert!(request_origin(&headers).is_err());
    }

    #[test]
    fn forged_loopback_headers_from_a_remote_peer_are_refused() {
        // The exact F1 exploit: a remote caller sending Host/Origin = loopback.
        let headers = loopback_headers("127.0.0.1:8787", Some("corr"), None);
        assert!(
            is_loopback_request(&headers),
            "headers alone are forged-truthy"
        );
        let remote_peer: SocketAddr = "203.0.113.9:4444".parse().unwrap();
        assert!(!loopback_request(&headers, remote_peer));
        let local_peer: SocketAddr = "127.0.0.1:4444".parse().unwrap();
        assert!(loopback_request(&headers, local_peer));
    }

    #[test]
    fn lifecycle_audit_is_content_free_and_covers_transitions() {
        let store = ModSessionStore::new(MOD_SESSION_TTL);
        let bootstrap = store
            .create_session("acme", "alice", "corr", "http://127.0.0.1:8787")
            .expect("session created");
        store
            .activate(
                &bootstrap.session_id,
                &bootstrap.capability,
                "acme",
                "alice",
                "corr",
            )
            .unwrap();
        let _ = store
            .subscribe(&bootstrap.session_id, &bootstrap.capability, "corr")
            .unwrap();
        store.publish(
            "acme",
            "alice",
            "corr",
            "[EMAIL:k1:abc]",
            "alice@example.com".into(),
        );
        store
            .revoke(&bootstrap.session_id, &bootstrap.capability)
            .unwrap();
        let audit = store.audit_for_tenant("acme");
        let actions: Vec<&str> = audit.iter().map(|r| r.action.as_str()).collect();
        for expected in ["issue", "activate", "subscribe", "project", "revoke"] {
            assert!(
                actions.contains(&expected),
                "missing {expected} in {actions:?}"
            );
        }
        let all = format!("{:?}", store.audit_all());
        assert!(!all.contains("alice@example.com"));
        assert!(!all.contains(&bootstrap.capability));
        assert!(!all.contains(&bootstrap.session_id));
    }
}
