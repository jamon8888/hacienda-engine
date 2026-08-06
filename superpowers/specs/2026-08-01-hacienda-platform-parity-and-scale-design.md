# Design Spec: Full-Platform API/SDK Parity and Horizontal Scale

**Date:** 2026-08-01
**Status:** Draft — decisions recorded, implementation gated on §9
**Scope:** `hacienda-engine` HTTP surface, a new `hacienda-sdks` client-SDK repo, and the
Cactus on-device target. Builds on, and does not duplicate, `2026-07-28-hacienda-cli-api-integration-design.md`
(hereafter "the integration spec"), whose §12 horizontal-scaling architecture (segmented
audit chains, adapter-aware routing) is adopted here unmodified and extended to new state.
**Non-scope:** re-deriving §12's audit-chain design, re-litigating Decision 1 (redacting
proxy, not superset) from the integration spec — both stand.

---

## 1. Executive Summary

The prior spec answered "does hacienda have a CLI and API." This one answers a different
question: **if the goal is to offer everything xberg Enterprise offers, plus hacienda's own
compliance layer, as a horizontally scalable multi-tenant service with client SDKs — what
is still missing, and in what order does it get built.**

Three gap categories, verified against source, not the integration spec's aspirational
tables:

1. **xberg-parity gaps.** hacienda-api exposes 6 content routes today. xberg Enterprise's
   Python SDK surfaces 33 distinct operations (§3.1). RAG, versioning/diff, presets,
   presigned uploads, usage/metering, and auth/login have no hacienda equivalent at all.
2. **hacienda's own designed-but-unbuilt additions.** The integration spec's Phase 5
   (audit/review/compliance/glossary HTTP routes) has not shipped, and the `pii:reveal`
   endpoint — the only thing that makes Phase 0's AES-SIV pseudonymisation useful in a
   running service rather than a one-way hash — does not exist either.
3. **Net-new platform work with no prior art in either repo.** There is no hacienda client
   SDK repository. There is no Postgres-backed store, so nothing here can run as more than
   one replica today despite §12 being designed for it. There is no on-device (Cactus)
   target, which is hacienda's actual differentiator against xberg Enterprise.

The through-line: every new surface below is specified against the **same statelessness
contract** the integration spec set for audit/review/jobs in §12.2 — `HaciendaFacade` holds
`Arc<dyn Store>`, no sticky sessions, readiness gates on store reachability. A RAG index or
a usage counter that reintroduces per-replica state undoes §12 for the one thing it was
built to fix.

---

## 2. Confirmed Decisions

| # | Decision | Value |
|---|----------|-------|
| 1 | Parity strategy | **Build hacienda-native equivalents, not an xberg-passthrough expansion.** The `xberg-passthrough` feature (integration spec Decision 2) stays off-by-default and unrelated; RAG/versions/presets/uploads/usage get first-class, capability-gated hacienda routes so every content-bearing path still passes through `HaciendaFacade`. |
| 2 | RAG storage + scope | **pgvector on the same Postgres instance from Decision 5, not an external vector store.** `xberg-rag` is not a dependable crate (§3.5 — confirmed absent at the true-latest official `v1.0.8`/`origin/main`, not just the pinned `v1.0.2`; removed from the public workspace before 1.0.0, single internal consumer); its own doc comments state Xberg Enterprise's backend is "tenant-scoped pgvector." hacienda mirrors that choice rather than inventing one, behind a `RagStore` trait recovered near-verbatim from `xberg-rag`'s final pre-removal source (`VectorStore`/`Filter`/`RetrieveQuery`/`types.rs` — MIT-licensed, forkable from git history; file-by-file plan in §3.7) rather than designed from scratch. hacienda-rag's own scope is the orchestration/storage layer only — embeddings, sparse embeddings, late-interaction, reranking, and RAG-aware chunking are not reimplemented; they're called directly from xberg-core's still-live `embeddings`/`sparse_embeddings`/`late_interaction`/`reranking`/`chunking::chunk_for_rag` modules (§3.7). |
| 3 | Metering | **Usage is derived from the audit chain, not tracked separately.** Every billable operation already produces an audit entry (integration spec §5, rule 2); usage aggregation is a read-model over `AuditStore`, not a second source of truth that can drift from it. |
| 4 | Auth/login | **API-key issuance and rotation, not a session/cookie login.** hacienda is a machine-to-machine API; xberg's `login`/`auth_config` pair maps to `POST /v1/auth/keys` (issue) and `DELETE /v1/auth/keys/{id}` (revoke), not a browser session flow. |
| 5 | Store backend for scale | **Postgres, as §12.6 already specified** — this spec adds the schema surface for RAG/usage/auth on top of the segments/review/jobs tables §12 already designs for. One backend, not one per subsystem. |
| 6 | SDK location | ~~New sibling repo `hacienda-sdks`, structurally cloned from `xberg-sdks`.~~ **Corrected 2026-08-05: `sdks/{python,typescript}` in this repo**, not a new one — the session's GitHub App cannot create repos, and the monorepo shape turned out simpler anyway (no cross-repo spec-sync). Same codegen-core + hand-wrapper split, same dual-target pattern, retargeted to a `target: "cloud" \| "device"` axis instead of xberg's `"enterprise" \| "pro"`. See §8. |
| 7 | Mobile/on-device target | **`target: "device"` is a distinct SDK backend, not a tier of the cloud API.** It talks to an embedded Cactus runtime, not `hacienda-api` over HTTP. The SDK method surface is shared where semantics match (extract, scan) and explicitly narrower where they can't (no audit segment reconciliation on-device; no multi-tenant RAG collections, one local index per app). |
| 8 | Reveal | **`POST /v1/pii/reveal` ships before any new xberg-parity route**, per §9 Phase ordering — it is the one item that makes an already-shipped control (AES-SIV pseudonymisation) functional rather than decorative. |
| 9 | xberg's built-in `redaction-rehydrate` feature | **Not adopted.** hacienda keeps its own stateless AES-SIV construction (integration spec §12.3). xberg's `RehydrationMap` is a stored, scrypt-passphrase-encrypted lookup blob — correct for a single-writer document, but reintroduces exactly the shared-mutable-state problem §12's stateless-facade design exists to avoid. See §3.6. |

---

## 3. Verified Ground Truth

