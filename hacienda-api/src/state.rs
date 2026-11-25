//! Shared application state threaded through every handler.

use hacienda_core::jobs::JobStore;
use hacienda_core::store::object::ObjectStore;
use hacienda_core::store::postgres::presets::PresetStore;
use hacienda_core::store::postgres::usage::UsageStore;
use hacienda_core::store::postgres::versions::DocumentVersionStore;
use hacienda_core::{auth::AuthState, HaciendaFacade};
use hacienda_rag::{GraphStore, RagStore};
use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
};

use crate::handlers::claude_messages::ClaudeMessagesProvider;
use crate::handlers::claude_mod::ModSessionStore;
use crate::handlers::codex_responses::CodexResponsesProvider;
use crate::handlers::codex_viewer::ViewerSessionStore;
use crate::job_state::{GuardedJobStore, JobRetention, RetryPolicy};
use crate::quota::InMemoryQuotaStore;

/// Body / batch limits applied uniformly to every request.
///
/// The defaults — 32 MiB for a body, 64 documents per batch — match the brief and
/// should be sufficient for French legal documents. Both limits are checked before
/// any decoding begins, so an oversized request is rejected cheaply.
#[derive(Debug, Clone)]
pub struct ApiLimits {
    /// Maximum accepted Content-Length (and body read limit) in bytes.
    ///
    /// Bodies that exceed this are rejected with 413 before the body is read.
    pub max_body_bytes: usize,
    /// Maximum number of documents in a single `POST /v1/documents` request.
    pub max_documents: usize,
}

pub(crate) enum ProbeState {
    Idle,
    Running,
    Ready(hacienda_core::capabilities::CapabilityStatus),
}

impl Default for ApiLimits {
    fn default() -> Self {
        Self {
            max_body_bytes: 32 * 1024 * 1024, // 32 MiB
            max_documents: 64,
        }
    }
}

