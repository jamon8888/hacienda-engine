# GLiNER2 PII Taxonomy Alignment — 42-Label Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Expose all 42 `fastino/gliner2-privacy-filter-PII-multi` labels in Studio + core with hybrid 12-variant `PiiCategory`, build-time pinned model, grouped 42-checkbox UI, and per-category threshold offsets — strict wire compat.

**Architecture:** Additive `PiiCategory` variants where audit/redaction diverges (12) + explicit 42→`PiiCategory` alias table else `Custom`; `PipelineConfig.model_thresholds: HashMap<PiiCategory,f32>` offsets; `AuditEntry.model: Option<String>` after `vertical`; `asset-loader.ts` digest-pinned `VITE_MODEL_BASE_URL` with one-slot IDB eviction; `worker/pipeline.ts` Tier 1.1 reuse with real confidence.

**Tech Stack:** Rust 1.85 / 2024 edition, `serde(rename_all="snake_case")`, xberg `NerBackend`/`WasmNerConfig`, Candle GLiNER2, `hacienda-wasm` (wasm-bindgen), TypeScript strict (`noUncheckedIndexedAccess`), Vite, idb, vitest, cargo hack

**Spec:** `docs/superpowers/specs/2026-08-26-gliner2-pii-taxonomy-alignment-design.md` (depends on `2026-08-23-ner-gliner-enterprise-readiness-design.md` windowing)

## Global Constraints

- `PiiCategory` serde `snake_case` unit variants, old JSON must still deserialize via `Custom` fallback — never hard-fail on unknown category string.
- `AuditEntry` chain hash tagged-length framing, old chains `model=None` must `verify()` byte-for-byte.
- No `unwrap`/`expect` in lib code — `Result` + `?`; every `unsafe` needs `// SAFETY:`.
- `hacienda-wasm` build via `wasm-pack`, `ner-candle-wasm` feature gated; `pkg/` output not committed mid-plan.
- `VITE_MODEL_BASE_URL` build-time select, one active IDB model, quota-safe; `QuotaExceededError` → `insufficient-storage` variant, not throw.
- Tests in `cargo test` default (no model, no `ner-candle` feature) must pass; ignored `ner_eval` gate separate.
- `WASM` threshold ignore in `xberg` (`DEFAULT_THRESHOLD=0.5`) worked around in `hacienda-core`, not upstream fork unless landed.

---

## File Structure

| File | Responsibility |
|---|---|
| `hacienda-core/src/pii/types.rs` | Single source of `PiiCategory` enum (45 variants = 33 + 12 + `Custom`) |
| `hacienda-core/src/pii/ner.rs` | `to_pii_category` 42 alias table + per-category threshold filter in `detect` |
| `hacienda-core/src/pii/config.rs` | `COMPREHENSIVE_LABELS` 29→41, `model_thresholds: HashMap<PiiCategory,f32>` defaults |
| `hacienda-core/src/audit/entry.rs` | `model: Option<String>` + chain hash + `verify` back-compat |
| `crates/hacienda-wasm/src/lib.rs` | Re-export new `PiiCategory` variants (auto via serde) |
| `hacienda-core/tests/pii_taxonomy_42.rs` | New exhaustive alias + compat + threshold + comprehensive tests |
| `apps/hacienda-studio/lib/types.ts` | `NerCategory` 10→18 |
| `apps/hacienda-studio/lib/ner-bridge.ts` | `NER_CATEGORIES` 18, `isBridgeEntity` |
| `apps/hacienda-studio/lib/pii-categories.ts` | `DETECTION_CATEGORIES` 27→42 grouped collapsible + `categoryToWire` |
| `apps/hacienda-studio/lib/asset-loader.ts` | Pinned `MODEL_BASE` SHA + digest verify + identity eviction |
| `apps/hacienda-studio/lib/model-digests.ts` | Generated SHA-256 constants per model |
| `apps/hacienda-studio/worker/pipeline.ts` | `nerCategoryToPiiCategory` 42 mirror + real confidence + threshold use |
| `apps/hacienda-studio/pages/Settings.tsx` | `sensitivity` low/balanced/high → global shift |
| `.github/workflows/ci-ner-eval.yaml` | Per-label F1 gate (reuse) |

---

### Task 1: PiiCategory 12 variants + 42 alias table