Checked 2026-08-01 against `/home/jamin/Documents/hacienda-engine`,
`/home/jamin/Documents/xberg` (working tree, tags fetched), and `/home/jamin/Documents/xberg-sdks`
(working tree + `git ls-remote --tags`). Supersedes any state described as "planned" in the
integration spec that has since shipped, and corrects one place where that spec's own
gap-closure annotations undersold what exists. Two corrections to this spec's own first draft
are folded in below rather than left standing: §3.1's evidence for the RAG group was weaker
than presented, and §4.1/§4.3's versioning shape was vaguer than the real spec, which was
findable and is now cited directly.

### 3.1 xberg Enterprise SDK surface — the parity target, with evidence strength noted

`v1.0.2` is confirmed still the latest xberg core tag (`git tag --sort=-v:refname` on the
sibling checkout; `[Unreleased]` in `CHANGELOG.md` is empty) — the integration spec's pin is
current, no correction needed there.

`xberg-sdks` itself is **pre-1.0 and mid-rebrand**: `VERSION` reads `0.3.1`, the newest pushed
tag is `v0.3.1`, and the "xberg-io-sdk / XbergClient" naming this spec and prior conversation
turns used throughout is the **`[Unreleased]` CHANGELOG entry**, not yet tagged or published.
What is live in the registries today is still Kreuzberg-branded (`kreuzberg-cloud-sdk-dart`,
class `KreuzbergCloudClient`, `'kreuzberg-cloud-sdk-dart/$packageVersion'` — read directly from
`packages/dart/lib/src/client.dart`). This doesn't change the *shape* being cloned in §8, but
it means "mature, production dual-target pattern" should be read as "mature pattern, immature
package" — treat published-package specifics (names, registry links) as moving targets, the
architecture as stable.

**A concrete reason not to trust the SDK's method list as ground truth on its own:** the
`[Unreleased]` entry removes `create_sandbox_key`/`fromSandbox`/`FromSandbox` in every language
because it "POSTed to `/v1/sandbox/key`, which exists in **neither service**... the helper
always 404'd." That method shipped in `v0.0.1` and survived multiple releases as dead code
before being caught. `client.py`'s method list is what the SDK *claims* the server does, not
independent confirmation that it does it — evidence strength varies by group below.

`xberg-sdks/packages/python/src/xberg_io_sdk/client.py`, public methods (sync and async
mirrors of the same 33 operations), cross-checked against `CHANGELOG.md`:

| Group | Operations | Evidence strength | hacienda equivalent |
|---|---|---|---|
| Extraction | `extract`, `extract_batch`, `extract_and_wait` | Strong — `v0.0.1` original surface, exercised by fixture-driven tests since `v0.2.0` | `POST /v1/documents`, `/v1/documents/async` — **have** |
| Jobs | `get_job`, `list_jobs`, `wait_for_job(s)`, `get_job_result` | Strong | **have** `get_job` only; `list_jobs`/`get_job_result` **missing** |
| Audit | `audit` | Method-name only, no changelog entry found | `AuditStore` exists (`hacienda-core/src/audit/store.rs:41`); **no route** |
| RAG | `list/create/get/delete_rag_collection`, `list/add_rag_documents`, `reindex_rag_document`, `rag_retrieve`, `migrate_rag_embeddings`, `get_rag_migration_job`, `get_rag_job`, `get/set_rag_config` | **Weak — zero mentions across all 126 lines of `CHANGELOG.md`**, unlike every other group. Present in `client.py` only. Given the sandbox-key precedent above, treat the entire RAG group's server-side existence as unconfirmed, not just its schema — the §9 phase-12 decision to spike this before building routes against it is now more important, not less | **entirely missing** — no route, no `RagStore` |
| Auth | `auth_config`, `login` | Method-name only | **missing** — capability middleware exists (`hacienda-core/src/auth/authz.rs`), but nothing issues a credential |
| Presets | `list/create/delete_saved_preset`, `presets`, `get_preset` | Method-name only | **missing** |
| Versioning | `versions`, `diff`, `get_diff_job` | **Strong — fully documented in `CHANGELOG.md` `[0.3.0]`**, exact route shapes below in §4.1/§4.3 | **missing** — hacienda-core has no version concept for a processed document |
| Uploads | `presign_upload`, `confirm_upload` | Strong — Dart `v0.1.0` shipped this first, ahead of Python/TS | **missing by design** so far — integration spec's inline-base64-only rule (`hacienda-api-cli-surface` convention) was an SSRF mitigation for a surface that didn't need large files yet; presign changes that calculus (§5 below) |
| Metering | `usage` | Method-name only | **missing** |

Also found: a **sandbox onboarding route**, `/v1/sandbox/public/extract`, mints and revokes
keys server-side (replacing the removed client-side helper). Out of scope for this spec — it's
a public-demo concern, orthogonal to §6's tenant key issuance — noted so it isn't rediscovered
and mistaken for a gap.

### 3.2 hacienda's own current route table

`hacienda-api/src/routes.rs` — unchanged since the integration spec's Phase 4, 10 routes
total: `/health`, `/version`, `/info`, `/openapi.json` (public); `POST /v1/documents`,
`POST /v1/documents/async`, `GET /v1/jobs/{id}`, `POST /v1/pii/scan`, `POST /v1/pii/redact`,
`GET /v1/pii/config` (all `Capability::DocumentsProcess`). The file's own comment still
reads: *"Audit and review endpoints are Phase 5 and must not be added to this table until
then."*

### 3.3 Store layer — more complete than the integration spec's gap list implies

The integration spec's §8 marks gaps 1/2/5/7 "Closed." Verified directly:

| Store | Trait | Backends | Evidence |
|---|---|---|---|
| Audit | `AuditStore` | in-memory, file (JSONL, per-node), wasm32 IndexedDB | `hacienda-core/src/audit/store.rs:41-110`, `store_file.rs`, `store_idb.rs` |
| Review | `ReviewStore` | in-memory, file | `hacienda-core/src/review/store.rs:55-119`, `store_file.rs` |
| Jobs | `JobStore` | in-memory only | `hacienda-core/src/jobs/store.rs:31-63`, explicitly commented "provisional (until Phase 4)" |
| Pseudonym keys | `KeyResolver` seam | config/env-sourced | `redaction/pseudonym.rs` |

**No Postgres backend exists for any of them** — zero `sqlx`/`diesel`/`tokio-postgres` in
any workspace `Cargo.toml`. §12.6 of the integration spec specifies Postgres as the scaled
backend; nothing implements that seam yet. This is the single largest blocker to running
more than one hacienda-api replica, independent of every gap this spec adds.