/// State shared by every request handler.
///
/// Clone is cheap — every field is either `Arc` or `Copy`.
#[derive(Clone)]
pub struct ApiState {
    /// The business-logic facade. All document processing, scanning, and audit go here.
    pub(crate) facade: Arc<HaciendaFacade>,
    /// Async job registry, wrapped in [`GuardedJobStore`] so retention and the
    /// retry-disabled policy apply to every producer. Durability is a property
    /// of the injected backend only: a host that builds a migrated Postgres
    /// pool can inject `PostgresJobStore`; the in-repo `hacienda serve` still
    /// injects `InMemoryJobStore`, so those jobs are lost on restart.
    pub(crate) jobs: Arc<dyn JobStore>,
    /// Unwrapped backend, kept so [`Self::with_job_retention`] can re-wrap the
    /// policy without stacking a second guard on top of the first.
    jobs_inner: Arc<dyn JobStore>,
    /// Retention policy kept alongside the backend so builder order does not
    /// matter: swapping the store later reuses the configured policy.
    job_retention: JobRetention,
    /// Auth middleware configuration. Owns the token resolver and route requirements.
    pub(crate) auth: AuthState,
    /// Per-request resource limits.
    pub(crate) limits: ApiLimits,
    /// RAG vector store backing `/v1/rag/*`. `None` means those routes are disabled
    /// (`ApiError::invalid_request`, mirroring how a `None` PII config disables
    /// `/v1/pii/*`'s pipeline-dependent behaviour) — RAG is opt-in, not every embedder
    /// wants the dependency or a live collection store.
    pub(crate) rag_store: Option<Arc<dyn RagStore>>,
    /// GraphRAG store backing `/v1/rag/**/graph/*`. `None` disables those routes
    /// (`ApiError::invalid_request`), mirroring `rag_store`: graph support is
    /// opt-in and the host decides whether it is ephemeral or durable.
    pub(crate) graph_store: Option<Arc<dyn GraphStore>>,
    /// Preset store backing `/v1/presets/*`. `None` means those routes are disabled
    /// (`ApiError::invalid_request`) — unlike `RagStore`/`JobStore`, `PresetStore`
    /// has no in-memory implementation (Postgres is its only backend), so this is
    /// opt-in the same way RAG is: attach it only when a pool is available.
    pub(crate) preset_store: Option<Arc<dyn PresetStore>>,
    /// Document version store backing `/v1/documents/{id}/versions`, `/v1/documents/{id}`,
    /// and the `/diff` routes. `None` means those routes are disabled
    /// (`ApiError::invalid_request`) — like `PresetStore`, `DocumentVersionStore` has no
    /// in-memory backend (Postgres-only), so this is opt-in the same way.
    pub(crate) version_store: Option<Arc<dyn DocumentVersionStore>>,
    /// Object store backing `/v1/uploads/presign` and `/v1/uploads/confirm`. `None` means
    /// those routes are disabled (`ApiError::invalid_request`) — like `PresetStore` and
    /// `DocumentVersionStore`, `ObjectStore`'s only implementation (`S3ObjectStore`) needs
    /// a live bucket, so this is opt-in the same way.
    pub(crate) object_store: Option<Arc<dyn ObjectStore>>,
    /// Usage read-model backing `GET /v1/usage`. `None` means that route is disabled
    /// (`ApiError::invalid_request`) — like `PresetStore`/`DocumentVersionStore`,
    /// `UsageStore`'s only implementation queries `audit_entries` directly (Postgres-only),
    /// so this is opt-in the same way.
    pub(crate) usage_store: Option<Arc<dyn UsageStore>>,
    /// Per-tenant request-rate quota (S1 spec §7), enforced by `quota::quota_middleware`
    /// before `DefaultBodyLimit`/any body decode. `None` means quotas are not enforced —
    /// today's behaviour, and opt-in the same way every other store above is.
    pub(crate) quota_store: Option<Arc<InMemoryQuotaStore>>,
    /// Per-router ONNX probe state; independent API routers must not share it.
    pub(crate) probe_state: Arc<Mutex<ProbeState>>,
    /// Anthropic Messages provider used by the authenticated Claude Code gateway.
    /// `None` keeps the route registered but disabled, which is the safe default.
    pub(crate) claude_messages_provider: Option<Arc<dyn ClaudeMessagesProvider>>,
    /// Codex Responses provider used by the authenticated `/v1/responses` and
    /// `/v1/models` gateway. `None` keeps both routes registered but disabled,
    /// which is the fail-closed default.
    pub(crate) codex_responses_provider: Option<Arc<dyn CodexResponsesProvider>>,
    /// BLAKE3 digest of the local gateway bearer secret. Codex sends it in
    /// `X-Hacienda-Gateway-Token`; `Authorization` is reserved for the user's
    /// ChatGPT token and is forwarded to the official upstream unchanged.
    pub(crate) codex_gateway_secret: Option<[u8; 32]>,
    /// Short-lived, in-memory Codex viewer sessions. Rehydration values never
    /// leave this process except over the authenticated viewer SSE channel.
    pub(crate) viewer_sessions: Arc<ViewerSessionStore>,
    /// Directory holding the loopback viewer assets. `None` disables the viewer
    /// routes; the host opts in explicitly so the default API stays unchanged.
    pub(crate) viewer_assets_dir: Option<PathBuf>,
    /// Short-lived, tenant/principal/correlation-bound Claude Mod projection
    /// sessions. Unlike the cookie-bound Codex viewer, these are bootstrapped
    /// out of band by an authenticated host adapter and never broadcast.
    pub(crate) mod_sessions: Arc<ModSessionStore>,
    /// Server-owned destinations permitted for RAG answer synthesis.
    /// `None` is fail-closed: the request cannot choose an LLM destination.
    pub(crate) rag_egress_allowlist: Option<Vec<String>>,
    /// TeamOnPremise allowlist of email sources. Clients select an opaque `source_ref`;
    /// this registry is the only place that maps it to a tenant-confined Maildir path.
    pub(crate) email_sources: Arc<crate::team_email::EmailSourceRegistry>,
    /// Short-lived, tenant/principal/document/email-bound viewer capabilities and their
    /// content-free audit trail. Never serialized into a job or a renderer DTO.
    pub(crate) viewer_capabilities: Arc<crate::team_email::ViewerCapabilityStore>,
}