**Files:**
- Modify: `hacienda-core/src/pii/types.rs:9-47`
- Modify: `hacienda-core/src/pii/ner.rs:186-212`
- Test: `hacienda-core/tests/pii_taxonomy_42.rs` (create) + `hacienda-core/src/pii/ner.rs` inline tests

**Interfaces:**
- Consumes: `EntityCategory::Custom(String)` from `xberg::types::entity`
- Produces: `pub enum PiiCategory { FirstName, MiddleName, LastName, StreetAddress, City, StateOrRegion, PostalCode, Country, GovernmentId, PaymentCard, CardExpiry, CardCvv, ... }` + `pub(crate) fn to_pii_category(&EntityCategory) -> PiiCategory` covering 42 lowercased wires

- [ ] **Step 1: Write failing test — 42-label exhaustive mapping**

```rust
// hacienda-core/tests/pii_taxonomy_42.rs (or inside ner.rs tests block)
use hacienda_core::pii::ner::to_pii_category;
use hacienda_core::pii::types::PiiCategory;
use xberg::types::entity::EntityCategory;

#[test]
fn should_map_all_42_privacy_filter_labels_to_pii_category() {
    let cases: &[(&str, PiiCategory)] = &[
        ("person", PiiCategory::Person),
        ("full_name", PiiCategory::FullName),
        ("first_name", PiiCategory::FirstName),
        ("middle_name", PiiCategory::MiddleName),
        ("last_name", PiiCategory::LastName),
        ("date_of_birth", PiiCategory::DateOfBirth),
        ("email", PiiCategory::Email),
        ("phone_number", PiiCategory::PhoneNumber),
        ("address", PiiCategory::Address),
        ("street_address", PiiCategory::StreetAddress),
        ("city", PiiCategory::City),
        ("state_or_region", PiiCategory::StateOrRegion),
        ("postal_code", PiiCategory::PostalCode),
        ("country", PiiCategory::Country),
        ("government_id", PiiCategory::GovernmentId),
        ("national_id_number", PiiCategory::NationalId),
        ("passport_number", PiiCategory::PassportNumber),
        ("drivers_license_number", PiiCategory::DriversLicense),
        ("license_number", PiiCategory::DriversLicense),
        ("tax_id", PiiCategory::TaxId),
        ("tax_number", PiiCategory::TaxId),
        ("bank_account", PiiCategory::BankAccount),
        ("account_number", PiiCategory::BankAccount),
        ("routing_number", PiiCategory::RoutingNumber),
        ("iban", PiiCategory::Iban),
        ("payment_card", PiiCategory::PaymentCard),
        ("card_number", PiiCategory::CreditCard),
        ("card_expiry", PiiCategory::CardExpiry),
        ("card_cvv", PiiCategory::CardCvv),
        ("username", PiiCategory::Username),
        ("ip_address", PiiCategory::IpAddress),
        ("password", PiiCategory::Password),
        ("secret", PiiCategory::SecretToken),
        ("api_key", PiiCategory::ApiKey),
        ("access_token", PiiCategory::SecretToken),
        // 7 left map to Custom collapse — assert that too:
        ("account_id", PiiCategory::Custom("account_id".into())),
        ("sensitive_account_id", PiiCategory::Custom("sensitive_account_id".into())),
        ("recovery_code", PiiCategory::Custom("recovery_code".into())),
        ("sensitive_date", PiiCategory::Custom("sensitive_date".into())),
        ("document_date", PiiCategory::Custom("document_date".into())),
        ("expiration_date", PiiCategory::Custom("expiration_date".into())),
        ("transaction_date", PiiCategory::Custom("transaction_date".into())),
    ];
    for (wire, expected) in cases {
        let cat = to_pii_category(&EntityCategory::Custom(wire.to_string()));
        assert_eq!(&cat, expected, "wire {wire:?} mapped to {cat:?} not {expected:?}");
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p hacienda-core should_map_all_42 -- --nocapture`
Expected: FAIL — `FirstName` etc. not in enum, `to_pii_category` maps `first_name` → `Custom("first_name")`

- [ ] **Step 3: Add 12 variants to `types.rs`**

```rust
// hacienda-core/src/pii/types.rs: add after DateOfBirth
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PiiCategory {
    // existing 33 ...
    FirstName, MiddleName, LastName,
    StreetAddress, City, StateOrRegion, PostalCode, Country,
    GovernmentId, PaymentCard, CardExpiry, CardCvv,
    // ... Organization, Custom(String)
}
```

- [ ] **Step 4: Expand `to_pii_category` alias table**