### 3.4 Pseudonymisation is real; reveal is not wired

`redaction/pseudonym.rs:407-570` performs genuine AES-256-SIV per the integration spec's
§12.3 construction — `[CATEGORY:key_id:base32]` tokens, a working `reveal()` method at
lines 520-546. **No route calls it.** This is the highest-priority gap in this spec: a
correct, already-implemented cryptographic control with no way to invoke its inverse over
the API is worse than not having built it, because it is indistinguishable from a working
feature until someone tries to use it.

### 3.5 xberg-rag — existed, was removed from the public workspace before 1.0.0, and is not a dependable crate

Correction to this spec's first draft, which described `xberg-rag::RagEngine<E: Embedder>` as
"a library, not a service" available to depend on. **It is not currently available at all.**
**Re-verified directly against the official remote** (`git fetch origin --tags --prune` against
`github.com/xberg-io/xberg`, the same URL hacienda's `Cargo.toml` pins) rather than trusting the
local checkout's state: the local clone was itself stale — official releases have moved from the
pinned `v1.0.2` to `v1.0.8`, with `origin/main` further ahead. `crates/` at both `v1.0.8` and
`origin/main` still has no `xberg-rag`; it was not reintroduced. `gh api orgs/xberg-io/repos`
confirms no standalone RAG repo exists anywhere in the org either (16 repos: `xberg`, `sdks`,
`liter-llm`, `alef`, `crawlberg`, `plugins`, etc. — nothing RAG-shaped). The absence is real and
current, not a local-clone artifact. `git log --all --oneline -i --grep="rag"` on the checkout
finds:

```
77e2fd3d71 chore: remove xberg-rag crate from workspace
```

dated **2026-07-12** — 16 days *before* the `v1.0.0` tag (2026-07-28). The commit message:

> Relocated to the monomyth repo, its only consumer. Enterprise and basemind have their own
> vector-store implementations, so xberg-rag has no ecosystem consumers to justify a workspace
> crate + published artifact. As monomyth's internal RAG layer it depends on xberg (app ->
> library), the correct direction.

Two consequences that change §7 below:

1. **`xberg-rag` was never a general-purpose published artifact** — it had exactly one internal
   consumer (`monomyth`) even before removal, and is not reachable as a git/crates.io dependency
   today. It is not "in the workspace and unused" (this spec's first draft); it's gone from the
   public repo entirely. Reusing it means forking from git history (`77e2fd3d71^` has the last
   full source, ~5,200 lines: `backends/{memory,sqlite}.rs`, `capability.rs`, `filter.rs`,
   `pipeline.rs`, `query.rs`, `registry.rs`, `scoring.rs`, `store.rs`, `stream.rs`, `types.rs`)
   under xberg's MIT license — a real option, not a live dependency.
2. **The removal commit directly answers §7's open backend question.** The deleted crate's own
   `lib.rs` doc comment states: *"This crate is the engine contract that the commercial products
   build on: Xberg Pro and Xberg Enterprise each implement `VectorStore` externally (single-node
   embedded store, tenant-scoped **pgvector**, …) while this crate stays single-tenant."*
   **Xberg Enterprise's own RAG backend is tenant-scoped pgvector** — not a bespoke vector
   database, not the in-memory/sqlite reference backends the OSS crate shipped. That is strong,
   direct evidence for hacienda's own choice, resolved in §7.

The recovered `lib.rs` is also useful as a design reference independent of the reuse question:
its `VectorStore` trait, `Filter`/`RetrieveQuery`/`RetrieveMode` IR, and typed
`SparseVector`/`MultiVector`/`DistanceMetric`/`IndexMethod` surface (hybrid dense+sparse+
late-interaction retrieval, matching the "Retrieval building blocks" entry in xberg 1.0.0's own
changelog) is a more mature starting shape for hacienda's `RagStore` trait than designing from a
blank page.

The integration spec's §10 Non-Goals lists "re-implementing xberg's Embed/Chunk/Cache" as
explicitly out of scope for the *CLI* — that non-goal does not extend to RAG-as-a-hosted-
capability, which was never evaluated in that spec because RAG wasn't in scope for it. This spec
brings it into scope for the first time.

### 3.6 xberg-core ships its own redaction + rehydration — deliberately not what hacienda uses

New finding, not in the integration spec: xberg 1.0.0's changelog lists "Redaction with
reversible rehydration and per-entity erasure" as a core feature, and the code is real —
`xberg/crates/xberg/src/text/redaction/{engine,rehydration,strategy}.rs`, gated behind Cargo
features `redaction` (pattern engine), `redaction-rehydrate` (adds `RehydrationMap`,
`encrypt_map`/`decrypt_map`, `find_subject`, `forget_subject`), and `redaction-ml` (couples in
NER). Read `rehydration.rs` directly: it is **not** the same construction as hacienda's own
`redaction/pseudonym.rs`. xberg's version encrypts an entire `HashMap<String, String>` (token →
original text) as one blob with AES-256-GCM under a **scrypt-derived passphrase key** — a
stored lookup table, protected as a unit. hacienda's AES-256-SIV construction (integration spec
§12.3) derives each token independently with no stored map at all, specifically so two nodes
redacting the same value with no coordination produce the same token.

These solve different problems and neither is a drop-in replacement for the other: xberg's
approach is simpler and fine for a single-writer, single-key document, but a shared
`RehydrationMap` blob is exactly the kind of shared mutable state §12.2's "stateless facade,
no sticky sessions" design was written to avoid — every redacting node would need to read *and
write* the same map, recreating the coordination problem pseudonymisation was supposed to solve
by being derived instead of stored. **hacienda should keep its own construction and not adopt
`redaction-rehydrate`** — but this should be an explicit, recorded decision (added as Decision
9 below) rather than tribal knowledge, since a future reader diffing hacienda against xberg's
feature list will otherwise reasonably ask why a built-in capability was reimplemented.

One small, separate finding while checking this: hacienda's `Cargo.toml` enables xberg's
`redaction` feature (`features = ["redaction", "ner", "tokio-runtime"]`), but
`grep -rn "xberg::text::redaction" hacienda-core/src/` returns **zero matches** — hacienda-core
never calls into it. The feature flag only unlocks `dep:sha2` transitively; harmless, but
worth a cleanup pass to confirm whether it's dead weight or a forgotten transitive requirement,
since carrying an unused feature invites exactly the "why is this here" confusion just raised
above about `redaction-rehydrate`.

