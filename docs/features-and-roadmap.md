# Hacienda features, status, and improvement proposals

**Verified against the repository on 2026-11-26.** This is the canonical product inventory. “Implemented” means code exists in this checkout; it does not mean every capability is enabled in every build or configured in every deployment. Feature flags, backend selection, credentials, and model assets affect what is available.

Historical plans and research remain useful context, but their proposed state may no longer match the product. Check this catalog and the implementation before relying on an older design document.

## Product shape

Hacienda is a document-processing and data-protection system built around Xberg extraction. Its Rust core is exposed through a facade, REST API, CLI, MCP server, WASM library, generated REST SDKs, and selected integrations. The workspace also contains a browser Studio and a Tauri desktop client.

The main processing path is extraction, PII detection, span merging, redaction, and audit. API and CLI entry points apply their own trust and output rules; the Studio uses the WASM pipeline locally in the browser.

## Document extraction and processing

- Xberg-backed extraction with the formats available in the selected build. `hacienda formats` reports the formats enabled by a specific binary; a single repository-wide format count would be misleading because features vary.
- MIME detection, cache statistics, cache clearing, and cache warming.
- Synchronous and asynchronous document processing, job listing/status/result routes, and result retrieval.
- Presigned upload initiation and confirmation routes for configured upload storage.
- Document version creation, listing, retrieval, diffs, and asynchronous diff-job polling.
- Configurable request-size limits, quotas, and usage aggregation.
- CLI extraction/scan can process multiple inputs and supports output, audit, vault, glossary, and concurrency options as defined by `hacienda --help`.

The API accepts document bytes or text in its request contract, not a caller-supplied local path or arbitrary URL. This prevents the API from acting as an unrestricted server-side fetcher. API response DTOs do not currently expose every Xberg extraction detail: page/table structure, rich metadata, quality scores, and complete extraction warnings are not all returned.

## PII detection and redaction

The built-in pattern set detects these categories:

- contact and network data: email, phone, IPv4/IPv6, MAC address, URL;
- financial data: IBAN, credit card, US routing number, SWIFT/BIC, EU VAT;
- identity and government identifiers: US SSN, US passport, US EIN/tax ID, driver's license;
- personal and medical data: date of birth, medical record number;
- other sensitive values: crypto wallet, vehicle VIN, API key, secret/password, JWT.

Some recognizers validate values, rather than accepting a shape match alone. This includes IBAN mod-97, card Luhn, VIN check digit, and Bitcoin Base58Check; structurally impossible US SSNs are rejected. Context words and confidence values are used by selected patterns. Regex detection works without a neural model. Neural NER is optional and depends on the enabled build feature and installed model assets.

Redaction modes are `mask`, `hash`, `pseudonymize`, `remove`, and `custom` (a configured replacement template). Pseudonymization is reversible with the correct key material, uses AES-SIV tokens containing a key ID, and supports key rotation. Custom terms and patterns are supported through core configuration; the API does not currently offer per-request mode selection or preset selection.

The reveal operation is separate from redaction and requires the `pii:reveal` capability. The MCP surface intentionally does not provide a reveal tool. Never treat `mask`, `hash`, or `remove` as reversible.