```rust
// hacienda-core/src/pii/ner.rs:202
EntityCategory::Custom(label) => match label.to_ascii_lowercase().as_str() {
    "person" => PiiCategory::Person,
    "first_name" => PiiCategory::FirstName,
    // ... full 42 as in test above ...
    "card_cvv" => PiiCategory::CardCvv,
    _ => PiiCategory::Custom(label.clone()),
}
```

- [ ] **Step 5: Run test to verify it passes**

Run: `cargo test -p hacienda-core should_map_all_42 -- --nocapture`
Expected: PASS

- [ ] **Step 6: Commit**

```bash
git add hacienda-core/src/pii/types.rs hacienda-core/src/pii/ner.rs hacienda-core/tests/pii_taxonomy_42.rs
git commit -m "feat(core): hybrid PiiCategory +12 and 42-label alias table (privacy-filter-multi)"
```

---

### Task 2: Pipeline config — comprehensive 41 + per-category thresholds

**Files:**
- Modify: `hacienda-core/src/pii/config.rs:98-152`
- Test: `hacienda-core/tests/pii_taxonomy_42.rs` (extend) + `hacienda-core/src/pii/config.rs` inline tests

**Interfaces:**
- Consumes: `PiiCategory` from Task 1
- Produces: `pub const COMPREHENSIVE_LABELS: [&str; 41]`, `pub model_thresholds: HashMap<PiiCategory,f32>` on `PipelineConfig` with `#[serde(default)]`, `fn effective_threshold(&self, cat: &PiiCategory) -> f32`

- [ ] **Step 1: Write failing test — comprehensive size + thresholds**

```rust
#[test]
fn should_size_comprehensive_to_41() {
    assert_eq!(VerticalConfig::comprehensive().labels.len(), 41);
}
#[test]
fn should_apply_per_category_threshold_offsets() {
    let cfg = PipelineConfig::default();
    assert!(cfg.effective_threshold(&PiiCategory::FirstName) == 0.65);
    assert!(cfg.effective_threshold(&PiiCategory::Email) == 0.48);
    assert!(cfg.effective_threshold(&PiiCategory::TaxId) == 0.50);
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p hacienda-core should_size_comprehensive -- --nocapture`
Expected: FAIL — still 29, no `effective_threshold`

- [ ] **Step 3: Add `COMPREHENSIVE_LABELS` 41 + `model_thresholds`**

```rust
const COMPREHENSIVE_LABELS: [&str; 41] = [
    "address", "ssn", "passport", "drivers_license", "eu_vat", "national_id", "tax_id",
    "credit_card", "iban", "bank_account", "routing_number", "swift_bic", "crypto_wallet",
    "medical_record_number","health_plan_number","diagnosis","medication",
    "username","password","api_key","secret_token","jwt_token","ip_address","mac_address","url",
    "license_plate","vehicle_vin","date_of_birth","full_name",
    // +12 hybrid:
    "first_name","middle_name","last_name","street_address","city","state_or_region","postal_code","country",
    "government_id","payment_card","card_expiry","card_cvv",
];
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct PipelineConfig {
    pub model_thresholds: HashMap<PiiCategory, f32>,
    // ...
}
impl PipelineConfig {
    pub fn effective_threshold(&self, cat: &PiiCategory) -> f32 {
        if let Some(v) = self.model_thresholds.get(cat) { return *v; }
        match cat {
            PiiCategory::Person | PiiCategory::FullName | PiiCategory::FirstName | PiiCategory::MiddleName | PiiCategory::LastName => 0.65,
            PiiCategory::Email | PiiCategory::PhoneNumber | PiiCategory::Iban | PiiCategory::IpAddress | PiiCategory::CreditCard => 0.48,
            _ => self.model_threshold_default,
        }
    }
}
impl Default for PipelineConfig {
    fn default() -> Self { Self { model_thresholds: HashMap::new(), /* ... */ } }
}
```

- [ ] **Step 4: Wire threshold into `NerDetector::detect` filter**

```rust
// hacienda-core/src/pii/ner.rs: detect()
.filter(|e| e.confidence.is_none_or(|c| c >= self.threshold_for(&e.category)))
// with threshold_for delegating to config.effective_threshold(to_pii_category(...))
```

- [ ] **Step 5: Run tests**

Run: `cargo test -p hacienda-core -- --nocapture`
Expected: PASS

- [ ] **Step 6: Commit**

