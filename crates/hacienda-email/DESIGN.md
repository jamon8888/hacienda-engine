# hacienda-email — design decisions (email-plugin-completion branch)

## Goal
Make the email module behave like folder-based document extraction/indexing on the
same hacienda base, so users can connect their Gmail/Outlook and still honour
professional secrecy (secret professionnel).

## Confirmed decisions

### D1 — Both redaction and reversible pseudonymisation, per-tenant choice
The plugin offers two privacy modes, selected per tenant/caller (option 3 chosen
by the user):
- **redact** (default, fail-closed): PII is masked before anything is persisted
  or indexed; plaintext never leaves the client.
- **pseudonymize**: PII is replaced by reversible tokens minted under a
  per-tenant key; reveal is a separate, audited `HaciendaFacade` capability.

The choice lives in config and is enforced at ingestion (not at query time), so
the wrong mode cannot leak plaintext into a vector store.

### D2 — Mandatory redaction before RAG indexing (E4 ingestion guard)
`rag.rs` must run PII redaction on every email body **before** chunking/embedding.
If the PII engine fails, indexing is refused (fail-closed). This closes the
pre-existing E4 gap named in the workspace `Cargo.toml` ("ingestion-time
redaction guard ... doesn't exist yet for the vector-store embedding path").

### D3 — Real hacienda-core / hacienda-rag / xberg APIs only
No invented APIs. PII work goes through `HaciendaFacade` (real detection, audit
chain, pseudonymisation). Chunking goes through `hacienda-rag::chunk_full_text`.
Attachments are extracted with the pinned xberg `extract()` /
`ExtractInput::from_bytes()`.

### D4 — OAuth/XOAUTH2 for Gmail & Outlook
Users connect accounts via OAuth tokens; `pimalaya-stream 0.0.1` exposes
`SaslXoauth2 { username, token: SecretString }`, so IMAP + XOAUTH2 works for
`imap.gmail.com` and `outlook.office365.com` without a separate graph client.
Refresh-token handling and encrypted token storage are part of this work.

### D5 — CLI first; HTTP/MCP surfaces require tenant-scoped auth
The CLI is the primary user surface (a locally spawned process is `Caller::Trusted`,
matching hacienda's existing precedent). The HTTP API and MCP server are
network-reachable: exposing a shared `EmailProcessor` there would let one tenant
read another's mail or trigger redaction under the wrong privacy mode. Wiring
those surfaces therefore requires building an `EmailProcessor` **per request from
the authenticated tenant's config** (`AuthContext`/`TenantCtx` already extracted by
`hacienda-api/src/handlers/mod.rs`), never a process-wide trusted instance. This
is tracked as the final Phase-5 step, not skipped.