### 3.7 hacienda-rag scope is smaller than §3.5 implies — most of the ML layer is already live in xberg-core

§3.5 established that the *orchestration/storage* layer (`xberg-rag`) is gone. Checking
`crates/xberg/src/` at the verified-current `v1.0.8`/`origin/main` (§3.5) shows the ML building
blocks that layer composed were never removed — they're separate, still-shipping public modules:

| Module | Gives hacienda-rag | Feature flag |
|---|---|---|
| `embeddings/` | Dense embeddings — ONNX, plus a pure-Rust model2vec fallback for WASM/no-ORT targets | `embeddings`, `static-embeddings` |
| `sparse_embeddings/` | SPLADE sparse vectors, for hybrid retrieval | `sparse-embeddings` |
| `late_interaction/` | ColBERT multi-vector / MaxSim | `late-interaction` |
| `reranking/` | Cross-encoder rerank — local ONNX, hosted (Cohere/Jina/Voyage via liter-llm), or caller plugin | `reranker` |
| `chunking/rag.rs` (`chunk_for_rag`) | Heading-aware chunking purpose-built for RAG, populates `heading_path` | always |
| `llm/text_completion.rs`, `llm/rerank.rs` | Hosted LLM completion + rerank via `liter-llm` | `liter-llm` |

hacienda-rag should depend on `xberg` with these features enabled and call `embed_texts_async`,
`chunk_for_rag`, `rerank_async` directly, exactly as the deleted crate's own `pipeline.rs` did
(its `pipeline-embeddings` feature wrapped `embed_texts_async` in a `CoreEmbedder`). This means
hacienda-rag's real scope is narrower than "rebuild a RAG engine" — it is the orchestration and
`VectorStore` contract only, recovered from `77e2fd3d71^` (§3.5's commit, confirmed the final,
most-evolved pre-removal state — it already includes sparse + late-interaction retrieval and
asymmetric query-prefix support, the last features added before deletion, so there is no newer
upstream version to recover instead):

| File (source) | Lines | Recover as-is / adapt |
|---|---|---|
| `store.rs` | 114 | As-is — the 8-method `VectorStore` trait (`ensure_collection`, `drop_collection`, `get_collection`, `upsert_document`, `delete_documents`, `delete_by_filter`, `retrieve`, `collection_stats`), documented single-tenant by design, matching hacienda's per-tenant capability model |
| `types.rs` | 318 | As-is — `CollectionSpec`, `ChunkRecord`, `DocumentRecord`, `RetrievedChunk`, `DistanceMetric`, `IndexMethod` (`Flat`/`Hnsw`/`Diskann` — `Diskann` anticipates pgvectorscale, not just plain pgvector) |
| `filter.rs`, `query.rs` | 368, 338 | As-is — filter/query IR, `RetrieveMode` (`Vector`/`FullText`/`Hybrid`/`Sparse`/`LateInteraction`) with capability-checked validation |
| `capability.rs` | 56 | As-is — per-backend `Capabilities` so unsupported modes are rejected up front |
| `pipeline.rs` | 420 | Adapt — ingest/retrieve orchestration; already wired to call the xberg-core modules listed above via feature flags |
| `backends/memory.rs` | 684 | As-is — brute-force in-memory backend, for tests |
| `backends/sqlite.rs` | 1,898 | Adapt, later phase — embedded `rusqlite` + `sqlite-vec` + FTS5 hybrid backend, native-only. Directly relevant to Phase 15's device-target: an embedded sqlite-vec store is the on-device RAG backend the mobile/offline story needs, and it is already-written rather than hypothetical |
| *(new)* `backends/pgvector.rs` | — | New — the cloud backend, against Decision 5's Postgres instance, implementing the recovered `store.rs` trait |
| `stream.rs` | 330 | **New concept, not yet in this spec** — streaming RAG *answer synthesis* over `liter-llm`: `AnswerEvent::{Token, Citation, Usage, Done, Error}` plus ingest/retrieval progress events. Everything in §4 today covers retrieval only (`/v1/rag/collections`, `/v1/rag/retrieve`); there is no answer-generation route or streaming concept anywhere in this spec. Needs its own design pass before Phase 12 — see updated Gap 3 in §9 |

---

## 4. Target API Surface

### 4.1 xberg-parity routes to add

All under `Capability::DocumentsProcess` unless noted; all responses carry
`AuditChain::tip()` per the integration spec's §5 response-shape rule.

| Route | Maps to | Notes |
|---|---|---|
| `GET /v1/jobs` | `list_jobs` | Paginated; filters by status |
| `GET /v1/jobs/{id}/result` | `get_job_result` | Distinct from `get_job` — result may be redacted-large, job status is not |
| `POST /v1/rag/collections`, `GET/DELETE /v1/rag/collections/{name}` | RAG collection CRUD | New `Capability::RagManage` |
| `GET/POST /v1/rag/collections/{name}/documents` | list/add documents | Documents added here go through `HaciendaFacade::process` first — **RAG never indexes unredacted text**; this is the one place Decision 1 of the integration spec (redacting proxy) most needs restating, since xberg's own RAG has no such constraint |
| `POST /v1/rag/collections/{name}/documents/{id}/reindex` | `reindex_rag_document` | |
| `POST /v1/rag/collections/{name}/retrieve` | `rag_retrieve` | New `Capability::RagQuery` — read access is a separate capability from write |
| `POST /v1/rag/collections/{name}/migrate-embeddings` + job lookup | embedding migration | Long-running; uses `JobStore` |
| `GET/PUT /v1/rag/config` | rag config | Per-tenant, not global |
| `POST /v1/auth/keys`, `DELETE /v1/auth/keys/{id}`, `GET /v1/auth/config` | key issuance | New `Capability::AuthManage`; see §6 |
| `GET/POST/DELETE /v1/presets` | saved presets | Extraction-option presets, scoped per tenant |
| `GET /v1/documents/{id}/versions`, `GET /v1/documents/{id}`, `GET /v1/documents/{id}/diff`, `GET /v1/documents/{id}/diff/{diff_job_id}` | versioning/diff | Exact shape now known, not assumed — see §4.3 |
| `POST /v1/uploads/presign`, `POST /v1/uploads/confirm` | presigned uploads | See §5 for the SSRF-safety argument this reopens |
| `GET /v1/usage` | metering | Read-model over `AuditStore`, per Decision 3 |

