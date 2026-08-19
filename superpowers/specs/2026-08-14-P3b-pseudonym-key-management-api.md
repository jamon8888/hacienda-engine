# P3b — Pseudonym key management API surface

**Date:** 2026-08-14
**Status:** Implemented (2026-08-14)
**Extends:** `2026-08-01-P3-pseudonymisation-as-a-service.md` §3 ("Surface") — closes two of
its four listed routes.
**Depends on:** `2026-08-14-P3a-tenant-scoped-pseudonym-keys.md` (per-tenant
`Pseudonymiser`/`KeyResolver` — this spec's `list_keys`/`token` both resolve strictly the
caller's own tenant's material, the same discipline P3a established for `reveal`)

---

## 1. Problem

`Pseudonymiser` has no API surface at all beyond `reveal` — verified directly against
`hacienda-api/src/routes.rs`: `/v1/pii/reveal` exists, but neither `GET /v1/keys` nor a
token-minting endpoint does. P3 §3 names four routes; only one (`reveal`, under the
`/v1/pii/*` naming convention rather than the spec's proposed `/v1/pseudonyms/*` — an
already-made, unremarked naming decision this spec keeps) is implemented.

This spec closes two of the remaining three. The fourth (`POST /v1/keys/rotate`) is
explicitly deferred — see §4.

## 2. `POST /v1/pii/token` — mint without reveal (P3 §3, D-P3-1)

Computes the pseudonym token for a caller-supplied `(category, value)` pair, without
running detection and without disclosing anything the caller doesn't already know. This is
what makes a GDPR right-of-access/erasure request practicable against redacted storage
(P3 §2.3): compute the token for the value a data subject names, then search the corpus for
*that token*, never decrypting it.

**Capability: `documents:process` only — not `pii:reveal`.** D-P3-1's own reasoning:
value→token discloses nothing the caller doesn't supply; only token→value (`reveal`)
discloses, and only that direction requires the reveal capability. No audit entry is
written for the same reason `reveal` writes one and this doesn't: `reveal` records a
disclosure event; minting isn't one.

Resolves the token through `HaciendaFacade::pseudonymiser_for(&tenant)` — the same
reveal-only cache P3a built for `reveal_token_with_auth`, not `pii_pipeline_for`. Consistent
with P3a §7's finding: minting/revealing a token needs only key material, not a configured
`[pii]` detection pipeline, so this endpoint works under a key-resolver-only configuration
exactly like `reveal` does.

Errors: `HaciendaError::PiiDisabled` (no key resolver configured) → 503, matching the P3 §6
table's "clé connue mais non chargée" row (an operations problem, not a client error) —
reusing the mapping `reveal`'s `PiiDisabled` already gets. `PseudonymError::UnsupportedCategory`
(a `Custom` category name containing a token delimiter) → 400.

## 3. `GET /v1/keys` — identifiers and status, never material (P3 §3, D-P3-3)

Returns the caller's tenant's key ids and rotation status (`active` | `retired`) — no key
material, ever (D-P3-3, the same discipline `RedactionConfig::key_id` already enforces
everywhere else in this codebase).

**Capability: `audit:read`**, per P3 §3's table.

`Pseudonymiser` already tracks exactly this shape internally (`active: PseudonymKey`,
`retired: HashMap<KeyId, PseudonymKey>`) — added `Pseudonymiser::key_statuses()` returning
`Vec<KeyStatus>` (id + `active: bool`), built from `PseudonymKey::id()`, never touching
`material`. Resolved through the same `pseudonymiser_for(&tenant)` cache as `token` above.

## 4. Deferred: `POST /v1/keys/rotate`

**Not implemented by this spec.** P3 §4 describes rotation as additive — "a new key
becomes active; previous ones become `retired` and stay revealable" — but that presupposes
a `KeyResolver` whose active key can change at runtime. `EnvKeyResolver` (the only resolver
this codebase has) resolves the active key id from a process environment variable read at
`Pseudonymiser` construction time; there is no supported way to mutate that after the
process starts, per-tenant, without either:

- mutating process environment at runtime (unsound for a multi-tenant server — one
  tenant's rotation would need to change a variable another tenant's already-cached
  `Pseudonymiser` never re-reads), or
- inventing a second, mutable-at-runtime `KeyResolver` implementation now, ahead of the
  KMS-backed resolver `2026-08-01-hacienda-platform-parity-program.md` §4 (S2) already
  plans, which would then need to be thrown away or awkwardly retrofitted once S2 lands.

Rotation's natural home is S2's KMS-backed `KeyResolver` (Vault/AWS KMS), where "promote a
new active key" is a real backend operation, not a process-environment mutation this
codebase has no mechanism for. Tracked there rather than built twice. `GET /v1/keys` and
`POST /v1/pii/token` above have no such dependency — both are read-only/derive-only against
whatever `KeyResolver` is configured today.

## 5. Tests

| Test | Assertion |
| --- | --- |
| `mint_token_same_value_same_token_two_calls` | The founding determinism property, at the facade layer (P3 §7's `same_value_same_token_across_processes`, narrowed to this facade method). |
| `mint_token_requires_only_documents_process_not_pii_reveal` | A caller with `documents:process` and *not* `pii:reveal` can still mint. |
| `list_keys_reports_active_and_one_retired_status` | Status shape is correct for a resolver with a retired key configured. |
| `list_keys_response_contains_no_key_material` | D-P3-3, scanned at the HTTP DTO layer, not just the facade return type — mirrors P3 §7's `no_response_contains_key_material`. |
| `list_keys_and_token_resolve_the_callers_own_tenant` | Two tenants get independent key listings/tokens through the same facade, via P3a's `two_tenant_key_resolver` test fixture. |

## 6. Exit criteria

- `POST /v1/pii/token` and `GET /v1/keys` are live, documented in the OpenAPI document
  (S4's anti-drift tests catch a route added without one), and covered by the tests in §5.
- No response, log line, or error message from either route contains key material.
- `POST /v1/keys/rotate` remains unbuilt, with this spec as the recorded reason and the
  named blocking dependency (S2), not a silent gap.
