# GLiNER2 PII Taxonomy Alignment — 42-Label Privacy-Filter-Multi Design

**Date**: 2026-08-26
**Status**: Proposed — ready for implementation planning (Approach A approved via brainstorm)
**Author**: Muse Spark + human partner (brainstorm: 4 sections approved, visual companion)
**Scope**: `hacienda-core` PII taxonomy (`src/pii/types.rs`, `ner.rs`, `config.rs`, `audit/entry.rs`), `crates/hacienda-wasm` bridge, `apps/hacienda-studio` asset-loader, NER bridge, worker pipeline, detection modal, and redaction/export. Does **not** retrain GLiNER2 or change the mDeBERTa 512-token encoder.

> **Brainstorm decision**: `C: Support both` models (build-time select), `Hybrid` taxonomy (12 new enum variants + 30 alias→Custom), `Grouped collapsible` UI (7 groups), `Strict additive` wire compat, `Build-time select + fallback` routing, `Hybrid persona boost` thresholds.

---

## 0. Summary

`hacienda-studio` pins `jamon8888/gliner2-guardrails-pii-f16` (derived from `fastino/GLiNER2-Guardrails-PII-Multi`, ~600 MB F16) while the user-facing question references `fastino/gliner2-privacy-filter-PII-multi` (2026-05-11, 205 M F32, **42 labels** in 7 semantic groups). The Studio path exposes only 10 `NerCategory` (`lib/types.ts:98`) and 27 UI checkboxes (`lib/pii-categories.ts:55`) and a collapsed `PiiCategory` (33 variants + `Custom`, `types.rs:9`) with an incomplete alias table (`ner.rs:190`, `worker/pipeline.ts:460`). As a result, 20+ of the 42 labels are either dropped at the `config.nerCategories` filter (`worker/pipeline.ts:988`) or collapsed to opaque `Custom` without distinct redaction, audit, or export handling.

This spec aligns the full 42-label taxonomy **additively**: 12 new `PiiCategory` variants where audit/redaction diverges, remainder via an explicit 42→`PiiCategory` alias table, a 7-group collapsible detection modal, a build-time model pin with digest-verified one-slot IndexedDB cache, and per-category threshold offsets to counter the model-card 0.35 precision warning on `person`/`full_name`.

---

## 1. Problem statement

### 1.1 The gap

| Group (model card) | 42 labels | Hacienda today | Outcome |
|---|---|---|---|
| Person / names | `person,full_name,first_name,middle_name,last_name,date_of_birth` | `Person, FullName, DateOfBirth` (3) | `first/middle/last` → `Custom("first_name")` → no distinct token, no audit distinction |
| Contact / address | `email,phone_number,address,street_address,city,state_or_region,postal_code,country` | `Email, PhoneNumber, Address` (3) via `Location→Address` | `street_address/city/state/postal/country` → `Custom` |
| Government / tax IDs | `government_id,national_id_number,passport_number,drivers_license_number,license_number,tax_id,tax_number` | `Ssn, PassportNumber, DriversLicense, NationalId, TaxId` (5) | `government_id/license_number` → `Custom` |
| Banking / payment | `bank_account,account_number,routing_number,iban,payment_card,card_number,card_expiry,card_cvv` | `BankAccount, RoutingNumber, Iban, CreditCard` (4) | `payment_card/card_number` collapse to `CreditCard`; `card_expiry/cvv` → `Custom` |
| Digital identity | `username,ip_address,account_id,sensitive_account_id` | `Username, IpAddress` (2) | `account/sensitive_account` → `Custom` |
| Secrets / credentials | `password,secret,api_key,access_token,recovery_code` | `Password, ApiKey, SecretToken, JwtToken` (4) | `secret/recovery_code` → `Custom` |
| Sensitive dates | `sensitive_date,document_date,expiration_date,transaction_date` | `Date→Custom("Date")` via `Date/Time` | all 4 → `Custom` |