### 4.2 hacienda's own routes still owed (integration spec Phase 5, unshipped)

| Route | Capability |
|---|---|
| `POST /v1/pii/reveal` | `Capability::PiiReveal` — **highest priority, see §3.4** |
| `GET /v1/audit`, `GET /v1/audit/verify` | `Capability::AuditRead` / `AuditExport` |
| `GET/POST /v1/review`, `POST /v1/review/{id}/decide` | `Capability::ReviewDecide` |
| `GET /v1/compliance/dpia`, `GET /v1/compliance/report` | new capability, undefined in current `Capability` enum — needs its own decision, not assumed |
| `GET/POST /v1/glossary` | existing `observe_glossary` path in the facade, no route yet |

### 4.3 New concept required: document versions — now specified against the real API, not guessed

Nothing in hacienda-core currently models "this document was processed twice, here is the
diff." This is new state (§7 below), not a route wrapping something that already exists —
and unlike the first draft of this spec, the shape doesn't need to be invented: it's fully
documented in `xberg-sdks/CHANGELOG.md` `[0.3.0]` (2026-06-01), added to track "the cloud's
`1.1.0` API additions backing design-partner Asks #47/#48/#49/#51":

- Submission carries an optional client-supplied `document_id` (UUID) on the JSON body and
  per-file multipart fields.
- The response envelope then adds `document_id` + `version_sequence` (1-based, **server-assigned
  via `MAX(version_sequence)+1`** for that `document_id`).
- **Idempotency, not just versioning:** re-uploading identical bytes under the same
  `document_id` returns the existing job — no new row. Distinguishing "same bytes, same id" from
  "new bytes, same id" is the actual hard part of this feature, not the diff computation.
- `GET /v1/documents/{id}/versions` — paginated, newest first.
- `GET /v1/documents/{id}` — latest version envelope, extraction result inline.
- `GET /v1/documents/{id}/diff?from=&to=` — pairwise structural diff, **sync by default with a
  2-second in-handler budget**; over budget, returns `202 Accepted` + `diff_job_id` instead of
  blocking. This sync/async split is a deliberate API design worth copying as-is: most diffs
  are small and a caller shouldn't have to poll for them, but a diff on two large documents
  must not hold an HTTP handler open indefinitely.
- `GET /v1/documents/{id}/diff/{diff_job_id}` — polling for the async fallback.

Do **not** confuse this with xberg-core's own `revisions` field on `ExtractionResult`
(`xberg/crates/xberg/src/types/revisions.rs`) — that surfaces track-changes metadata *embedded
inside a single document* (DOCX `w:ins`/`w:del`, ODT `text:change-*`, PDF xref chains), a
within-document format feature, unrelated to and no help for building this service-level
across-submissions versioning concept. Confirmed by reading the type directly: `revisions.rs`
is intra-document diff-line/cell-change types, with no notion of a document identity persisting
across separate uploads.

For hacienda, this means the content-addressing decision in §7's table (hash of *redacted*
output, not raw input) needs one more piece pinned before implementation: whether client-supplied
`document_id` (matching the real API) or hacienda-derived content-hash identity is the primary
key. The real API's choice — client-supplied, with server-assigned idempotent sequencing — is
the one to mirror unless there's a hacienda-specific reason not to, since it's what any ported
client code will already assume.

---

## 5. Presigned Uploads Reopen an SSRF Decision

