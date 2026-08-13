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