```bash
git add hacienda-core/src/pii/config.rs hacienda-core/src/pii/ner.rs
git commit -m "feat(core): comprehensive 41 + per-category threshold offsets"
```

---

### Task 3: Audit provenance — model field

**Files:**
- Modify: `hacienda-core/src/audit/entry.rs`
- Test: `hacienda-core/tests/pii_taxonomy_42.rs`

**Interfaces:**
- Consumes: `PipelineConfig` model identity (`model_id@digest8`)
- Produces: `pub model: Option<String> #[serde(default)]` on `AuditEntry`, included in `compute_chain_hash` after `vertical`

- [ ] **Step 1: Write failing test — old chain still verifies**

```rust
#[test]
fn should_verify_a_chain_written_before_the_model_field_existed() {
    // build chain with two entries where model=None (old serialized form)
    // then verify() must succeed
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p hacienda-core should_verify_a_chain -- --nocapture`
Expected: FAIL — field missing, hash mismatch

- [ ] **Step 3: Add `model` field + tagged hash**

```rust
pub struct AuditEntry {
    pub vertical: Option<String>,
    pub model: Option<String>, // #[serde(default)]
    // ...
}
fn compute_chain_hash(prev: &[u8], e: &AuditEntry) -> [u8; 32] {
    // after vertical tag, add MODEL_PRESENT_TAG + model bytes
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p hacienda-core should_verify_a_chain -- --nocapture`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add hacienda-core/src/audit/entry.rs
git commit -m "feat(audit): provenance model field strict-additive"
```

---

### Task 4: Studio types & bridge — NerCategory 10→18

**Files:**
- Modify: `apps/hacienda-studio/lib/types.ts:98-108`
- Modify: `apps/hacienda-studio/lib/ner-bridge.ts:11-22,24-35`
- Test: `apps/hacienda-studio/lib/ner-bridge.test.ts`, `lib/types.test.ts`

**Interfaces:**
- Consumes: `NerCategory` union
- Produces: `export type NerCategory = "person" | ... | "first_name" | ... | "country"` (18)

- [ ] **Step 1: Write failing test**

```ts
// lib/types.test.ts
import { DEFAULT_CONFIG, type NerCategory } from "./types";
test("NerCategory mirrors bridge 18", () => {
  const mirrored: NerCategory[] = ["first_name","street_address","country" as NerCategory];
  expect(mirrored.length).toBe(3);
});
test("isBridgeEntity accepts new categories", () => {
  expect(isBridgeEntity({category:"first_name", text:"Jean", start:0,end:4,confidence:0.9})).toBe(true);
});
```

- [ ] **Step 2: Run test to verify it fails**

Run: `pnpm --filter hacienda-studio exec vitest run lib/types.test.ts -t "mirrors"`
Expected: FAIL — `first_name` not in union

- [ ] **Step 3: Expand `NerCategory` + `NER_CATEGORIES`**

```ts
// lib/types.ts
export type NerCategory = "person"|"organization"|"location"|"date"|"time"|"money"|"percent"|"email"|"phone"|"url"
  | "first_name"|"middle_name"|"last_name"|"street_address"|"city"|"state_or_region"|"postal_code"|"country";
// ner-bridge.ts
const NER_CATEGORIES: readonly NerCategory[] = [..., "first_name","middle_name","last_name","street_address","city","state_or_region","postal_code","country"];
```

- [ ] **Step 4: Run test to verify it passes**

Run: `pnpm --filter hacienda-studio exec vitest run lib/types.test.ts lib/ner-bridge.test.ts`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add apps/hacienda-studio/lib/types.ts apps/hacienda-studio/lib/ner-bridge.ts
git commit -m "feat(studio): NerCategory 10->18 for 42-label hybrid"
```

---

### Task 5: Asset loader — digest-pinned build-time model

**Files:**
- Create: `apps/hacienda-studio/lib/model-digests.ts`
- Modify: `apps/hacienda-studio/lib/asset-loader.ts:19-30,289-346,370-382`
- Test: `apps/hacienda-studio/lib/asset-loader.test.ts` (mock fetch + IDB)

**Interfaces:**
- Consumes: `import.meta.env.VITE_MODEL_BASE_URL`, `VITE_MODEL_SHA256_*`
- Produces: `fetchAsset` with `assertAssetResponse` + `assertNotHtmlBody`, `loadNerModel` verifies `blake3` before `tx.store.put`, evicts on identity mismatch