The integration spec's `hacienda-api-cli-surface` convention states document input is
"base64 inline bytes only — never accept a wire-supplied path or URL." Presigned uploads do
not violate this: the client PUTs bytes directly to object storage out-of-band (xberg's own
pattern, confirmed in the Dart SDK's `presignUpload`/`confirmUpload` pair), and hacienda's
`confirm_upload` handler fetches from **hacienda's own storage bucket by a server-issued key**,
never from a client-supplied URL. This is not the SSRF shape the original rule was written
to block — the rule concerned a wire-supplied *arbitrary* URL, not a presigned key hacienda
itself generated. Restated explicitly so a future reader doesn't misread §5 of the integration
spec as forbidding this.

---

## 6. Auth: API Key Issuance

`hacienda-core/src/auth/authz.rs` enforces capabilities given a token; nothing mints one.
Minimum viable shape, deliberately smaller than OIDC:

- `POST /v1/auth/keys` — issues an API key bound to a `CapabilitySet` and a tenant ID.
  Requires an existing `AuthManage`-capable key (bootstrap key provisioned out-of-band, not
  over HTTP).
- Keys are hashed at rest (never store the raw key — same principle as the pseudonym key,
  §12.3.2 of the integration spec: "the highest-value secret... sourced from a KMS... never
  from the config file").
- `DELETE /v1/auth/keys/{id}` revokes. Revocation must be visible to every replica within
  one poll interval — this is the first piece of state in this spec that is read on *every*
  request, making its cache-invalidation latency a correctness property, not a performance
  one. Options: short-TTL cache with active revocation push, or a Postgres lookup per
  request behind a connection pool (simpler, correct-by-construction, revisit if latency
  data says otherwise — do not pre-optimize this without a measurement, per the integration
  spec's own Phase 2/6 gating precedent in §9).

No `login`/session/cookie flow — hacienda is machine-to-machine only. If a future browser
client needs interactive auth, that is a distinct spec, not an extension of this one.

---

## 7. State Inventory — Extends Integration Spec §12.2

| State | Backend | Scaling note |
|---|---|---|
| RAG collection metadata | Postgres | Small, relational — collection name, config, tenant |
| RAG vector/BM25 index | **pgvector on the Postgres instance from Decision 5** (§3.5, Decision 2) — mirrors Xberg Enterprise's own tenant-scoped pgvector backend, evidenced by the deleted `xberg-rag` crate's doc comment. No per-replica in-process index, no external vector-store product to operate. Scaling note: pgvector index maintenance (HNSW/IVFFlat rebuilds) is a Postgres-ops concern, not a hacienda-service concern — track it alongside Decision 5's Postgres capacity planning, not as separate state |
| Document versions | Postgres (Phase 9, shipped) | Content-addressed by hash of redacted output, not raw input — versioning raw content would defeat Decision 1 |
| Presets | Postgres (Phase 9, shipped) | Per-tenant, small |
| API keys | Postgres (Phase 9, shipped) | Hashed; revocation-latency property per §6 |
| Usage/metering | **Derived, not stored** (Decision 3) — a query over `AuditStore`, so it inherits whatever backend §12.6 gives audit (Postgres) with no separate migration path |

This table is deliberately a superset of the integration spec's §12.2, not a replacement —
segments/review/glossary/pseudonym-keys/adapter-cache rows there are unchanged.

---

## 8. SDK Repository Plan

**Superseded 2026-08-05: monorepo (`sdks/{python,typescript}` in this repo), not a new
`hacienda-sdks` repo.** The session's GitHub App integration cannot create repositories
(`403 Resource not accessible by integration`) — rather than gate on an org-level
permission grant, the SDK packages live under `sdks/` here instead. This also removes
the cross-repo `spec-sync` workflow this section originally implied: CI builds
`hacienda-cli`, starts it, and fetches `/openapi.json` in the same job that generates
and tests the client, always against the exact commit under test — no vendored spec,
no cross-repo token. The rest of this section (dual-target axis, language scope, the
`_resolve_tier` correction below) still describes the actual design; only "new repo,
structurally mirroring `xberg-sdks`" is no longer accurate — read that as "new
top-level directory," not "new repo."

~~New repo, `hacienda-sdks`, structurally mirroring `xberg-sdks`~~ (confirmed structure:
`packages/{python,typescript,go}` + generated-core/hand-wrapper split, `CONTRIBUTING.md`'s
`scripts/sync-versions.py` + root `VERSION` + unified-tag release process) — see correction
above; `sdks/{python,typescript}` here, `scripts/sync-versions.py` + `VERSION` unchanged in
spirit.

**What changes from the xberg-sdks template:**

- The dual-target axis is `target: "cloud" | "device"`, not `"enterprise" | "pro"`. Cloud
  talks to `hacienda-api` over HTTPS. ~~with the tier-probe pattern from `client.py`'s
  `_resolve_tier` (adapted: hacienda has one tier, so this becomes a capability-probe against
  `GET /v1/auth/config` rather than a tier-probe against `/healthz`)~~ **Corrected
  2026-08-05:** `GET /v1/auth/config` requires `Capability::AuthManage`, which a normal SDK
  caller does not hold, so it cannot serve as that probe. `GET /v1/auth/whoami` (added
  specifically for this) reports the calling principal's own granted capabilities instead,
  gated on `Capability::DocumentsProcess`; both `HaciendaClient`s expose it as `.whoami()`.
  Device wraps the Cactus FFI directly — no HTTP client underneath at all for that target.
- Method surface is generated from hacienda's OpenAPI spec for `target: "cloud"` only. There
  is no OpenAPI equivalent for the device target since it's a native FFI call, not a REST
  endpoint — that half of each language package is hand-written, not codegen'd, same as how
  `xberg-sdks`' Dart `_generated/` tree coexists with hand-written `client.dart` wrapper logic
  today.
- **Precondition, not implementation detail:** codegen cannot start until hacienda-api's
  `/openapi.json` actually describes the routes in §4 with real schemas. The current 10-route
  surface has never been checked for utoipa annotation completeness against what
  `openapi-python-client`/`oapi-codegen` require — this needs a verification pass before Phase
  9 begins, not an assumption that it's already codegen-ready.

**Language scope:** start with Python + TypeScript only (matches the two most complete
xberg-sdks packages); Dart is the eventual bridge to the Cactus mobile target and should
follow once `target: "device"` has a working prototype in one language, not be built in
parallel with an unproven device path.

---

## 9. Blocking Gaps, Ordered by Severity

1. **`POST /v1/pii/reveal` is missing.** A shipped, correct crypto primitive
   (`pseudonym.rs:520-546`) with no caller. Blocks nothing else, but every day it's absent is
   a day pseudonymisation is decorative. No dependencies — buildable now.
2. ~~**No Postgres backend for any store.**~~ **Closed by Phase 9.** Six Postgres-backed store
   implementations now exist (`hacienda-core/src/store/postgres/`: audit/segments, review,
   jobs, document_versions, presets, api_keys), backed by the embedded schema in
   `hacienda-core/migrations/0001_init.sql`, a testcontainers-based disposable-Postgres test
   fixture (`store/postgres/test_support.rs`), and a `with_stores`-style builder extension on
   `HaciendaFacade` for wiring them in place of the in-memory defaults. This unblocks running
   more than one `hacienda-api` replica and gives §7's Postgres-backed rows (versions, presets,
   keys) somewhere to live.
3. **RAG route existence on the hosted service is unconfirmed (§3.1, §3.5); the recovery plan
   for everything else is not (§3.7).** The backend *architecture* is evidenced — pgvector on
   Postgres, per §7 and Decision 2 — and the `VectorStore` trait/types/filter/query IR to build
   `RagStore` against has a concrete, file-by-file recovery source (§3.7). What remains open is
   whether xberg-sdks's RAG method group (`collections.*`, `retrieve`, etc.) reflects routes
   that actually exist and are documented on the hosted Enterprise service, or names inherited
   from the pre-removal `xberg-rag` crate with weak/no changelog corroboration (§3.1's
   evidence-strength table). Confirm route existence against the live OpenAPI spec before
   building `/v1/rag/*` — the risk is a wasted CRUD implementation, not an architectural
   rewrite. Separately, **answer synthesis (streaming LLM answers over retrieved chunks) has no
   design in this spec at all** — the recovered `stream.rs` (§3.7) is the only evidence such a
   feature was ever planned upstream; decide whether hacienda ships it before Phase 12 locks
   the `/v1/rag/*` route shape, since adding it later would be an additive route, not a rewrite,
   but the citation/event schema is easier to get right once than to bolt on.
   **Partially closed:** the `RagStore` trait, backend-agnostic types/filter/query IR, and an
   in-memory reference backend (`crates/hacienda-rag`, recovered from `xberg-rag`'s final
   pre-removal source) shipped per
   `superpowers/plans/2026-08-01-hacienda-rag-vector-store-layer.md` — 49 tests, `cargo clippy
   -p hacienda-rag --all-targets -- -D warnings` clean. A durable `PgVectorStore` backend
   (`crates/hacienda-rag/src/backends/pgvector.rs`, gated behind a new `postgres` Cargo feature)
   also shipped per Phase 12 Task 2 — 57 non-live + 19 live (real pgvector/Postgres,
   `--ignored`) tests passing. Route existence against the real, CI-synced `xberg-sdks` OpenAPI
   spec was confirmed (Phase 12 Task 3 Step 1) and the answer-synthesis scope question was
   explicitly decided: not built in Phase 12 (Task 3 Step 4) — no upstream route exists to build
   a contract against. **Known deployment issue, now fixed:** `hacienda-rag`'s and
   `hacienda-core`'s sqlx migrations were both numbered `0001`; sqlx's `_sqlx_migrations` table is
   one-per-physical-database, so running both crates' migrations against the same database
   produced `VersionMismatch(1)`. Resolved by renumbering `hacienda-rag`'s migrations into a
   reserved `0100-0199` range (`hacienda-core` keeps `0001-0099`) — see
   `hacienda-core/migrations/README.md` for the permanent numbering convention and
   `crates/hacienda-rag/src/backends/pgvector.rs`'s
   `live_tests::should_run_both_crates_migrations_against_one_shared_database` for the
   regression proof. A production hacienda-rag deployment may now share one database with
   hacienda-core's stores. The 5 of 8 confirmed `/v1/rag/*` routes that map onto an existing `RagStore`
   method (create/get/delete collection, upsert document, retrieve) are now built in
   `hacienda-api` (Phase 12 Task 3 Steps 2/3/5/6), gated on `Capability::DocumentsProcess`, 400
   when no store is configured, with 4 route tests passing. ~~**What remains open:** the other 3
   confirmed routes — list-collections, list-documents, and migrate-embeddings (plus the
   RAG-specific use of the jobs-poll route) — have no `RagStore` trait primitive to serve them;
   building them requires extending the trait first, which was out of scope for this task.~~
   **Closed — found already shipped, 2026-08-05.** Verified directly against source while
   preparing Phase 14: `RagStore` (`crates/hacienda-rag/src/store.rs`) now has
   `list_collections`, `list_documents`, `set_embedding_provenance`, `get_document_chunks`,
   and `update_chunk_embeddings`, and `hacienda-api/src/handlers/rag.rs` implements
   `list_collections`, `list_documents`, `migrate_embeddings` (with a background job via
   `run_migrate_embeddings_job`/`migrate_embeddings_work`), and `get_migrate_status` —
   `ROUTE_TABLE` has all 8 confirmed `/v1/rag/*` routes plus the migrate-embeddings job-poll
   pair, none of the previously-open 3 remain missing. This was not tracked in a checked-off
   plan step anywhere in `2026-08-01-platform-parity-and-scale-implementation.md`'s Phase 12
   section — the code moved ahead of both this spec and that plan's checkboxes, apparently in
   the "Phase 12 Tracks 1-3" merge (`a0b1b84`) this spec's own tracking predates. Additionally,
   **streaming answer synthesis now exists** —
   `POST /v1/rag/collections/{name}/answer` (`hacienda-api/src/handlers/rag_stream.rs`,
   Server-Sent Events over `hacienda_rag::answer_stream`) — despite §9 Gap 3's own text below
   and the Phase 12 plan's Task 3 Step 4 recording a decision *not* to build it ("no upstream
   route exists to build a contract against"). Whoever built it evidently revisited that
   decision without updating either tracking document. This is a correction, not new scope:
   Phase 14 SDK generation must cover this route (and its 3 previously-"open" RAG routes) as
   already-shipped surface, not as future work.
4. ~~**No auth-key issuance.**~~ **Closed by Phase 11.** `Capability::AuthManage`,
   `HaciendaFacade::{issue_key_with_auth, revoke_key_with_auth}`,
   `auth::authn::ApiKeyTokenResolver`, and `POST /v1/auth/keys` /
   `DELETE /v1/auth/keys/{id}` / `GET /v1/auth/config` are implemented and tested
   end-to-end (issue → authenticate → revoke → re-authenticate fails), against both
   `InMemoryApiKeyStore` and `PostgresApiKeyStore`. Revocation-latency caching (Phase 11
   Task 4) was deliberately deferred — no measurement yet shows uncached per-request
   resolution is a real bottleneck.
5. ~~**Phase 5 (audit/review/compliance/glossary routes) still unshipped**, per the integration
   spec's own unchanged file comment (§3.2).~~ **Closed by Phase 10.** `GET /v1/audit`,
   `GET /v1/audit/verify`, `GET /v1/review`, `POST /v1/review/{id}/decide`,
   `GET /v1/compliance/dpia`, `GET /v1/compliance/report`, `GET /v1/glossary` are implemented
   (`hacienda-api/src/handlers/audit_review.rs`) and route-table-reflected
   (`every_guarded_route_reflected_in_auth_state`, `route_table_has_no_duplicate_paths` both
   pass), backed by two new facade accessors (`glossary_snapshot_with_auth`,
   `compliance_report_with_auth`, both tested). CLI parity re-check (Phase 10 Task 3)
   confirmed the CLI's own audit/review/compliance/glossary subcommands remain deliberately
   absent — no change to that judgment. **Bug found and fixed while closing this gap:**
   writing the missing two-capability test (Task 2 Step 3) found that `GET /v1/review`
   actually required `review:decide` (via a facade call shared with `decide_review`)
   instead of the route table's declared `audit:read` — fixed with a new
   `review_queue_read_with_auth` facade method scoped to `audit:read`; see CHANGELOG's
   `Fixed` entry. **Known gap, still not closed:** the route suite has not been re-run
   with `PostgresAuditStore`/`PostgresReviewStore` wired into the test `ApiState` (Phase 10
   Task 2 Step 5) — the routes call facade methods only, so this is expected to be a
   formality, but it is unverified and requires a live Postgres this environment does not
   have running.
6. ~~**hacienda-sdks repo does not exist.**~~ **Closed by Phase 14 (monorepo shape),
   2026-08-05.** `/openapi.json` was not merely "unverified" — it was a hand-built stub
   with no HTTP methods, no schemas, and no operations at all (`build_openapi()` emitted
   one `{"description": "Access: ..."}` object per path). Replaced with a real OpenAPI 3.1
   document generated via `utoipa` (`#[derive(ToSchema)]` on all 59 `hacienda-api` DTOs,
   `#[utoipa::path]` on all 44 route-table handlers), verified end-to-end by running
   `openapi-generator-cli generate -g python` and `-g typescript-fetch` against a live
   instance's `/openapi.json` — both produced complete, syntactically valid typed clients.
   Added `GET /v1/auth/whoami`, correcting §8's `_resolve_tier` design (see §8's own
   correction). The SDK repo itself does not exist — per §8's correction, `sdks/{python,
   typescript}` live in this repo instead (GitHub App repo-creation permission gap), fully
   built: hand-written `HaciendaClient`/`AsyncHaciendaClient` (Python) and `HaciendaClient`
   (TypeScript) covering all 44 operations across 14 tags, 12 pytest + 7 vitest tests
   against a live `hacienda serve`, `ci-sdk-python.yaml`/`ci-sdk-typescript.yaml` triggered
   on schema changes, `publish-sdk.yaml` scaffolded (not activated — needs org-level PyPI/npm
   trusted-publishing). See the implementation plan's Phase 14 for the full record.
7. **Cactus device-target spike not run.** Whether `cactus convert` handles GLiNER2's
   span-classification head is still unverified (noted in the prior conversation's gap
   analysis); §8's language-scope decision to defer Dart/device explicitly avoids blocking
   Python/TypeScript SDK work on this unresolved question.

---

## 10. Phasing

Continues the integration spec's §9 table (Phases 0-7); phase numbers below start at 8 to
avoid renumbering shipped work.

| Phase | Deliverable | Closes | Gated on |
|---|---|---|---|
| 8 | `POST /v1/pii/reveal` | Gap 1 (§9 above) | — |
| 9 | Postgres store backend: segments, review, jobs (completes integration spec §8 gap 4/§12.6), plus new tables for versions/presets/keys — **done** | Gap 2 | — |
| 10 | Phase 5 from the integration spec: `/v1/audit/*`, `/v1/review/*`, `/v1/compliance/*`, `/v1/glossary` — **done** | Gap 5 | Phase 9 (needs a durable review/audit store to be worth exposing over HTTP) |
| 11 | `/v1/auth/keys` issuance + revocation — **done** | Gap 4 | Phase 9 |
| 12 | Task 1 (`RagStore` trait + types/filter/query IR + `InMemoryVectorStore`) — **done**, `crates/hacienda-rag`; Task 2 (`PgVectorStore` backend, `postgres` feature) — **done**; Task 3 (route existence confirmed, answer-synthesis scope decided: not built, 5 of 8 confirmed `/v1/rag/*` routes built and tested) — **done**; 3 routes (list-collections, list-documents, migrate-embeddings) remain unbuilt — no `RagStore` trait primitive serves them | Gap 3 (mostly closed — see §9) | Phase 9; backend architecture and trait shape already decided (§7, §3.7, Decision 2) — this phase is route verification + implementation, not design |
| 13 | `/v1/jobs` list + result, presets, versions/diff, presigned uploads, `/v1/usage` — **done** | — | Phase 9; usage additionally needs Phase 10's audit routes as its read-model source |
| 14 | `sdks/{python,typescript}` (monorepo, not a separate `hacienda-sdks` repo — see §8), `target: "cloud"` only — **done** | Gap 6 | Phases 8, 10, 11, 12, 13 (needs a stable, complete OpenAPI surface — this is why SDK work is last, not first) |
| 15 | Cactus device-target spike (GLiNER2 span-head via `cactus convert`) + `target: "device"` prototype in one language | Gap 7 | Phase 14's SDK scaffolding, but not on any cloud-route phase |

**Why Postgres (Phase 9) gates almost everything:** every new capability in §4 except the
RAG index either needs relational storage directly or needs auth keys to gate it, and auth
keys need relational storage too. Sequencing SDK work (Phase 14) after the routes it
generates against, rather than in parallel, mirrors the integration spec's own Phase 2-before-4
rationale: building against a moving target guarantees a regeneration, same as building a CLI
against an unbuilt capability model would have.

---

## 11. Open Risks

- **RAG-in-hacienda is new product surface, not just new plumbing — and its upstream route
  surface may not exist yet, even though the backend architecture and trait shape are now
  settled.** Unlike the audit/review work, which formalizes something `HaciendaFacade` already
  does, RAG-over-redacted-content has no existing implementation to wrap. The backend question
  is resolved (§3.5, §7, Decision 2: pgvector on Postgres, mirroring Enterprise's own
  tenant-scoped pgvector), and the `VectorStore` trait/types/filter/query IR has a concrete
  recovery source (§3.7) — re-verified against the official remote at its true latest state
  (`v1.0.8`/`origin/main`, not the pinned `v1.0.2`) and against the whole `xberg-io` org (no
  standalone RAG repo exists), so the absence is confirmed current, not a stale-checkout
  artifact. What is **not** resolved is whether the SDK's RAG *routes* exist server-side: `xberg-
  rag` was never a live dependency to verify hacienda's design against the way the integration
  spec's §3 verified xberg's CLI/HTTP surface, and §3.1 found the RAG method group is the one
  xberg Enterprise SDK group with **zero corroborating changelog entry**, unlike every other
  group — and the SDK has shipped dead methods before (the sandbox-key removal). Phase 12 must
  start by confirming the RAG routes respond at all before building hacienda's collection model
  against them — treat "does `POST /v1/rag/collections` exist server-side" as an open question,
  not a premise. Additionally, the recovered source surfaces a feature this spec has no design
  for at all: streaming answer synthesis (`stream.rs`, §3.7) — worth an explicit scope decision
  before Phase 12, not a silent omission.
- ~~**Usage-as-a-read-model (Decision 3) assumes audit entries carry enough detail to bill
  from.** Not yet checked against `AuditChain`'s actual entry schema — if entries don't carry
  a billable unit (bytes processed, entity count, etc.), Decision 3 needs revisiting before
  Phase 13, not after.~~ **Resolved in Phase 13 Task 5:** `AuditEntry` carries `principal`
  and `span_length`, both attributable and summable per-principal and time-windowed — entity
  count and byte count are real billable units. Document count is *not* derivable (no
  `document_id` on the entry) and is deliberately omitted from `GET /v1/usage` rather than
  guessed at.
- **The device-target SDK method surface (§8) is speculative until Phase 15's spike lands.**
  Everything in this spec's SDK section describing what device-mode "can't do" (multi-tenant
  RAG collections, segment reconciliation) is a reasonable inference from Cactus's C FFI
  surface as read during the earlier architecture investigation, not a constraint verified
  against a running prototype.