Even if the underlying ONNX/Candle weights already score those labels zero-shot, the pipeline discards them:

1. **Closed `NerCategory`** (`lib/types.ts:98` = 10; `hacienda-core/src/pii/ner.rs:18` `DEFAULT_CATEGORIES` = 5). `lib/ner-bridge.ts:28` `isBridgeEntity` rejects a `category` outside the 10 — the call throws and falls back to regex. `worker/pipeline.ts:988` then filters `xbergEntities` by `config.nerCategories` (default 5 in `types.ts:173`), so a model returning `postal_code` at offset 320 never becomes an `Entity` or `PiiEntity`.
2. **Incomplete alias** — `ner.rs:190` `to_pii_category` and `worker/pipeline.ts:460` `nerCategoryToPiiCategory` handle ~10 aliases explicitly; everything else → `Custom(label.clone())`. No test asserts 42 coverage.
3. **Collapsed redaction** — `types.rs` has one `Address`, one `CreditCard`, one `Person`; `redaction/modes.ts` and `audit/entry.rs` cannot distinguish `[FIRST_NAME]` from `[PERSON]` or `[CARD_CVV]` from `[CREDIT_CARD]`.
4. **Different pin** — `asset-loader.ts:19` fetches `jamon8888/gliner2-guardrails-pii-f16/resolve/main` (moving branch, no digest) — not the 42-label `privacy-filter-multi`. `VITE_MODEL_BASE_URL` override exists but has no identity check; a stale IDB entry silently serves the wrong taxonomy.

### 1.2 User impact

An operator enabling "PII exhaustif" expects GDPR-grade masking of street-level addresses and card CVVs. The app reports success while `card_cvv` and `street_address` mentions remain unredacted in `markdown` and exported `entities-registry.json`/`zip-export`. The `privacy-filter-multi` model card explicitly warns that recall is the PII-critical metric and that `person`/`full_name` over-predict at 0.35 precision — Studio has no per-label threshold to compensate.

---

## 2. Goals

1. **Full 42-label coverage** — every `privacy-filter-PII-multi` label is requestable, detectable, redactable, auditable, and exportable; zero labels silently dropped.
2. **Support both models** — one active model at a time via build-time `VITE_MODEL_BASE_URL`, with autonomous IDB eviction on switch; low-quota browsers stay regex-only.
3. **Strict wire compat** — `PiiCategory` serde remains `snake_case`; old `Custom("first_name")` strings and old audit chains verify byte-for-byte.
4. **Grouped collapsible UX** — 7 groups mirror the model card; parent toggle cascades to children; 14 `defaultSelected` preserved and extended.
5. **Precision-compensated thresholds** — `person`-family raised +0.15, high-precision categories untouched, single global still governs; no extra UI required in v1.
6. **Provenance** — `AuditEntry.model` records `model_id@digest8`, mirroring `vertical` provenance.

## 3. Non-goals

- Retraining or replacing the DeBERTa-v2 encoder; `MAX_WIDTH=8` and 512 `max_position_embeddings` accepted.
- Long-context encoder or batch inference; see `2026-08-23-ner-gliner-enterprise-readiness-design.md` windowing for long-document handling (42 labels increase schema prompt to ~140 tok, so `WindowPlan` budget must be respected).
- A pluggable JSON taxonomy loader; deferred until N>2 models demand it.
- Per-label sliders in the detection modal in v1; table-driven in code, UI later if needed.

---

## 4. Architecture

### 4.1 Hybrid additive taxonomy

`hacienda-core/src/pii/types.rs:9` gains **12** new unit variants (strictly additive, `#[serde(rename_all="snake_case")]`):

```rust
FirstName, MiddleName, LastName,
StreetAddress, City, StateOrRegion, PostalCode, Country,
GovernmentId,
PaymentCard, CardExpiry, CardCvv,
```