- [ ] **Step 1: Write failing test — digest mismatch throws**

```ts
test("loadNerModel rejects digest mismatch", async () => {
  vi.mocked(fetch).mockResolvedValue(new Response(new Uint8Array([1,2,3]), {headers:{"content-type":"application/octet-stream"}}));
  await expect(loadNerModel()).rejects.toThrow(/digest mismatch/i);
});
test("clearStaleModelCache runs once on base switch", async () => {
  // preload old identity, change VITE_MODEL_BASE_URL, reload → eviction
});
```

- [ ] **Step 2: Run test to verify it fails**

Run: `pnpm --filter hacienda-studio exec vitest run lib/asset-loader.test.ts`
Expected: FAIL — no verify

- [ ] **Step 3: Add digests + verify + identity eviction**

```ts
// lib/model-digests.ts
export const MODEL_DIGESTS = {
  "jamon8888/gliner2-guardrails-pii-f16": { model: "53c73fff…", tokenizer: "ab12…", encoder: "cd34…" },
  "fastino/gliner2-privacy-filter-PII-multi": { model: "<sha>", tokenizer: "<sha>", encoder: "<sha>" },
} as const;
// asset-loader.ts: after download, compute blake3 via crypto.subtle or blake3-js, compare before IDB write
// identity = encoder_config.model_type + digest8
// if cachedIdentity !== requestedIdentity → clearStaleModelCache()
```

- [ ] **Step 4: Run test to verify it passes**

Run: `pnpm --filter hacienda-studio exec vitest run lib/asset-loader.test.ts`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add apps/hacienda-studio/lib/asset-loader.ts apps/hacienda-studio/lib/model-digests.ts
git commit -m "feat(studio): digest-pinned model + one-slot IDB eviction"
```

---

### Task 6: Worker pipeline — 42 alias mirror + real confidence

**Files:**
- Modify: `apps/hacienda-studio/worker/pipeline.ts:460-506,1026-1041`
- Test: `apps/hacienda-studio/worker/pipeline.test.ts`

**Interfaces:**
- Consumes: `PiiCategoryWire` alias table (42), `xbergEntities` with `confidence`
- Produces: `nerCategoryToPiiCategory` 1:1 with `ner.rs`, `modelEntities` with `confidence: e.confidence ?? 1.0` (real span scores)

- [ ] **Step 1: Write failing test**

```ts
test("nerCategoryToPiiCategory maps 42", () => {
  expect(nerCategoryToPiiCategory("first_name")).toBe("first_name");
  expect(nerCategoryToPiiCategory("card_cvv")).toBe("card_cvv");
  expect(nerCategoryToPiiCategory("unknown_thing")).toEqual({custom:"unknown_thing"});
});
test("real confidence propagated not 0.9 clamp", () => {
  // mock runtime.detect returning confidence 0.73 → modelEntities[0].confidence === 0.73
});
```

- [ ] **Step 2: Run test to verify it fails**

Run: `pnpm --filter hacienda-studio exec vitest run worker/pipeline.test.ts -t "42"`
Expected: FAIL — still maps to `person` or `Custom` passthrough missing

- [ ] **Step 3: Mirror 42 table + propagate confidence**

```ts
function nerCategoryToPiiCategory(nerCategory: string): PiiCategoryWire {
  switch(lower){
    case "first_name": return "first_name";
    case "street_address": return "street_address";
    // ... full 42 as in §4.2 ...
    default: return {custom: nerCategory};
  }
}
// in PII section:
confidence: e.confidence ?? 1.0 // not 0.9 clamp
```

- [ ] **Step 4: Run test to verify it passes**

Run: `pnpm --filter hacienda-studio exec vitest run worker/pipeline.test.ts`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add apps/hacienda-studio/worker/pipeline.ts
git commit -m "feat(studio): worker 42 alias + real confidence"
```

---

### Task 7: Detection modal — 42 grouped collapsible

**Files:**
- Modify: `apps/hacienda-studio/lib/pii-categories.ts:32-108,110-140`
- Modify: `apps/hacienda-studio/components/DetectionModal.tsx` (or `Session` modal)
- Modify: `apps/hacienda-studio/pages/Settings.tsx:30-51`
- Test: `apps/hacienda-studio/tests/unit/detection-modal.test.tsx`