impl ApiState {
    /// `jobs` is the repository-backed store chosen by the host. The API adds
    /// retention and retry policy on top via [`GuardedJobStore`]; it never
    /// persists a second copy of job state itself.
    pub fn new(
        facade: Arc<HaciendaFacade>,
        jobs: Arc<dyn JobStore>,
        auth: AuthState,
        limits: ApiLimits,
    ) -> Self {
        let jobs_inner: Arc<dyn JobStore> = jobs;
        let job_retention = JobRetention::default();
        let jobs: Arc<dyn JobStore> = Arc::new(GuardedJobStore::with_policy(
            Arc::clone(&jobs_inner),
            job_retention,
            RetryPolicy::Disabled,
        ));
        Self {
            facade,
            jobs,
            jobs_inner,
            job_retention,
            auth,
            limits,
            rag_store: None,
            graph_store: None,
            preset_store: None,
            version_store: None,
            object_store: None,
            usage_store: None,
            quota_store: None,
            probe_state: Arc::new(Mutex::new(ProbeState::Idle)),
            claude_messages_provider: None,
            codex_responses_provider: None,
            codex_gateway_secret: None,
            viewer_sessions: Arc::new(ViewerSessionStore::default()),
            viewer_assets_dir: None,
            mod_sessions: Arc::new(ModSessionStore::default()),
            rag_egress_allowlist: None,
            email_sources: Arc::new(crate::team_email::EmailSourceRegistry::from_env()),
            viewer_capabilities: Arc::new(crate::team_email::ViewerCapabilityStore::default()),
        }
    }

    /// Replace the injected job repository (e.g. swap an in-memory test store
    /// for a durable `PostgresJobStore`) while keeping the API's retention and
    /// retry policy in force. Builder-style so host setup stays composable.
    #[must_use]
    pub fn with_job_store(mut self, jobs: Arc<dyn JobStore>) -> Self {
        self.jobs_inner = jobs;
        self.jobs = Arc::new(GuardedJobStore::with_policy(
            Arc::clone(&self.jobs_inner),
            self.job_retention,
            RetryPolicy::Disabled,
        ));
        self
    }

    /// Apply a non-default retention policy to the job store without stacking
    /// a second guard. This is the public seam that makes the policy reachable
    /// from a host instead of leaving it fixed at the constructor's default.
    #[must_use]
    pub fn with_job_retention(mut self, retention: JobRetention) -> Self {
        // Clamp before storing as well as passing down: a struct literal can
        // carry `prune_every: 0`, and the stored copy is reused by a later
        // `with_job_store` call.
        let retention = retention.clamped();
        self.job_retention = retention;
        self.jobs = Arc::new(GuardedJobStore::with_policy(
            Arc::clone(&self.jobs_inner),
            retention,
            RetryPolicy::Disabled,
        ));
        self
    }

    /// Override the server-provisioned email source registry (tests / host config).
    #[must_use]
    pub fn with_email_source_registry(
        mut self,
        registry: crate::team_email::EmailSourceRegistry,
    ) -> Self {
        self.email_sources = Arc::new(registry);
        self
    }

    /// Override the viewer capability store (tests / host config).
    #[must_use]
    pub fn with_viewer_capability_store(
        mut self,
        store: crate::team_email::ViewerCapabilityStore,
    ) -> Self {
        self.viewer_capabilities = Arc::new(store);
        self
    }

    /// Enable `/v1/rag/*` by attaching a store. Builder-style so existing call sites
    /// (which construct `ApiState` without RAG) do not need to change.
    #[must_use]
    pub fn with_rag_store(mut self, rag_store: Arc<dyn RagStore>) -> Self {
        self.rag_store = Some(rag_store);
        self
    }

