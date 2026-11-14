# hacienda-api — the audit surface

This crate serves hacienda's HTTP API. This README covers **what the audit endpoints prove**,
because that is the part most easily mistaken for something it is not.

## The chain is not an activity log

Most products that expose "audit" expose an activity log: who called what, when. The entry looks
like this, and it is what Xberg Enterprise's `GET /v1/audit` returns:

```json
{ "id": "...", "actor": "api_key_7f3", "action": "job.submit",
  "resource_type": "job", "created_at": "..." }
```

That answers *"which operations ran"*. It does not answer *"was this personal value actually
redacted, by which detector, under which configuration — and who has read it since"*, because
nothing in it is tied to the content.

hacienda writes **one entry per redacted span**, and each entry is linked into a blake3 hash
chain:

```json
{ "id": "...", "category": "Email", "action": "Mask",
  "span_hash": "5e884898da28047151d0e56f8dc629...",
  "span_length": 16, "confidence": 1.0, "source": "Regex",
  "pipeline_version": "0.1.0", "config_hash": "...",
  "principal": "api_key_7f3", "chain_hash": "..." }
```

Three properties follow, and each is testable rather than asserted:

**It is verifiable.** `GET /v1/audit/verify` recomputes the chain and names the first entry or
seal that does not match. `GET /v1/audit/export` produces an envelope you can verify **without
the server** — no trust in this process is required to check that a record was not altered
after the fact.

**It joins redaction to disclosure.** The `span_hash` written when a value is redacted is the
same digest written when someone later reveals it under `pii:reveal`. An auditor can therefore
answer *"this value was redacted here — and this principal read it there"*, which is the
question GDPR Art. 30 and AI Act Art. 12 actually ask.

**It contains no personal data.** Entries carry a blake3 digest and a length, never a value.
A test processes a control corpus and then queries every route in the table, asserting no
response contains any corpus value.

## Two artefacts, and only one of them is evidence

`GET /v1/audit/export` serves both. They are not interchangeable:

| `?format=` | What it is | Verifiable offline |
| --- | --- | --- |
| `json` *(default)*, `jsonl` | **Evidence envelope** — entries grouped by segment, with seals | **Yes** |
| `csv` | Tabular extract, for a spreadsheet or a SIEM | **No** |

A CSV cannot be made verifiable by adding columns. Verifying an entry needs its predecessor's
hash and its sequence number; each segment restarts its chain at genesis with the sequence back
at zero, and a flat table has no way to say where. The envelope's grouping is what makes both
recoverable — it is structure, not presentation.

The response says which one you got: `x-hacienda-audit-evidence: envelope | none`, and the CSV
downloads as `hacienda-audit-extract-NOT-EVIDENCE.csv`. The filename carries the warning because
headers disappear the moment a file is saved, and the mistake happens later — when someone
attaches a file out of their downloads folder for a regulator.

## Scope: this node, not the deployment

Every response carries `"scope": "this_node"`. Each writer owns its own segment directory and
its own seal chain; there is **no defined total order across nodes**. Combining several nodes'
records means merging their seal lists and verifying each node's subsequence separately.

Saying "the audit entries" while meaning "this node's" is the same class of error as an endpoint
that returns one segment while claiming to return the history — an auditor reads absence where
events exist. Hence the field, and hence the type being named `NodeAuditPage`.

## Paging

Cursor-based, never offset: the chain only grows, so an offset drifts mid-pagination and an
auditor silently skips a record.

**Page until you receive an empty page** — not until `next_cursor` is null. A caller that has
caught up still holds a resumable cursor, which is what makes the log tailable. `limit=0` is
refused with 400 rather than served, because an empty page with no cursor is byte-identical to
"you are caught up", and an uninitialised paging variable would read as an empty history.

Filters are not implemented and are refused with 400 rather than ignored. Filtering above the
cursor would yield empty pages while unread entries remain, and a paginating auditor would stop
early without knowing.

## Capabilities

| Route | Requires |
| --- | --- |
| `GET /v1/audit/entries`, `/verify`, `/seals` | `audit:read` |
| `GET /v1/audit/export` | `audit:export` |
| `GET /v1/audit/tip` | `documents:process` |

`tip` is deliberately the exception. It is an opaque hash that reveals nothing about the entries
behind it, and gating it would stop a caller holding `documents:process` from obtaining the
chain evidence for **its own** result.

## Anthropic Messages gateway

`POST /v1/messages` implements a restricted Anthropic Messages gateway for callers that attach a
`ClaudeMessagesProvider` to `ApiState`. It is disabled by default, including in the standard
server state until that provider is configured. The route always requires an authenticated
principal with `documents:process`, even when the general API auth middleware is disabled.
Tenant routing comes from the authenticated principal; callers cannot select a tenant in the
request body.

Requests must use `stream: true` and the supported Messages schema. Inspectable text in messages,
system content, tool definitions, tool results, and the supported metadata field is redacted
before the provider call. Unknown fields and blocks, images, files, and other unsupported
non-text content fail closed. Only the documented Anthropic routing headers are forwarded.
Provider events must match the validated event contract; text deltas and error messages are
redacted before they are exposed. Failures return generic client-visible errors.

The response uses SSE framing, but the current adapter collects and validates the provider's
complete event stream before returning it. It does not yet provide incremental end-to-end
delivery. Configure a Claude Code client only when that limitation is acceptable; incremental
relay remains an implementation requirement for production streaming.

## Anthropic Messages gateway

`POST /v1/messages` is the Anthropic-compatible gateway used by the Hacienda secrecy client and
Claude Code. It is disabled unless an application attaches a `ClaudeMessagesProvider` to
`ApiState`. The route always requires an authenticated principal with `documents:process`, even
when the API's general authentication middleware is disabled. Tenant routing comes from that
principal; the request body cannot select a tenant.

The gateway accepts streaming requests (`stream: true`) with a supported subset of the Messages
schema. It redacts text in user and system content, tool definitions and tool results, and
metadata before calling the configured provider. Images, files, unknown content blocks, unknown
fields, and unsupported non-text payloads fail closed. Only selected Anthropic routing headers
are forwarded. Provider responses must follow the validated Anthropic event contract; response
text and error messages pass through redaction before they are exposed. Authentication, invalid
request, provider, and redaction failures use generic client-visible errors.

The current HTTP response uses SSE framing, but the provider adapter collects and validates the
complete provider event stream before returning it. It therefore does **not** yet guarantee
incremental end-to-end delivery. Do not configure Claude Code against this route when progressive
streaming is required until the incremental relay is implemented. This boundary does not apply to
the Tauri client's direct provider transport.