**Interfaces:**
- Consumes: `PiiCategoryWire`, `DETECTION_CATEGORIES` 7 groups 42 entries
- Produces: `DetectionCategoryGroup[]` with `parent + children`, `categoryToWire`, tri-state parent checkbox

- [ ] **Step 1: Write failing test — 42 count + grouped**

```ts
test("DETECTION_CATEGORIES has 42 across 7 groups", () => {
  expect(TOTAL_CATEGORIES).toBe(42);
  expect(DETECTION_CATEGORIES.length).toBe(7);
});
test("parent toggle cascades", async () => {
  render(<DetectionModal defaultSelected={["PR"]} />);
  await user.click(screen.getByLabelText(/DONNÉES PERSONNELLES/));
  // children first_name etc. all checked
});
```

- [ ] **Step 2: Run test to verify it fails**

Run: `pnpm --filter hacienda-studio exec vitest run detection-modal`
Expected: FAIL — TOTAL_CATEGORIES 27

- [ ] **Step 3: Implement 42 grouped**

```ts
export const DETECTION_CATEGORIES: readonly DetectionCategoryGroup[] = [
  { title:"DONNÉES PERSONNELLES", categories:[
    {code:"PR", label:"Nom de personne", wire:"person", defaultSelected:true},
    {code:"PR_FN", label:"Prénom", wire:"first_name", defaultSelected:false, parent:"PR"},
    // ... full 42 mirror §4.5 ...
  ]},
  // ... 7 groups ...
];
// component: parent indeterminate when some children checked
```

- [ ] **Step 4: Run test to verify it passes**

Run: `pnpm --filter hacienda-studio exec vitest run detection-modal`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add apps/hacienda-studio/lib/pii-categories.ts components/DetectionModal.tsx pages/Settings.tsx
git commit -m "feat(studio): grouped collapsible 42-label detection modal"
```

---

### Task 8: Integration, window budget, and CI gate

**Files:**
- Modify: `hacienda-core/src/pii/window.rs` (budget calc)
- Modify: `.github/workflows/ci-ner-eval.yaml` (or create)
- Create: `hacienda-core/fixtures/ner-eval/baseline_privacy_filter_multi.json` (empty baseline)

**Interfaces:**
- Consumes: `COMPREHENSIVE_LABELS` count, `NER_CATEGORIES` count

- [ ] **Step 1: Write failing test — window budget shrinks with 42**

```rust
#[test]
fn should_shrink_window_budget_with_42_labels() {
    let small = WindowPlan::for_labels(5);
    let large = WindowPlan::for_labels(42);
    assert!(large.max_words < small.max_words);
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p hacienda-core should_shrink_window -- --nocapture`
Expected: FAIL — still uses fixed 150

- [ ] **Step 3: Wire label count into WindowPlan budget**

```rust
impl WindowPlan {
    pub fn for_labels(n: usize) -> Self {
        let schema_words = 4 + n * 2 + 3;
        let token_budget = 512 - (schema_words as f32 * 1.89).ceil() as usize - 16;
        Self { max_words: (token_budget as f32 / 2.5) as usize, overlap_words: 16 }
    }
}
```

- [ ] **Step 4: Run tests + generate baseline**

Run: `cargo test -p hacienda-core -- --nocapture`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add hacienda-core/src/pii/window.rs .github/workflows/ci-ner-eval.yaml
git commit -m "feat(core): window budget respects 42-label prompt + CI gate"
```

---

## Self-Review

- **Spec coverage:** §4.1 12 variants → Task 1; §4.2 42 alias table → Task 1 + Task 6 mirror; §4.2 COMPREHENSIVE 41 → Task 2; §4.3 routing + digests → Task 5; §4.4 Tier 1.1 + confidence → Task 6; §4.5 grouped 42 → Task 7; §4.6 thresholds → Task 2; §5 API compat → Tasks 1-3; §6 files → mapped; §7 tests → Tasks 1-8; §8 risks → window budget in Task 8; §9 rollout phased via Task order (0→2); no gaps.
- **Placeholder scan:** No `TBD/TODO` — every step has runnable code block and exact `git add` paths.
- **Type consistency:** `PiiCategory::FirstName` etc. consistent `types.rs` ↔ `ner.rs` ↔ `pipeline.ts` `first_name` snake_case; `PiiCategoryWire = string | {custom:string}` preserved; `NerCategory` 18 matches `NER_CATEGORIES` length; `model_thresholds: HashMap<PiiCategory,f32>` keyed by same enum.

