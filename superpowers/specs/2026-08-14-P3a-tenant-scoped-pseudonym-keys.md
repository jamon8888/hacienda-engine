# P3a — Tenant-scoped pseudonym keys

**Date:** 2026-08-14
**Status:** Implemented (2026-08-14) — all six tasks shipped; see §7 for what landed and the
one deviation from the design as originally proposed.
**Extends:** `2026-08-01-P3-pseudonymisation-as-a-service.md` §5 ("Cloisonnement par
tenant") and `2026-08-01-S1-tenancy-and-projects.md` — this spec is what closes P3 §5's
open dependency on S1, which S1 itself never actually delivered for the pseudonym layer
specifically (see §1).
**Depends on:** nothing unshipped — `TenantCtx`/`Caller::tenant_ctx()` already exist and
already reach every call site this spec touches (§3.3)
**Sibling:** `2026-08-14-S1b-tenant-scoped-audit-review-job-document-stores.md` — covers
every other store S1 promised tenant scoping for (audit, review, jobs, document versions,
presets) that this spec doesn't. The two together close S1 completely; independent of
each other, may ship in either order.
**Blocks:** nothing new adopting pseudonymisation in a multi-tenant deployment should ship
before this — see §1 for why the current behavior is a live cross-tenant leak, not a
theoretical gap

---

## 1. Problem, verified against current code

`Pseudonymiser::token(&self, category: &PiiCategory, text: &str)`
(`hacienda-core/src/redaction/pseudonym.rs:487`) derives the deterministic AES-256-SIV token
from `category` + `text` + the pseudonymiser's own key material — nothing else.
`KeyResolver::active()`/`resolve()` (same file, lines 295/302) take no tenant parameter.
`HaciendaFacade` holds exactly one `Option<Arc<Pseudonymiser>>`
(`hacienda-core/src/facade.rs:48`), built once, from one `KeyResolver` call, at facade
construction (`with_key_resolver`, `facade.rs:178-196`) — for the lifetime of the process.

**Consequence:** two different tenants sharing one deployment (one `HaciendaFacade`, the
normal shape for a multi-tenant API server: `ApiState.facade: Arc<HaciendaFacade>` is built
once in `hacienda-api`) redacting the same value — the same email, the same SSN, the same
IBAN — get the byte-identical token. Any tenant who can compare tokens across a shared
surface (a support ticket, a demo environment, an analytics pipeline reading redacted
exports from multiple tenants) can correlate identities across organizations. This is
exactly the leak `2026-08-01-hacienda-platform-parity-program.md` §4's S1 section names
("deux tenants ayant la même valeur obtiennent le même jeton, ce qui est une fuite par
corrélation") and P3 §5 promises is closed ("Deux tenants portant la même valeur obtiennent
deux jetons différents") — but S1 as shipped built `TenantCtx`/`TenantId`/`Caller::tenant_ctx()`
(`hacienda-core/src/tenancy.rs`, `auth/mod.rs:255`) without ever threading a tenant into the
pseudonymisation layer specifically. `TenantCtx` reaches every store trait; it reaches
nothing in `redaction/pseudonym.rs` or `pii/pipeline.rs`.

**Why this is urgent on its own clock, not just important.** Once a value is tokenized under
today's non-tenant-scoped key, that token is durable — it's what's stored, exported, and
diffed against (E2). Fixing this after a deployment has real multi-tenant data means
migrating every already-minted token to a new derivation, live, which is exactly the
"retrofitting after production" cost the platform-parity program flags as the one thing
worth not deferring. Fixing it now, before any multi-tenant corpus exists, costs a contained
`hacienda-core` change with no data migration.

## 2. Non-objectives

| Deferred | Reason |
| --- | --- |
| A KMS-backed `KeyResolver` (Vault, AWS KMS) | S2 (`2026-08-01-hacienda-platform-parity-program.md` §4), not written — "dépend du choix de backend." This spec's `EnvKeyResolver` extension is a stopgap for the same reason the current single-tenant `EnvKeyResolver` is: it get the *isolation property* correct now, and is replaceable later without touching the isolation logic in `Pseudonymiser`/`KeyResolver`'s trait shape. |
| Per-tenant key rotation UI/API (`POST /v1/keys/rotate` scoped per tenant) | P3 §4 already covers rotation generally; this spec only makes resolution tenant-aware, not the rotation surface. Rotation per tenant falls out of the same `&TenantId` parameter once P3's `/v1/keys/*` routes exist. |
| Migrating pre-existing single-tenant deployments' tokens to per-tenant derivation | Not needed — see §4's back-compat requirement: the default tenant's tokens are untouched by this change. |

## 3. Design

### 3.1 `KeyResolver` gains a tenant parameter

```rust
pub trait KeyResolver: Send + Sync {
    fn active(&self, tenant: &TenantId) -> Result<PseudonymKey, PseudonymError>;
    fn resolve(&self, tenant: &TenantId, id: &KeyId) -> Result<PseudonymKey, PseudonymError>;
}
```

**Back-compat requirement, non-negotiable:** `EnvKeyResolver` resolving `TenantId::default_tenant()`
must return *exactly* the same key material it returns today for a bare (untenanted) lookup.
This is what makes the change safe to ship into an existing single-tenant deployment without
re-provisioning anything or invalidating a single already-minted token — every token minted
before this spec ships was, in effect, minted "for" the default tenant, and must keep
revealing.

**`EnvKeyResolver`'s lookup convention:** its current constructor
(`EnvKeyResolver::with_lookup(lookup: impl Fn(&str) -> Option<String>)`,
`pseudonym.rs:335`) takes a bare variable-name lookup closure. Extend it to build the lookup
key from `(tenant, name)`: for `TenantId::default_tenant()`, the variable name is unchanged
(`HACIENDA_PSEUDONYM_KEY_ACTIVE`, etc. — whatever `ACTIVE_KEY_VAR`/`KEY_BYTES` name today);
for any other tenant, the variable name gains a `HACIENDA_TENANT_<tenant>_` prefix. A tenant
with no provisioned variable is a resolution failure (`PseudonymError::NoActiveKey`/`KeyNotFound`)
— per `KeyResolver::active`'s own existing doc comment, *"This must never fall back to a
default or generated key"* — extended here to also never fall back to another tenant's key.
This is a deliberate operational cost (every tenant needs its key material provisioned
explicitly before it can pseudonymise) in exchange for the alternative being a silent
correlation leak.

### 3.2 `PiiPipeline` becomes cacheable per tenant, reusing the loaded detector

Confirmed by reading `hacienda-core/src/pii/pipeline.rs`: `PiiPipeline::assemble` already
takes `detector: Option<NerDetector>` and `pseudonymiser: Option<Arc<Pseudonymiser>>` as
**separate** parameters — this separation already exists, for testing
(`with_detector_and_pseudonymiser`, used by this file's own test module to vary the
pseudonymiser against a fixed detector). `NerDetector` (`hacienda-core/src/pii/ner.rs:52`)
already wraps its expensive part — `backend: Arc<dyn NerBackend>` — behind an `Arc`; the
rest of the struct (`categories: Vec<EntityCategory>`, `threshold: f32`) is cheap. Adding
`#[derive(Clone)]` to `NerDetector` (currently absent) makes constructing a second
`PiiPipeline` that reuses the same loaded model weights a cheap operation — no re-load, no
duplicated GLiNER2/ONNX runtime.

This is the mechanism: `HaciendaFacade` builds **one detector, once**, at facade
construction (as today), but instead of baking one `Pseudonymiser` into one `PiiPipeline`
permanently, it holds a small per-tenant cache:

```rust
pii_pipelines: Mutex<HashMap<TenantId, Arc<PiiPipeline>>>,
shared_detector: Option<NerDetector>,   // built once; Clone is now cheap (§3.2)
pii_config: PipelineConfig,             // needed to rebuild a pipeline per tenant
key_resolver: Option<Arc<dyn KeyResolver>>,
retired_keys: Vec<KeyId>,
```

`HaciendaFacade::pii_pipeline_for(&self, tenant: &TenantId) -> Result<Arc<PiiPipeline>, HaciendaError>`:
look up `tenant` in the cache; on miss, resolve that tenant's `Pseudonymiser` via
`key_resolver` (§3.1), build a `PiiPipeline` via `with_detector_and_pseudonymiser(pii_config.clone(),
shared_detector.clone(), Some(Arc::new(pseudonymiser)))`, insert, return. A tenant with no PII
configured at all (`self.pii_config` absent) behaves exactly as today —
`HaciendaError::PiiDisabled` before this method is even reached.

**Why a cache and not per-call resolution.** `Pseudonymiser::cipher` already documents that
constructing an `Aes256Siv` per call is cheap and deliberate (`pseudonym.rs`'s comment on
why the cipher isn't cached inside `Pseudonymiser` itself) — but *resolving key material*
via `KeyResolver` is a separate cost this spec doesn't want to pay on every redacted span
(an env lookup today; a KMS round-trip once S2 lands). Caching the built `Pseudonymiser` per
tenant, invalidated only by an explicit rotation call (out of scope, §2), keeps the
per-span cost exactly what it is today while making per-tenant resolution a one-time cost
per tenant per process lifetime.

### 3.3 Call sites: no signature changes needed above `hacienda-core`

Confirmed: `Caller::tenant_ctx()` (`hacienda-core/src/auth/mod.rs:255`) already resolves a
`TenantCtx` for both `Caller::Trusted` (→ `TenantId::default_tenant()`) and
`Caller::Principal` (→ the authenticated context's tenant) — and every public facade method
already takes `Caller` as a parameter. **`hacienda-api`, `hacienda-cli`, and `hacienda-mcp`
need zero changes** — the tenant is already reachable everywhere it's needed; only
`hacienda-core`'s internals currently ignore it. The facade-internal change is: every method
that currently reads `self.pii_pipeline.as_ref()`/`self.pseudonymiser.as_ref()` switches to
`self.pii_pipeline_for(&caller.tenant_ctx().tenant)?`. From my read of `facade.rs`, that's
the following methods (not exhaustive — verify against the file at implementation time):

- `redact_text_with_auth` / `scan_text_with_auth`
- `process_batch_with_auth` (via `detect_concurrently`, which currently receives
  `pipeline: &Arc<PiiPipeline>` as an explicit parameter already — the caller just needs to
  pass the tenant-resolved one instead of `self.pii_pipeline.as_ref()`)
- `reveal_token_with_auth` (currently reads `self.pseudonymiser` directly, not through a
  `PiiPipeline` — needs its own `self.pseudonymiser_for(&tenant)` accessor, or route through
  `pii_pipeline_for` and expose the inner pseudonymiser)
- Every `redact_structured_fields`/`redact_document_recursively`/`redact_table`/etc. helper
  added by the P7 fix (`2026-08-13-P7-structured-field-redaction-gap.md`) already takes
  `pipeline: &Arc<PiiPipeline>` as an explicit parameter (P7 threaded it through
  specifically so redaction helpers don't each independently reach into `self` — this
  spec's tenant resolution slots into the *one* place each top-level method already
  resolves `pipeline` before calling into them, not into every helper).

## 4. Migration and back-compat

- **No data migration.** No token stored today changes meaning: every token minted so far
  was minted under what becomes, after this ships, `TenantId::default_tenant()`'s key — and
  §3.1 requires that key be unchanged. `reveal` for an old token continues to work
  unmodified.
- **A pre-existing single-tenant deployment upgrading needs no new configuration.** Every
  caller that doesn't set up multi-tenancy resolves through `Caller::Trusted` or a
  `Principal` with no explicit tenant, both of which map to `TenantId::default_tenant()` —
  identical behavior to today.
- **A deployment provisioning a second tenant must provision that tenant's key material
  explicitly** (§3.1) before that tenant can pseudonymise — this is the intended friction,
  not a bug to smooth over.

## 5. Tests (adapted from P3 §7, now concrete against this design)

| Test | Assertion |
| --- | --- |
| `default_tenant_token_unchanged_by_this_change` | A value pseudonymised under `TenantId::default_tenant()` before and after this spec ships produces the same token, given the same env-provisioned key. Regression guard for §4. |
| `two_tenants_same_value_different_tokens` | The founding test P3 §7 already names. Two `TenantId`s, same plaintext, same category → different tokens. |
| `tenant_with_no_provisioned_key_fails_closed` | A tenant absent from `EnvKeyResolver`'s lookup gets `PseudonymError::NoActiveKey`/`KeyNotFound`, never a fallback to another tenant's key or a generated one. |
| `pii_pipeline_for_reuses_the_loaded_detector_across_tenants` | Constructing pipelines for two tenants does not reload the NER backend — assert on a call-counting test double `NerBackend`, mirroring how `CountingAuditStore` proves D3 elsewhere in this codebase. |
| `reveal_resolves_the_callers_own_tenants_key` | A `Caller::Principal` from tenant A cannot have a tenant-B token revealed through their own facade call — ties into P3's existing `cross_tenant_token_is_unknown_key_not_forbidden` (D-P3-6): unknown key, not forbidden, so a probing caller doesn't learn the token is valid elsewhere. |

## 6. Exit criteria

- `KeyResolver`/`EnvKeyResolver` changes compile and every existing (single-tenant) test in
  `hacienda-core/src/redaction/pseudonym.rs` and `hacienda-core/src/facade.rs` continues to
  pass unmodified except for the mechanical addition of a `&TenantId::default_tenant()`
  argument at call sites that construct a resolver directly in test setup.
- The five tests in §5 pass.
- No call site outside `hacienda-core` changes signature (§3.3) — if implementation
  discovers one is needed, that's a signal this design missed something, not a small
  extension to wave through.

## 7. Implementation notes

Shipped as designed in §3.1–§3.3, with all five §5 tests passing and every §6 exit
criterion met. `hacienda-api`/`hacienda-cli`/`hacienda-mcp` needed no call-site signature
changes, confirming §3.3's prediction — the one exception (below) is a constructor-time
change, not a per-request call site.

**One real, small, justified deviation from §3.2's sketch:** `HaciendaFacade::with_key_resolver`
took `resolver: &dyn KeyResolver` before this spec; it now takes `resolver: Arc<dyn
KeyResolver>`. The per-tenant cache means the facade must retain the resolver to build
later tenants' pipelines lazily, not just consume it once at construction — a borrowed
reference can't outlive the constructor call. Every caller of `with_key_resolver`
(`hacienda-cli`, `hacienda-api` test setup) updated to pass `Arc::new(resolver)`; this is a
one-time constructor change, not a widened per-request surface.

**One correction to §3.3's open question, found by a CLI integration-test regression:**
§3.3 offered two options for `reveal_token_with_auth` — "its own
`self.pseudonymiser_for(&tenant)` accessor, or route through `pii_pipeline_for` and expose
the inner pseudonymiser." The first implementation attempt took the second option (a
`PiiPipeline::pseudonymiser()`/`RedactionEngine::pseudonymiser()` accessor pair, reusing the
`pii_pipeline_for` cache), on the reasoning that it avoided a second per-tenant map. That
broke `hacienda-cli`'s `pii_reveal.rs` integration tests: `with_key_resolver` can be, and is
in real CLI usage (`hacienda pii reveal <token>` run with no `[pii]` config section at
all — a config the CLI's `run_pii_reveal` always builds via `load_config(..., None, None)`,
carrying no pipeline config), called with a resolver but no `[pii]` section configured.
Routing reveal through `pii_pipeline_for` made it depend on `config.pii` being `Some`, which
this call shape doesn't guarantee — a real, pre-existing decoupling (`pii_pipeline` vs.
`pseudonymiser` were always two independent optional fields before this spec) that the
"one cache" sketch had missed. Fixed by taking §3.3's first-offered option instead: a
dedicated `pseudonymisers: Option<Mutex<HashMap<TenantId, Arc<Pseudonymiser>>>>` cache,
gated on `key_resolver.is_some()` rather than `config.pii.is_some()`, with its own eager
default-tenant build in `build()` for the same fail-fast-at-startup guarantee
`pii_pipelines` gets. `pii_pipeline_for` and `pseudonymiser_for` share the actual key
resolution step (`build_pseudonymiser`) — only the cache and its gating condition differ,
so this is not a return to fully duplicated logic, just two caches instead of one.

Verified: `cargo test --workspace` — every test passes except the five pre-existing,
environment-only failures unrelated to this change (two read-only-file-permission tests
that don't hold as root, and three testcontainers/Postgres tests that need Docker, which
this sandbox does not have).