**Pending work:** PR [#181](https://github.com/jamon8888/hacienda-engine/pull/181) adds French checksum recognizers and `libphonenumber` phone validation. Those additions are not part of `main` until merged.

## Audit, review, and compliance

- Audit entries are hash-linked and can be listed, verified, sealed, paginated, and exported as JSON, CSV, or JSONL. Available persistence adapters depend on the component and build, including in-memory, file, browser IndexedDB, and PostgreSQL implementations.
- The human review queue supports pending/in-review/approved/rejected/modified states, reviewer assignment, decisions, and statistics. In-memory and persistent adapters are available where configured.
- Compliance functions produce DPIA, AI Act model-card and article/control checklist, DORA incident reports, and combined reports from supplied configuration and input data.
- A glossary endpoint and core linking support are present. Output features depend on the caller and configuration.

These are report-generation and evidence-storage features; generated artefacts do not independently certify a deployment or establish legal compliance.

## Authentication and API safeguards

- Capability-based authorization with a deny-by-default route map, API-key operations, caller identity, and tenant-scoped key behavior.
- Separate capabilities for operations such as document processing, audit reads, review actions, and PII reveal.
- Body-size limits and configurable quotas.
- OpenAPI 3.1 is available from `GET /openapi.json`; this is the authoritative live endpoint inventory.
- The default CLI server bind is loopback. A non-loopback bind requires authentication to be enabled.
- Claude-style `/v1/messages` and OpenAI/Codex-style `/v1/responses` and `/v1/models` compatibility surfaces exist, along with moderation/session/viewer routes. Their usable provider behavior depends on gateway and model configuration.

The central route table is the source for route registration, authorization reflection, and OpenAPI generation. Consult `hacienda-api/README.md` and the live schema for exact request/response contracts rather than copying a route count into documentation.

## RAG and knowledge retrieval

The RAG crate provides collection/document operations, chunking, embedding integration, retrieval, filtering, graph queries, embedding migration, and streaming answer synthesis with citations. Backends include in-memory, LanceDB, and PostgreSQL/pgvector, with graph persistence options including SQLite. Exact features depend on the configured backend:

- The in-memory vector store has no full-text index and rejects full-text/hybrid retrieval.
- LanceDB full-text capability depends on its full-text index being available for the collection/store.
- PostgreSQL/pgvector implements vector, full-text, and hybrid retrieval paths.
- Sparse and late-interaction modes are not supported by every backend.
- The `/answer` endpoint needs a routable language model and a configured PII pipeline.

**Security boundary:** RAG upsert is storage-oriented and does not automatically run the document redaction pipeline. Do not ingest raw sensitive text when the collection must contain redacted data. The answer route redacts the prompt and retrieved chunks before sending them to the configured model. The memory backend is useful for local/development use, but it is not a durable store.

## Email and legal research

The email crate implements IMAP (including OAuth/XOAUTH2 for Gmail and Outlook) and Maildir paths, plus archive processing, NER, redaction, indexing, and RAG integration. The CLI/API add async ingestion, search, and controlled team-email viewing. Gmail REST, Microsoft Graph, JMAP, M2Dir, and SMTP appear in configuration/features but are not currently implemented as working client backends; the email guide documents the exact runtime status. Enabled IMAP providers require their corresponding configuration and credentials.

The legal module and MCP tools support official-source search/fetch, asynchronous deep research, result status, and monitoring. Provider implementations cover EU, French, German, Dutch, Spanish, and Italian legal sources. Network access and source configuration are required.

## CLI, MCP, and API surfaces

The CLI offers extraction, scanning, configuration inspection, capability/format inspection, MIME/cache tools, HTTP serving, PII token reveal, audit verification/list/export, MCP serving, legal search/fetch/research/monitoring, review management, compliance reports, email processing, shell completions, and Claude/Codex installer commands. The exact command and option list is `hacienda --help`.

The MCP server exposes document and email processing, PII scan/redact, audit verification/entries, compliance reports, capability/format/MIME/cache helpers, and legal research operations. It deliberately does not expose PII reveal/rehydration.

The REST API groups endpoints for health/discovery, documents/jobs, email, legal research, PII, MIME/cache, audit, review, compliance, glossary, auth/keys, RAG/graph, presets, versions/diffs, uploads, usage, model-compatible messages/responses, moderation, and viewer operations. Use `/openapi.json` for the exact routes and schemas.

## Applications, SDKs, and integrations

### Hacienda Studio

The browser workspace runs document extraction and PII/redaction work through WASM and Web Workers. It includes document/editor flows, local review, entity linking/knowledge-graph views and exports. Its audit data is held in IndexedDB and remains within that browser profile; clearing browser data removes it. The application is in development and should not be represented as a finished, supported production product.

### Secrecy Client

The Tauri/React desktop app provides a conversation surface for Claude/Codex gateway workflows and a local viewer. Provider sending depends on successful local redaction and gateway configuration. Pseudonym reveal can be user-triggered in the local viewer. Model weights are installed separately rather than bundled with the app. The runtime does not currently provide every possible authorized original-document viewer reference or a document-save workflow.

### SDKs and framework integrations

- Python REST SDK: `hacienda-sdk`.
- TypeScript REST SDK: `@hacienda-engine/sdk`.
- Implemented LangChain adapters in Python and Node: loaders, retrievers, and document push helpers.
- LlamaIndex, CrewAI, txtai, n8n, and Spring AI directories are planned/scaffolded integrations without equivalent adapter implementations today.
- SDKs target the REST service; the documented SDK `target` is cloud. There is no official device/on-device SDK.

See [`sdks/README.md`](../sdks/README.md), [`integrations/README.md`](../integrations/README.md), and [`integrations/LIMITATIONS.md`](../integrations/LIMITATIONS.md).

## Persistence and operational limits

Persistence is adapter-specific. Jobs use in-memory storage unless a durable implementation is explicitly configured; in-memory audit, review, vector, and quota stores lose state on process restart. Check the deployment configuration rather than assuming durability from the existence of an API route. The CLI's audit list/export commands expect the durable file-store layout, while `--audit-out` produces a separate flat export intended for verification.

Presets currently have CRUD operations, but are not wired into `/v1/documents` or `/v1/pii/redact`. There is no per-request redaction mode in those request DTOs. RAG ingestion has the redaction boundary described above. LangChain wrappers do not currently wrap answer synthesis. See the integration limitations document for exact SDK/RAG behavior.

## Improvement proposals

These are proposals based on confirmed product gaps, not committed delivery dates. A project owner should prioritize them against user needs:

1. **Enforce redaction on every persistence boundary.** Add an explicit safe ingestion path for RAG/email and other stores, with tests proving raw PII cannot bypass configured policy.
2. **Connect presets to processing.** Add a validated per-request or named-preset contract and make API, SDKs, CLI, and integrations use the same semantics.
3. **Expose extraction detail.** Extend document results with page/table structure, metadata, quality signals, detected language, and warnings while retaining redaction guarantees.
4. **Make async work durable.** Provide persistent job storage, restart recovery, idempotency, cancellation, and retention controls for deployments that need them.
5. **Publish backend capability behavior.** Expose retrieval/store capability details and make clients choose supported modes; document durability and operational requirements by backend.
6. **Complete framework adapters.** Prioritize adapters from actual demand; LlamaIndex is the closest match to the existing LangChain shape. Add tests against a running Hacienda service.
7. **Finish Studio production readiness.** Define durable/exportable audit evidence, model installation/update handling, accessibility and browser support, and a support/security posture before declaring it production-ready.
8. **Close desktop workflow gaps.** Implement authorized original-viewer references and document save/export flows with clear trust boundaries.
9. **Improve observable operations.** Add structured metrics/health details, queue and backend diagnostics, and deployment guidance without exposing document content.
10. **Keep docs and contracts in sync.** Generate route/CLI/integration status tables from source where practical and add a documentation check for stale hard-coded counts and claims.

## Change status snapshot

On 2026-11-26, the repository had these relevant open changes; statuses can change and should be checked on GitHub:

- [PR #180](https://github.com/jamon8888/hacienda-engine/pull/180): generate Alef bindings without Dart.
- [PR #181](https://github.com/jamon8888/hacienda-engine/pull/181): French checksum recognizers and phone validation.
- [PR #182](https://github.com/jamon8888/hacienda-engine/pull/182): agent harness and codemap setup.

The current checkout may include an open PR branch. A feature belongs to `main` only after it is merged there.