Kept collapsed where audit/redaction does not diverge: `Person`/`FullName` remain (regex `FullName` pattern stays), `Address` remains as parent for unstructured `address`, `CreditCard` remains for `card_number` (alias to it), `TaxId` covers `tax_id`/`tax_number`.

### 4.2 42→PiiCategory alias table (single source)

Expand `hacienda-core/src/pii/ner.rs:190` `to_pii_category` to the full 42, case-insensitive, with exact lowercasing match on the model-card spellings. Any label not in the table → `PiiCategory::Custom(label.clone())` (current pass-through). `apps/hacienda-studio/worker/pipeline.ts:460` `nerCategoryToPiiCategory` mirrors the table 1:1, returning `PiiCategoryWire` (`string` for unit variants, `{custom: label}` for `Custom`).

Expected mapping (authoritative; tests assert):

| wire label | PiiCategory |
|---|---|
| `person` | `Person` |
| `full_name` | `FullName` |
| `first_name` | `FirstName` |
| `middle_name` | `MiddleName` |
| `last_name` | `LastName` |
| `date_of_birth` | `DateOfBirth` |
| `email` | `Email` |
| `phone_number` | `PhoneNumber` |
| `address` | `Address` |
| `street_address` | `StreetAddress` |
| `city` | `City` |
| `state_or_region` | `StateOrRegion` |
| `postal_code` | `PostalCode` |
| `country` | `Country` |
| `government_id` | `GovernmentId` |
| `national_id_number` | `NationalId` |
| `passport_number` | `PassportNumber` |
| `drivers_license_number`,`license_number` | `DriversLicense` |
| `tax_id`,`tax_number` | `TaxId` |
| `bank_account`,`account_number` | `BankAccount` |
| `routing_number` | `RoutingNumber` |
| `iban` | `Iban` |
| `payment_card` | `PaymentCard` |
| `card_number` | `CreditCard` |
| `card_expiry` | `CardExpiry` |
| `card_cvv` | `CardCvv` |
| `username` | `Username` |
| `ip_address` | `IpAddress` |
| `account_id`,`sensitive_account_id` | `Custom("account_id"/"sensitive_account_id")` (kept collapse; distinct tokens not justified) — **or** `GovernmentId` family if audit needs split (decision: keep `Custom` in v1, promote later if needed) |
| `password` | `Password` |
| `secret` | `SecretToken` |
| `api_key` | `ApiKey` |
| `access_token` | `SecretToken` (alias; `JwtToken` retained for `jwt_token` label) |
| `recovery_code` | `Custom("recovery_code")` |
| `sensitive_date`,`document_date`,`expiration_date`,`transaction_date` | `Custom("Date")` family collapsed; `expiration_date` alias → `CardExpiry` when co-occurring with card (explicit match first) |

`BASE_CATEGORY_NAMES` in `config.rs:17` stays the 5 base names; `VerticalConfig::validate()` continues to reject a label that case-insensitively duplicates a base name.

`COMPREHENSIVE_LABELS` (`config.rs:108`) grows `29 → 41` (adds the 12 hybrid variants). `VerticalConfig::comprehensive().labels.len()` test updated to `41`.

`NerCategory` (`lib/types.ts:98`) grows `10 → 18` (adds `first_name,middle_name,last_name,street_address,city,state_or_region,postal_code,country` — the 8 where NER span boundaries differ from parent; the remaining 4 new variants `government_id,payment_card,card_expiry,card_cvv` are reached via `customLabels`/`EntityCategory::Custom`). `DEFAULT_CATEGORIES` stays 5, rest via `customLabels`.

### 4.3 Model routing

`lib/asset-loader.ts:19` `RAW_MODEL_BASE`/`MODEL_BASE`/`MODEL_URL`/`TOKENIZER_URL`/`ENCODER_CONFIG_URL` remain, but the default env becomes digest-pinned:

```
VITE_MODEL_BASE_URL = https://huggingface.co/fastino/gliner2-privacy-filter-PII-multi/resolve/<sha>  // new default
                    | https://huggingface.co/jamon8888/gliner2-guardrails-pii-f16/resolve/<sha>       // rollback
```

`loadNerModel()` after download verifies `blake3` digests (constants shipped via `import.meta.env.VITE_MODEL_SHA256_*` or baked in `lib/model-digests.ts`) before the IDB `put`; mismatch is a hard `throw` with a distinct `ModelDigestMismatch` error, not a silent regex fallback.

Identity key = `encoder_config.model_type` + `tokenizer.json` digest prefix. On `loadNerModel()` mismatch between `requested base` and `cached identity`, `clearStaleModelCache()` runs once autonomously, then re-fetches. One active model in IDB; two models never coexist (quota-safe for 600 MB vs 300 MB). Low-quota / `QuotaExceededError` still yields the existing `insufficient-storage` `NerModelLoad` variant.

`MODEL_STORAGE_BYTES_NEEDED` adjusted per model (guardrails 620 MB, privacy-filter-multi ~350 MB).

### 4.4 Worker reuse (Tier 1.1 preserved)

`worker/pipeline.ts` keeps the single NER pass: `xbergEntities` → `modelEntities` via the expanded `nerCategoryToPiiCategory`, then `redactPiiWithModelEntities(markdown, modelEntities)`. Propagates real `confidence` from `v2/decode.rs:46` `span_scores` rather than the observed constant `0.899…`; `selectNerBridge` no longer clamps to `0.9` fallback but uses `raw.confidence`.

`config.nerCategories` now holds up to 18 + `nerCustomLabels` holds the remainder of the 42 (34 labels via `customLabels` to `WasmNerConfig.customLabels`); the schema prompt cost rises to ~140 tok, so `WindowPlan` (`pii/window.rs` from the 2026-08-23 spec) budget is respected — `max_words` shrinks ~110 for 34 labels vs ~150 for 5, with `overlap_words=16`.

### 4.5 Studio detection modal

`lib/pii-categories.ts:55` `DETECTION_CATEGORIES` moves `27 → 42` checkboxes in 7 groups that mirror the model card:

- `DONNÉES PERSONNELLES` — parent `person` with collapsible children `first_name/middle_name/last_name/full_name/date_of_birth` + `email/phone_number`
- `ADRESSE / CONTACT` — parent `address` with `street_address/city/state_or_region/postal_code/country`
- `IDENTITÉS GOUVERNEMENTALES` — `government_id/national_id_number/passport_number/drivers_license_number/license_number/tax_id/tax_number`
- `PAIEMENT` — `bank_account/account_number/routing_number/iban/payment_card/card_number/card_expiry/card_cvv`
- `IDENTITÉ NUMÉRIQUE` — `username/ip_address/account_id/sensitive_account_id`
- `SECRETS` — `password/secret/api_key/access_token/recovery_code`
- `DATES SENSIBLES` — `sensitive_date/document_date/expiration_date/transaction_date`

Parent checkbox is tri-state: all children checked → checked; some → indeterminate; none → unchecked. Toggling a parent cascades to children; toggling a child updates parent. `DEFAULT_SELECTED_CODES` preserved (existing 14 remain checked; new labels default unchecked until eval calibrates them). `categoryToWire()` now returns the 42-aware `PiiCategoryWire` shape.

`lib/ner-bridge.ts:28` `NER_CATEGORIES` updated to 18; `isBridgeEntity` gate widens accordingly. `apps/hacienda-studio/lib/registry.ts` `identityFor` unchanged (`ent-` + slug from `PiiCategory` snake_case).

### 4.6 Thresholds — hybrid persona boost

`PipelineConfig` gains:

```rust
pub model_thresholds: HashMap<PiiCategory, f32>, // #[serde(default)]
```

Empty means "use `model_threshold_default`". When present, effective threshold = `model_thresholds.get(&cat).copied().unwrap_or(model_threshold_default)`. Defaults baked in `config.rs`:

- `Person, FullName, FirstName, MiddleName, LastName` → `0.65` (+0.15)
- `Email, PhoneNumber, Iban, IpAddress, CreditCard` → `0.48` (no penalty, model card high P)
- All others → `0.50`

`NerDetector::with_threshold` is replaced by `with_thresholds(HashMap)` internally; `detect()` filters per entity category after `to_pii_category` mapping, not at the `xberg` `CandleBackend` level (where `DEFAULT_THRESHOLD=0.5` still ignores the caller — the filter lives in `hacienda-core`, not upstream, until the 2026-08-23 threshold-plumbing lands).

Studio `pages/Settings.tsx` surfaces a single `sensitivity: low/balanced/high` → `0.35/0.50/0.65` that shifts the whole table uniformly (no per-label sliders in v1).

---

## 5. Public API changes

| Item | Change | Breaking? |
|---|---|---|
| `PiiCategory` (`types.rs`) | +12 variants (`FirstName` etc.) | No — `#[serde(rename_all="snake_case")]`, old JSON still deserializes; new strings unknown to old code become `Custom` via serde `other` fallback — but old code seeing a new variant string would fail if it strictly matches; mitigation: old `pii-registry.json` readers treat unknown strings as `Custom` via manual `Deserialize` fallback (add `#[serde(other)]` on `Custom` is not valid; instead a `deserialize_pii_category` helper that maps unknown unit strings to `Custom(s)` — existing `hacienda-cli` already does this for registry reads) |
| `AuditEntry` (`audit/entry.rs`) | `+ model: Option<String> #[serde(default)]`, hashed after `vertical` | No — `None` for old chains → same bytes → verify OK |
| `PipelineConfig` | `+ model_thresholds: HashMap<PiiCategory,f32>` | No (serde default) |
| `VerticalConfig::comprehensive()` | `29 → 41` labels | No (additive) |
| `NerDetector` | `with_thresholds` addition; `detect` respects per-category thresholds | Behavioural — precision up for person |
| `crates/hacienda-wasm` | re-export new `PiiCategory` variants; no ABI break | No |
| `asset-loader.ts` | digest-pinned `MODEL_BASE`, one-slot eviction | No — env override still works |
| `DETECTION_CATEGORIES` | `27 → 42` | No — `categoryToWire` unknown code → `undefined` guard still holds |

---

## 6. Files touched

### 6.1 New

- `hacienda-core/tests/pii_taxonomy_42.rs` — alias-table and compat tests (see §7).
- `apps/hacienda-studio/lib/model-digests.ts` — pinned SHA-256 constants per model (generated, not hand-typed).

### 6.2 Modified

- `hacienda-core/src/pii/types.rs` — 12 variants, `Display` already handles via `Debug`, no change.
- `hacienda-core/src/pii/ner.rs` — 42 alias table, per-category threshold filter, `categories_with_vertical` unchanged.
- `hacienda-core/src/pii/config.rs` — `COMPREHENSIVE_LABELS` 41, `model_thresholds` field, `validate()` unchanged.
- `hacienda-core/src/audit/entry.rs` — `model` field, chain hash, back-compat test.
- `hacienda-core/src/pii/window.rs` — no change beyond respecting new `customLabels` count in budget (already computes `schema_words`).
- `crates/hacienda-wasm/src/lib.rs` — surface new variants (auto via `PiiCategory` serde).
- `apps/hacienda-studio/lib/types.ts` — `NerCategory` 18.
- `apps/hacienda-studio/lib/ner-bridge.ts` — `NER_CATEGORIES` 18, `isBridgeEntity` widen.
- `apps/hacienda-studio/lib/pii-categories.ts` — 42 `DETECTION_CATEGORIES` grouped collapsible.
- `apps/hacienda-studio/lib/asset-loader.ts` — pinned SHA, identity check, eviction.
- `apps/hacienda-studio/lib/pii-engine.ts` — unchanged shape, but `PiiCategoryWire` now covers new variants.
- `apps/hacienda-studio/worker/pipeline.ts` — 42 alias mirror, real confidence propagation.
- `apps/hacienda-studio/pages/Settings.tsx` — sensitivity control wiring.
- `.github/workflows/ci-ner-eval.yaml` — per-label F1 gate (reuse from 2026-08-23 spec).