    /// Enable the GraphRAG routes by attaching a graph store. A host wanting
    /// durable graphs can attach a `SqliteGraphStore` (behind hacienda-rag's
    /// `sqlite-graph` feature); the CLI's `serve` uses the in-memory backend.
    #[must_use]
    pub fn with_graph_store(mut self, graph_store: Arc<dyn GraphStore>) -> Self {
        self.graph_store = Some(graph_store);
        self
    }

    /// Enable `/v1/presets/*` by attaching a store. Builder-style, mirroring
    /// `with_rag_store` exactly, so existing call sites are unaffected.
    #[must_use]
    pub fn with_preset_store(mut self, preset_store: Arc<dyn PresetStore>) -> Self {
        self.preset_store = Some(preset_store);
        self
    }

    /// Enable `/v1/documents/{id}/versions`, `/v1/documents/{id}`, and the `/diff` routes
    /// by attaching a store. Builder-style, mirroring `with_preset_store` exactly, so
    /// existing call sites are unaffected.
    #[must_use]
    pub fn with_version_store(mut self, version_store: Arc<dyn DocumentVersionStore>) -> Self {
        self.version_store = Some(version_store);
        self
    }

    /// Enable `/v1/uploads/presign` and `/v1/uploads/confirm` by attaching a store.
    /// Builder-style, mirroring `with_version_store` exactly, so existing call sites are
    /// unaffected.
    #[must_use]
    pub fn with_object_store(mut self, object_store: Arc<dyn ObjectStore>) -> Self {
        self.object_store = Some(object_store);
        self
    }

    /// Enable `GET /v1/usage` by attaching a store. Builder-style, mirroring
    /// `with_object_store` exactly, so existing call sites are unaffected.
    #[must_use]
    pub fn with_usage_store(mut self, usage_store: Arc<dyn UsageStore>) -> Self {
        self.usage_store = Some(usage_store);
        self
    }

    /// Enable per-tenant request-rate quota enforcement (S1 spec §7) by attaching a
    /// quota store. Builder-style, mirroring `with_usage_store` exactly, so existing
    /// call sites are unaffected — quotas stay off until an embedder opts in.
    #[must_use]
    pub fn with_quota_store(mut self, quota_store: Arc<InMemoryQuotaStore>) -> Self {
        self.quota_store = Some(quota_store);
        self
    }

    /// Enable the authenticated Claude Code Messages gateway.
    #[must_use]
    pub fn with_claude_messages_provider(
        mut self,
        provider: Arc<dyn ClaudeMessagesProvider>,
    ) -> Self {
        self.claude_messages_provider = Some(provider);
        self
    }

    /// Enable the authenticated Codex Responses gateway.
    #[must_use]
    pub fn with_codex_responses_provider(
        mut self,
        provider: Arc<dyn CodexResponsesProvider>,
    ) -> Self {
        self.codex_responses_provider = Some(provider);
        self
    }

    /// Access the in-memory Codex viewer sessions for host-side diagnostics.
    #[must_use]
    pub fn viewer_sessions(&self) -> &Arc<ViewerSessionStore> {
        &self.viewer_sessions
    }

    /// Access the in-memory Claude Mod projection sessions for host diagnostics.
    #[must_use]
    pub fn mod_sessions(&self) -> &Arc<ModSessionStore> {
        &self.mod_sessions
    }

    /// Override the Mod projection store (e.g. a shorter TTL in tests or a
    /// host-specific lifetime policy). Builder-style, like the other stores.
    #[must_use]
    pub fn with_mod_sessions(mut self, store: Arc<ModSessionStore>) -> Self {
        self.mod_sessions = store;
        self
    }

    /// Configure the local gateway secret. Only its digest is retained; the
    /// plaintext never enters state, a log, or a response body.
    #[must_use]
    pub fn with_codex_gateway_secret(mut self, secret: impl AsRef<str>) -> Self {
        self.codex_gateway_secret = Some(*blake3::hash(secret.as_ref().as_bytes()).as_bytes());
        self
    }