---

## 7. Testing strategy

### 7.1 Unit & integration (default `cargo test`, no model)

| Test | Asserts |
|---|---|
| `should_map_all_42_privacy_filter_labels_to_pii_category` | each of the 42 lowercased wires lands on the expected variant (table in §4.2), exhaustive |
| `should_map_unknown_label_to_custom` | `"tax_id_number"` → `Custom("tax_id_number")` |
| `should_keep_old_custom_strings_deserializing` | `"first_name"` stored as `Custom("first_name")` before the variant existed still deserializes (now as `FirstName`) via migration read path |
| `should_validate_grouped_categories_cascade` | parent toggle → all children toggled; child change → parent indeterminate |
| `should_verify_a_chain_written_before_the_model_field_existed` | audit chain written pre-`model` still `verify()` |
| `should_apply_per_category_threshold_offsets` | `person` at 0.52 filtered, at 0.66 kept; `email` at 0.49 kept |
| `should_size_comprehensive_to_41` | `VerticalConfig::comprehensive().labels.len()==41` |
| `should_not_evict_when_cached_model_matches_requested` | `loadNerModel` identity match → no `clearStaleModelCache` |

All use the existing `StubBackend` pattern (`ner.rs` tests); no weights, no `ner-candle` feature needed.

### 7.2 Quality gate

Reuse `2026-08-23-ner-gliner-enterprise-readiness-design.md` §7.2 workflow `ci-ner-eval.yaml`:

1. Fetch **pinned** `privacy-filter-multi` via digest cache.
2. Run `cargo test -p hacienda-core --features ner-candle -- --ignored ner_eval`.
3. Compare per-label F1 against `fixtures/ner-eval/baseline_privacy_filter_multi.json`.
4. Fail if per-label F1 drops > bootstrap CI half-width below baseline; post delta as PR comment.
5. Expected: `person` family precision +10–15 pts at `0.65` vs `0.50` baseline, recall −3–5 pts; other categories flat.

### 7.3 Performance budget

42 labels → schema prompt ~140 tok → `max_words ≈110` (vs 150 for 5). For a 10-page French document (~5000 words): ~46 windows vs 34; browser mid-tier budget <25 s, OOM guard as in enterprise-readiness spec §7.3.

---

## 8. Risks and mitigations

| Risk | Likelihood | Mitigation |
|---|---|---|
| Schema prompt bloat cuts `max_words` → more windows → slower + more overlap dedup pressure | High | Respects existing `WindowPlan` budget derivation (§4.4); measure in §7.3; consider capping active categories per document if >45 windows |
| `FirstName` vs `FullName` overlap dedup (nested spans) | Medium | Windowing dedup rule keeps longer span; `merge::merge_entities` `prefer_stronger` tie-break already prefers longer |
| New `PiiCategory` strings break external `pii-registry.json` consumers | Low | Strict additive + `Custom` fallback deserializer; publish taxonomy JSON `taxonomy/privacy-filter-multi.json` with 42 entries as contract |
| Two models' IDB churn on repeated `VITE_MODEL_BASE_URL` switches | Low | One-slot eviction only on identity mismatch; no auto-toggle loop; user must clear cache or change env |
| `person` threshold +0.15 hurts recall on sparse names | Medium | `format_preserving: false` regex still covers nothing for `person`; but per-label `0.65` is tunable via `PipelineConfig.model_thresholds` toml override without rebuild |

---

## 9. Rollout

**Phase 0 — Additive taxonomy, off by default.** Land `types.rs` + `ner.rs` alias + `config.rs` 41 + tests. `DETECTION_CATEGORIES` still 27; new labels only reachable via `nerCustomLabels`/`VerticalConfig`. No behavior change without flag.

**Phase 1 — Model pin behind feature flag.** Ship `asset-loader.ts` digest pin + identity eviction under `VITE_MODEL_BASE_URL` switch; CI builds both digests; `main` stays on `guardrails-pii-f16`.

**Phase 2 — Flip default + UI.** Point default `VITE_MODEL_BASE_URL` to `privacy-filter-multi` SHA, ship grouped 42 modal, enable per-category offsets. Keep guardrails as rollback via env var. Run §7.2 gate to prove recall + precision delta before merge.

Each phase is independently revertible; Phase 0 needs no model download.

---

## 10. Open questions

1. Should `account_id`/`sensitive_account_id` become their own `PiiCategory` variants or stay `Custom` collapsed? Proposed: stay `Custom` in v1 — pseudonym token distinction not needed until a consumer routes on them.
2. Should `recovery_code` and `secret` share `SecretToken` or get a dedicated `RecoveryCode` variant? Proposed: `Custom` in v1; promote if export proves need.
3. Do `expiration_date`/`transaction_date` need distinct variants or stay `Custom("Date")`? Proposed: stay collapsed; `CardExpiry` already extracts the card-relevant case.
4. Confirm `privacy-filter-multi` F32 vs F16 distribution — `jamon8888` F16 was a derived artifact; `fastino` F32 is 0.3 B params. Quota math in §4.3 assumes ~350 MB post-conversion; verify after fetching pinned SHA.

---

## 11. Dependencies on other specs

- `2026-08-23-ner-gliner-enterprise-readiness-design.md` — windowing (§4.1), truncation observability, pin/digest verification, and threshold plumbing are assumed present. This spec **builds on** that work; if windowing is not yet landed, Phase 0 still ships but long documents with 42 labels will see lower `max_words` coverage per window.

---

## Appendix A — Model card reference

`fastino/gliner2-privacy-filter-PII-multi` (2026-05-11, Apache-2.0, 205 M params, 7 languages EN/FR/ES/DE/IT/PT/NL, 4,910 synthetic texts, 129,951 mentions):

| Group | Labels |
|---|---|
| Person / names | `person, full_name, first_name, middle_name, last_name, date_of_birth` |
| Contact / address | `email, phone_number, address, street_address, city, state_or_region, postal_code, country` |
| Government / tax IDs | `government_id, national_id_number, passport_number, drivers_license_number, license_number, tax_id, tax_number` |
| Banking / payment | `bank_account, account_number, routing_number, iban, payment_card, card_number, card_expiry, card_cvv` |
| Digital identity | `username, ip_address, account_id, sensitive_account_id` |
| Secrets / credentials | `password, secret, api_key, access_token, recovery_code` |
| Sensitive dates | `sensitive_date, document_date, expiration_date, transaction_date` |

Model card warns: precision 0.35–0.37, `person`/`full_name` over-predict. Benchmark SPY avg F1 0.477 (best recall among compared GLiNER-based detectors).

---

## Appendix B — Migration sketch

Old `pii-registry.json` entry:

```json
{"category":"custom","span_hash":"ab12…","action":"mask"}
```

New:

```json
{"category":"first_name","span_hash":"ab12…","action":"mask","model":"privacy-filter-multi@3f9a1c02"}
```

Reader code:

```rust
fn deserialize_pii_category<'de, D>(d: D) -> Result<PiiCategory, D::Error> where D: Deserializer<'de> {
  let s = String::deserialize(d)?;
  Ok(match PiiCategory::try_from_snake(&s) {
    Some(cat) => cat,
    None => PiiCategory::Custom(s),
  })
}
```

No chain migration needed; `verify()` on old chains succeeds with `model=None`.