    /// Enable the same-origin loopback viewer by pointing it at its asset dir.
    #[must_use]
    pub fn with_viewer_assets(mut self, dir: impl Into<PathBuf>) -> Self {
        self.viewer_assets_dir = Some(dir.into());
        self
    }

    /// Configure the server-owned RAG LLM egress allowlist.
    ///
    /// The list is copied into each answer request and cannot be supplied by the
    /// caller. Leaving it unset deliberately disables outbound RAG model calls.
    #[must_use]
    pub fn with_rag_egress_allowlist<I, S>(mut self, destinations: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.rag_egress_allowlist = Some(destinations.into_iter().map(Into::into).collect());
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hacienda_core::{
        auth::{authn::DevTokenResolver, AuthState},
        jobs::InMemoryJobStore,
        tenancy::TenantId,
        HaciendaConfig,
    };

    /// The retention builder must reach the store the handlers actually use,
    /// not leave a policy object with no effect on job reads.
    #[tokio::test]
    async fn with_job_retention_applies_to_the_handlers_job_store() {
        let repo: Arc<dyn JobStore> = Arc::new(InMemoryJobStore::new());
        let state = ApiState::new(
            Arc::new(HaciendaFacade::new(HaciendaConfig::default()).unwrap()),
            Arc::clone(&repo),
            AuthState::new(Arc::new(DevTokenResolver)),
            ApiLimits::default(),
        )
        .with_job_retention(JobRetention::new(0, 10).with_prune_every(1));

        let tenant = TenantId::new("acme");
        let job = state.jobs.create(&tenant, None).await.unwrap();
        state
            .jobs
            .finish(&tenant, &job.id, "{}".to_owned())
            .await
            .unwrap();
        assert!(
            state.jobs.get(&tenant, &job.id).await.unwrap().is_none(),
            "a zero-TTL policy set via the builder must hide the terminal job"
        );
    }

    fn state_with(repo: Arc<dyn JobStore>) -> ApiState {
        ApiState::new(
            Arc::new(HaciendaFacade::new(HaciendaConfig::default()).unwrap()),
            repo,
            AuthState::new(Arc::new(DevTokenResolver)),
            ApiLimits::default(),
        )
    }

    async fn assert_zero_ttl_applied(state: &ApiState) {
        let tenant = TenantId::new("acme");
        let job = state.jobs.create(&tenant, None).await.unwrap();
        state
            .jobs
            .finish(&tenant, &job.id, "{}".to_owned())
            .await
            .unwrap();
        assert!(
            state.jobs.get(&tenant, &job.id).await.unwrap().is_none(),
            "retention must apply regardless of builder call order"
        );
    }

    /// LOW follow-up: `with_job_store` must not silently reset a previously
    /// configured retention policy.
    #[tokio::test]
    async fn job_store_then_retention_builder_order() {
        let state = state_with(Arc::new(InMemoryJobStore::new()))
            .with_job_store(Arc::new(InMemoryJobStore::new()));
        let state = state.with_job_retention(JobRetention::new(0, 10).with_prune_every(1));
        assert_zero_ttl_applied(&state).await;
    }

    #[tokio::test]
    async fn retention_then_job_store_builder_order() {
        let state = state_with(Arc::new(InMemoryJobStore::new()))
            .with_job_retention(JobRetention::new(0, 10).with_prune_every(1))
            .with_job_store(Arc::new(InMemoryJobStore::new()));
        assert_zero_ttl_applied(&state).await;
    }

    /// A struct literal bypasses the constructor's clamping; the builder must
    /// normalise it so the guard cannot divide by zero on the first write.
    #[tokio::test]
    async fn struct_literal_retention_is_clamped_by_builder() {
        let state =
            state_with(Arc::new(InMemoryJobStore::new())).with_job_retention(JobRetention {
                terminal_ttl_secs: 0,
                max_terminal_per_tenant: 0,
                prune_every: 0,
            });
        assert_zero_ttl_applied(&state).await;
    }
}
