# Phase 0: Real Pseudonymisation — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace `RedactionMode::Pseudonymize`, which currently emits a per-category constant, with a keyed, deterministic, reversible pseudonymiser — and make it impossible to run the mode without a key.

**Architecture:** A new `redaction::pseudonym` module wrapping AES-SIV (RFC 5297) deterministic AEAD. Tokens are derived, never stored, so two nodes produce identical tokens with no shared table. The key is resolved through an injected `KeyResolver`, deliberately outside the config precedence chain. `RedactionEngine` gains an optional `Arc<Pseudonymiser>` and a fallible constructor that refuses `Pseudonymize` without one.

**Tech Stack:** Rust 2021, `aes-siv` 0.7 (stable — **not** the 0.8 RC), `data-encoding` 2.11, `unicode-normalization` 0.1, `zeroize` 1.9, `blake3` (already present), inline `#[cfg(test)]` tests.

**Spec:** `superpowers/specs/2026-07-28-hacienda-cli-api-integration-design.md` §12.3, §12.3.1, §12.3.2. Closes §8 Gap 7.

---

## Ground Truth — Verified vs Assumed

A previous plan in this repo described APIs that did not exist. This section separates what was read from source on 2026-07-28 from what still needs confirming.

**Verified by reading source:**

| Fact | Location |
|---|---|
| `RedactionMode::Pseudonymize => format!("[{:?}:****]", entity.category).to_uppercase()` | `redaction/engine.rs:93` |
| `Pseudonymize` is `#[default]` | `redaction/types.rs:14-15` |
| `RedactionConfig::default().mode == Pseudonymize` | `redaction/types.rs:53` |
| `RedactionEngine { mode, custom_template }` — no key field; `new()` is infallible, returns `Self` | `redaction/engine.rs:11-22` |
| `replacement_for(&self, entity: &MergedEntity, original: &str) -> String` | `redaction/engine.rs:83-104` |
| `RedactionEngine::new(config.redaction.clone())` is the only construction site | `pii/pipeline.rs:99` |
| `assemble` already returns `Result<Self, PiiError>` | `pii/pipeline.rs:81` |
| `PiiError` has `#[from] RedactionError` | `pii/mod.rs` |
| `PiiCategory` has 32 unit variants plus `Custom(String)`; `Display` prints the variant name, `Custom(s)` prints `s` | `pii/types.rs:9-52` |
| Redaction is synchronous — `redact()` is a plain `fn` called from an async context | `redaction/engine.rs:30` |
| No AEAD, base32, or normalisation crate exists anywhere in the workspace | `grep` over both manifests |
| All tests are inline `#[cfg(test)]`; there is no `tests/` directory | whole crate |
| Existing pseudonymize test asserts `"mail [EMAIL:****] ok"` | `redaction/engine.rs:163` |

**`aes-siv` 0.7.0 API — verified in Task 1** (read from
`~/.cargo/registry/src/*/aes-siv-0.7.0/src/{lib.rs,siv.rs}`, then proven by 8 passing tests):

```rust
pub type Aes256Siv = CmacSiv<Aes256>;              // siv.rs:65
impl KeySizeUser for Siv<Aes256, _> { type KeySize = U64; }   // 64 bytes, NOT 32

impl KeyInit { fn new(key: &GenericArray<u8, KeySize<C>>) -> Self; }

pub fn encrypt<I, T>(&mut self, headers: I, plaintext: &[u8]) -> Result<Vec<u8>, Error>
pub fn decrypt<I, T>(&mut self, headers: I, ciphertext: &[u8]) -> Result<Vec<u8>, Error>
where I: IntoIterator<Item = T>, T: AsRef<[u8]>
```

Three findings that change the plan as originally written:

1. **`encrypt`/`decrypt` take `&mut self`.** A `Pseudonymiser` cannot hold a live `Siv` and
   still expose `&self` methods, which it must in order to be `Sync` behind an `Arc` in the
   facade. Task 5 therefore stores **key material** and constructs a `Siv` per call.
   Construction is a CMAC key schedule — cheap relative to the surrounding extraction.
2. **Ciphertext length is exactly `16 + plaintext.len()`** (a prepended synthetic IV).
   Token length tracks *padded* plaintext length, which is what Task 3's bucketing hides.
3. **`Siv::encrypt` errors if `plaintext.len() < 16`.** Task 3's PKCS#7 padding to 16-byte
   buckets guarantees a minimum of 16 bytes, so this is unreachable by construction — but
   the two tasks are coupled, and reordering them would break it.

`aes-siv` 0.7 builds on the pinned stable toolchain (`rust-version: 1.56`); confirmed by
the spike compiling and running.

---

## Global Constraints

- Workspace root `/home/jamin/Documents/hacienda-engine/`, edition 2021, resolver 2.
- Toolchain is **stable** (`rust-toolchain.toml`). Nothing here may reintroduce a nightly requirement.
- Build host is RAM-constrained (4 cores, ~3.8 GB). `.cargo/config.toml` pins `jobs = 2` — do not raise it. Prefer `cargo check -p hacienda-core` over full-workspace builds while iterating.
- New dependencies go in `[workspace.dependencies]` (alphabetical — `cargo sort` enforces it) and are inherited with `{ workspace = true }`.
- Pin `aes-siv = "0.7"`. **Do not use `0.8.0-rc.3`.** A compliance product does not ship a pre-release AEAD.
- Zero clippy warnings. `poly lint hacienda-core` must pass.
- No `.unwrap()` / `.expect()` on any path reachable from library code. Crypto failures are `Result`.
- Every step follows red-green: write the failing test, watch it fail, then implement.

---

## Design Decisions Settled Before Coding

These are consequences the implementer must not silently re-litigate.

**1. No truncation.** The token carries the full SIV output. Truncating discards the bits reveal needs (§12.3.1). Because SIV is injective under a fixed key, collisions are impossible and no birthday bound applies.

**2. Reveal returns the *normalised* value, not the original span.** Co-reference requires normalising before encryption, so `Alice@X.io` and `alice@x.io` must produce one token — which means only the normalised form survives. This is inherent, not a defect, but it must be documented on the public method: a right-of-access response gets the identity, not the original casing.

**3. Token length leaks a length bucket.** Deterministic encryption already leaks equality — that is the property being bought. Length is an *additional* leak, mitigated by PKCS#7 padding to 16-byte buckets before encryption.

**4. The default mode changes from `Pseudonymize` to `Mask`.** A default must be safe and must work without a secret. Combined with decision 5, leaving `Pseudonymize` as the default would make the crate unusable out of the box.

**5. `Pseudonymize` without a key is a construction error, never a fallback.** Falling back to masking would silently apply a weaker control than the config requests — the exact class of failure this phase exists to remove. Fail closed.

**6. The key never enters `HaciendaConfig`.** Config carries `key_id` only. Key material is resolved through `KeyResolver` from the environment or a keyfile, so no `--flag`, env override, or `config show` can print or substitute it (§6.3, §12.3.2).

**Wire format**, fixed here and treated as a compatibility surface from this point:

```text
[<CATEGORY>:<key_id>:<base32-nopad-ciphertext>]
```

`<CATEGORY>` is the uppercased `Display` of `PiiCategory`. Note this fixes a latent bug: the current code uses `{:?}` (Debug), so `Custom("EmployeeId")` renders as `CUSTOM("EMPLOYEEID")`. `Display` renders it `EMPLOYEEID`.

---

## Task 1: Dependency spike — confirm the AES-SIV API

**Files:**

- Modify: `Cargo.toml` (workspace `[workspace.dependencies]`)
- Modify: `hacienda-core/Cargo.toml`
- Create: `hacienda-core/src/redaction/pseudonym.rs`
- Modify: `hacienda-core/src/redaction/mod.rs`

**Interfaces:**

- Consumes: nothing
- Produces: a compiling round-trip proof; the real `aes-siv` 0.7 signatures recorded in this file

- [x] **Step 1: Add dependencies**

In workspace `Cargo.toml` `[workspace.dependencies]`, inserted alphabetically:

```toml
aes-siv = "0.7"
data-encoding = "2.11"
unicode-normalization = "0.1"
zeroize = { version = "1.9", features = ["zeroize_derive"] }
```

In `hacienda-core/Cargo.toml` `[dependencies]`, alphabetically: `aes-siv`, `data-encoding`, `unicode-normalization`, `zeroize`, each `{ workspace = true }`.

Run `cargo sort --workspace` and `cargo check -p hacienda-core`.

- [x] **Step 2: Write the round-trip spike as a failing test**

Create `redaction/pseudonym.rs` with only a test module. This exists to discover the real API, not to be kept:

```rust
#[cfg(test)]
mod spike {
    #[test]
    fn aes_siv_round_trips_with_associated_data() {
        // Confirm: key length, constructor name, encrypt/decrypt signatures,
        // and that encryption is deterministic across two separate instances.
        todo!("write against the real aes-siv 0.7 API")
    }
}
```

Add `pub mod pseudonym;` to `redaction/mod.rs`.

- [x] **Step 3: Make it pass, then record the API**

Consult `cargo doc -p aes-siv --open` or docs.rs for 0.7. Replace the `todo!` with a real round trip asserting:

- encrypting the same plaintext twice yields identical ciphertext (determinism — the whole premise)
- two independently constructed instances with the same key agree
- decrypt recovers the plaintext
- decrypt with *different* associated data fails

Then **update the "Assumed" table above** with the verified signatures so no later task guesses.

- [x] **Step 4: Verify**

`cargo test -p hacienda-core pseudonym` passes. `poly lint hacienda-core` clean.

---

## Task 2: Normalisation

**Files:**

- Modify: `hacienda-core/src/redaction/pseudonym.rs`

**Interfaces:**

- Consumes: `PiiCategory` (`crate::pii::types`)
- Produces: `fn normalize(category: &PiiCategory, text: &str) -> String`

Normalisation defines what counts as the same person and is part of the token contract — changing it later re-derives every token ever emitted (§12.3.1).

- [x] **Step 1: Write failing tests**

```rust
#[test]
fn should_collapse_case_and_whitespace_variants_to_one_form() {
    let c = PiiCategory::FullName;
    assert_eq!(normalize(&c, "Jean Dupont"), normalize(&c, "jean  dupont"));
    assert_eq!(normalize(&c, "  Jean Dupont  "), normalize(&c, "Jean Dupont"));
}

#[test]
fn should_not_merge_an_abbreviated_name_with_its_full_form() {
    // Merging two distinct identities is a worse failure than failing to link one.
    let c = PiiCategory::FullName;
    assert_ne!(normalize(&c, "Jean Dupont"), normalize(&c, "J. Dupont"));
}

#[test]
fn should_treat_canonically_equivalent_unicode_as_equal() {
    // "é" precomposed (U+00E9) vs decomposed (U+0065 U+0301)
    let c = PiiCategory::FullName;
    assert_eq!(normalize(&c, "Jos\u{00e9}"), normalize(&c, "Jose\u{0301}"));
}

#[test]
fn should_lowercase_an_email_domain_and_local_part() {
    let c = PiiCategory::Email;
    assert_eq!(normalize(&c, "Alice@X.IO"), normalize(&c, "alice@x.io"));
}

#[test]
fn should_ignore_separators_in_a_phone_number() {
    let c = PiiCategory::PhoneNumber;
    assert_eq!(normalize(&c, "+33 6 12 34 56 78"), normalize(&c, "+33612345678"));
}
```

- [x] **Step 2: Implement**

Base rule for every category: NFKC (`unicode_normalization::UnicodeNormalization::nfkc`), then `to_lowercase()`, then collapse internal whitespace runs to a single space, then trim.

Category-specific rules layered on top, per §12.3.1:

- `PhoneNumber` — strip everything except `+` and ASCII digits.
- `Email` — base rule is sufficient (lowercasing the local part is technically lossy per RFC 5321 but is what every real mail system does; document the choice).

Document on the function that `to_lowercase` is Unicode lowercasing, not full case-folding — sufficient for Latin/Cyrillic/Greek, imperfect for a few edge cases. Record it as a known limitation rather than pretending otherwise.

- [x] **Step 3: Verify**

All five tests pass. `poly lint hacienda-core` clean.

---

## Task 3: Padding

**Files:**

- Modify: `hacienda-core/src/redaction/pseudonym.rs`

**Interfaces:**

- Produces: `fn pad(bytes: &[u8]) -> Vec<u8>`, `fn unpad(bytes: &[u8]) -> Result<Vec<u8>, PseudonymError>`

- [x] **Step 1: Write failing tests**

```rust
#[test]
fn should_round_trip_every_length_across_a_bucket_boundary() {
    for len in 0..64 {
        let input = vec![b'a'; len];
        assert_eq!(unpad(&pad(&input)).unwrap(), input, "length {len}");
    }
}

#[test]
fn should_pad_to_a_multiple_of_the_bucket_size() {
    for len in 0..64 {
        assert_eq!(pad(&vec![b'a'; len]).len() % BUCKET, 0, "length {len}");
    }
}

#[test]
fn should_hide_exact_length_within_a_bucket() {
    assert_eq!(pad(b"a").len(), pad(b"abcdefghijklmno").len());
}

#[test]
fn should_reject_malformed_padding_rather_than_returning_garbage() {
    assert!(unpad(&[0u8; 16]).is_err());       // zero pad byte
    assert!(unpad(&[0xffu8; 16]).is_err());    // pad longer than block
    assert!(unpad(&[]).is_err());              // empty
}
```

- [x] **Step 2: Implement**

PKCS#7 with `const BUCKET: usize = 16`. A full-length input gets a whole extra block, which is what makes the scheme unambiguous. `unpad` validates every pad byte, not just the last.

- [x] **Step 3: Verify**

Tests pass, including the full 0..64 sweep.

---

## Task 4: Keys and the resolver seam

**Files:**

- Modify: `hacienda-core/src/redaction/pseudonym.rs`
- Modify: `hacienda-core/src/redaction/mod.rs` (add `PseudonymError` to `RedactionError`, or as its own error)

**Interfaces:**

- Produces: `KeyId`, `PseudonymKey`, `KeyResolver`, `EnvKeyResolver`, `PseudonymError`

- [x] **Step 1: Write failing tests**

```rust
#[test]
fn should_reject_a_key_id_that_would_break_token_parsing() {
    // ':' and ']' are token delimiters — a key id containing them is unparseable.
    assert!(KeyId::new("k1").is_ok());
    assert!(KeyId::new("k:1").is_err());
    assert!(KeyId::new("k]1").is_err());
    assert!(KeyId::new("").is_err());
    assert!(KeyId::new(&"k".repeat(17)).is_err());
}

#[test]
fn should_report_a_missing_key_rather_than_panicking() {
    let r = EnvKeyResolver::new();
    assert!(matches!(r.resolve(&KeyId::new("absent").unwrap()),
                     Err(PseudonymError::KeyNotFound { .. })));
}

#[test]
fn should_reject_key_material_of_the_wrong_length() { /* 32 bytes where 64 expected */ }

#[test]
fn should_not_expose_key_material_in_debug_output() {
    let key = PseudonymKey::from_bytes(KeyId::new("k1").unwrap(), &[7u8; 64]).unwrap();
    assert!(!format!("{key:?}").contains('7'));
}
```

- [x] **Step 2: Implement**

```rust
pub struct KeyId(String);                    // ^[a-z0-9_-]{1,16}$
pub struct PseudonymKey { id: KeyId, material: Zeroizing<[u8; 64]> }

pub trait KeyResolver: Send + Sync {
    fn active(&self) -> Result<PseudonymKey, PseudonymError>;
    fn resolve(&self, id: &KeyId) -> Result<PseudonymKey, PseudonymError>;
}
```

`EnvKeyResolver` reads `HACIENDA_PSEUDONYM_KEY_<ID uppercased>` as hex, and `HACIENDA_PSEUDONYM_ACTIVE_KEY` for the active id.

Hand-write `Debug` for `PseudonymKey` to print `PseudonymKey { id, material: <redacted> }`. Derive `Zeroize`/`ZeroizeOnDrop`. Deriving `Debug` here would print the crown-jewel secret into any error log — the test above exists to keep that from regressing.

`Send + Sync` on the trait is required: the facade is shared across an async runtime (CLAUDE.md async-and-concurrency).

- [x] **Step 3: Verify**

Tests pass. Confirm by inspection that no `Display`/`Debug`/`Serialize` impl can reach `material`.

---

## Task 5: The pseudonymiser

**Files:**

- Modify: `hacienda-core/src/redaction/pseudonym.rs`

**Interfaces:**

- Consumes: `normalize`, `pad`/`unpad`, `PseudonymKey`, `KeyResolver`
- Produces: `Pseudonymiser::new`, `::token`, `::reveal`

- [x] **Step 1: Write failing tests**

The behavioural contract of the whole phase:

```rust
#[test]
fn should_produce_the_same_token_for_the_same_value_every_time() { }

#[test]
fn should_produce_the_same_token_from_two_independently_built_instances() {
    // The cross-node consistency property. Without this, horizontal scaling is dead.
}

#[test]
fn should_produce_the_same_token_for_case_and_whitespace_variants() {
    // "Alice@X.io" and "alice@x.io" co-refer.
}

#[test]
fn should_produce_different_tokens_for_the_same_text_in_different_categories() {
    // Category is associated data — domain separation.
}

#[test]
fn should_produce_different_tokens_under_different_keys() { }

#[test]
fn should_reveal_the_normalised_value() {
    // Documents decision 2: reveal returns normalised, not original.
    let t = p.token(&PiiCategory::Email, "Alice@X.io").unwrap();
    assert_eq!(p.reveal(&t).unwrap(), "alice@x.io");
}

#[test]
fn should_reveal_a_token_minted_under_a_retired_key() {
    // Rotation is additive: k1 tokens stay revealable after k2 becomes active.
}

#[test]
fn should_mint_new_tokens_under_the_active_key_after_rotation() { }

#[test]
fn should_reject_a_token_whose_key_is_unknown() { }

#[test]
fn should_reject_a_tampered_token_rather_than_returning_garbage() {
    // AEAD authentication — flip a base32 char, expect Err.
}

#[test]
fn should_reject_a_malformed_token() {
    for bad in ["", "[EMAIL]", "[EMAIL:k1]", "not-a-token", "[EMAIL:k1:!!!]"] {
        assert!(p.reveal(bad).is_err(), "{bad}");
    }
}
```

- [x] **Step 2: Implement**

```rust
pub struct Pseudonymiser {
    active: PseudonymKey,
    retired: HashMap<KeyId, PseudonymKey>,
}

impl Pseudonymiser {
    pub fn new(resolver: &dyn KeyResolver, retired: &[KeyId]) -> Result<Self, PseudonymError>;
    pub fn token(&self, category: &PiiCategory, text: &str) -> Result<String, PseudonymError>;
    pub fn reveal(&self, token: &str) -> Result<String, PseudonymError>;
}
```

Per Task 1 finding 1, `Siv::encrypt` needs `&mut self`, so these `&self` methods build a
`Siv` from the stored key material on each call rather than caching one. Do not "optimise"
this into a `Mutex<Siv>`: that would serialise every span redaction in the process — the
same global-lock shape that §8 gap 4 already flags on the audit chain, reintroduced on a
far hotter path.

`token`: `normalize` → `pad` → SIV-encrypt with `category.to_string()` as associated data → `BASE32_NOPAD` → format `[{CATEGORY}:{key_id}:{b32}]`.

`reveal`: parse the three fields → select key (active, else retired, else `KeyNotFound`) → base32-decode → SIV-decrypt using the parsed category as associated data → `unpad` → UTF-8.

Use `data_encoding::BASE32_NOPAD` — no padding characters and no `=` to escape inside the token.

Rustdoc on `reveal` must state plainly that it returns the normalised form and that possession of the key de-pseudonymises the entire corpus.

- [x] **Step 3: Verify**

All twelve tests pass. `poly lint hacienda-core` clean.

---

## Task 6: Wire into `RedactionEngine` — fail closed

**Files:**

- Modify: `hacienda-core/src/redaction/engine.rs`
- Modify: `hacienda-core/src/redaction/types.rs`

**Interfaces:**

- Consumes: `Pseudonymiser`
- Produces: `RedactionEngine::new(...) -> Result<Self, RedactionError>` (signature change)

- [x] **Step 1: Write failing tests**

```rust
#[test]
fn should_refuse_to_build_a_pseudonymize_engine_without_a_key() {
    let cfg = RedactionConfig { mode: RedactionMode::Pseudonymize, ..Default::default() };
    assert!(RedactionEngine::new(cfg, None).is_err());
    // Never silently degrade to masking — that applies a weaker control than requested.
}

#[test]
fn should_default_to_mask_rather_than_a_mode_that_needs_a_secret() {
    assert_eq!(RedactionConfig::default().mode, RedactionMode::Mask);
}

#[test]
fn should_replace_a_span_with_a_keyed_reversible_token() {
    // Replaces the old assertion of "mail [EMAIL:****] ok".
    // Assert the shape and that reveal() recovers the value — not a hardcoded ciphertext,
    // which would only re-encode whatever the implementation happens to do.
}

#[test]
fn should_give_two_occurrences_of_one_value_the_same_token_in_one_document() {
    // Co-reference is the entire point.
}

#[test]
fn should_build_the_other_modes_without_a_key() {
    for m in [Mask, Hash, Remove, Custom] { assert!(RedactionEngine::new(cfg(m), None).is_ok()); }
}
```

- [x] **Step 2: Implement**

- `redaction/types.rs`: move `#[default]` from `Pseudonymize` to `Mask`; change `RedactionConfig::default()` to match; add `pub key_id: Option<String>` with `#[serde(default)]`. **Key material must not be added to this struct** — it is serialised and printed by `config show`.
- `redaction/engine.rs`: add `pseudonymiser: Option<Arc<Pseudonymiser>>` to the struct; change `new` to `new(config: RedactionConfig, pseudonymiser: Option<Arc<Pseudonymiser>>) -> Result<Self, RedactionError>`, returning `RedactionError::MissingPseudonymKey` when `mode == Pseudonymize && pseudonymiser.is_none()`.
- Replace the arm at `engine.rs:93` with a call to `Pseudonymiser::token`.

`replacement_for` returns `String`, but `token()` returns `Result`. Do **not** paper over this with `unwrap_or_else` back to a constant — that reinstates the bug. Either propagate by changing `replacement_for` to return `Result`, or (simpler, and adequate here) note that with a validated key held in `Arc`, the only residual failure is a UTF-8/padding invariant violation; if the chosen shape is infallible, add a comment justifying it. Prefer propagating.

- [x] **Step 3: Update the existing test**

`should_pseudonymize_a_span_with_a_category_placeholder` (`engine.rs:163`) asserts the old constant and **will** fail. Rewrite it as `should_pseudonymize_a_span_with_a_reversible_token` — do not delete it.

- [x] **Step 4: Verify**

`cargo test -p hacienda-core redaction` green.

---

## Task 7: Thread through pipeline and facade

**Files:**

- Modify: `hacienda-core/src/pii/pipeline.rs`
- Modify: `hacienda-core/src/facade.rs`
- Modify: `hacienda-core/src/pii/config.rs`

**Interfaces:**

- Consumes: `Pseudonymiser`, `KeyResolver`
- Produces: `PiiPipeline::with_pseudonymiser`, `HaciendaFacade::with_key_resolver`

- [x] **Step 1: Write failing tests**

```rust
#[tokio::test]
async fn should_redact_with_reversible_tokens_end_to_end() {
    // Facade → process → token in content → reveal recovers the address.
}

#[tokio::test]
async fn should_give_one_value_the_same_token_across_two_documents_in_a_batch() {
    // Cross-document co-reference through process_batch.
}

#[tokio::test]
async fn should_fail_construction_when_pseudonymize_is_configured_without_a_resolver() {
    // The fail-closed property must survive all the way to the public entry point.
}
```

- [x] **Step 2: Implement**

- `pii/pipeline.rs:99`: `RedactionEngine::new(config.redaction.clone(), pseudonymiser.clone())?` — `assemble` already returns `Result<_, PiiError>` and `PiiError` already has `#[from] RedactionError`, so `?` works with no new error plumbing.
- Add `PiiPipeline::with_pseudonymiser(config, detector, Option<Arc<Pseudonymiser>>)`. Keep `new` and `with_detector` delegating with `None`, so callers on non-`Pseudonymize` modes are unaffected.
- `facade.rs`: add `HaciendaFacade::with_key_resolver(config, &dyn KeyResolver)`, building the `Pseudonymiser` from `config.pii.redaction.key_id` plus any retired ids. Leave `new(config)` as-is — it now returns an error for `Pseudonymize`, which is the intended fail-closed behaviour.

- [x] **Step 3: Verify**

`cargo test -p hacienda-core` fully green — 116 tests passed before this phase; expect that plus the new ones, with one rewritten.

---

## Task 8: Document the break

**Files:**

- Modify: `CHANGELOG.md`
- Modify: `superpowers/specs/2026-07-28-hacienda-cli-api-integration-design.md` (§8 Gap 7)

- [x] **Step 1: CHANGELOG**

Two breaking changes, both required by CLAUDE.md's api-compatibility rule:

- `RedactionConfig::default().mode` changes `Pseudonymize` → `Mask`.
- `RedactionEngine::new` gains a parameter and returns `Result`.

And the behavioural change worth stating in its own right: `Pseudonymize` previously emitted a per-category constant — output produced by any earlier version is masked data, not pseudonymised data, and cannot be revealed. Anyone relying on the old output for a compliance claim needs to know that.

- [x] **Step 2: Mark Gap 7 closed in the spec**, referencing this plan.

- [x] **Step 3: Final verification**

```sh
cargo test -p hacienda-core -p hacienda
poly lint hacienda-core hacienda
poly fmt --check .
cargo audit          # new crypto dependencies — CLAUDE.md dependency-awareness
```

---

## Explicitly Out of Scope

| Deferred | Why |
|---|---|
| The `pii:reveal` HTTP endpoint and its capability check | §7 is Phase 3; `Pseudonymiser::reveal` is the library primitive it will call |
| A KMS-backed `KeyResolver` | The trait is the seam; env/file resolvers are enough until there is a server to deploy |
| Short non-reversible tokens (truncated keyed MAC) | §12.3.1 — a separate mode with a different legal character; needs demand first |
| Persisting tokens or key ids | Phase 1 owns storage |
| `RedactionAction::Custom` hard-coding `"template"` instead of the real template (`audit/entry.rs:17-28`) | Pre-existing, unrelated bug found while reading. Worth its own issue |

---

## Completion note — 2026-07-28

All eight tasks done. `cargo test -p hacienda-core -p hacienda` green at 171 (116 at phase
start), clippy clean, `poly fmt --check` clean. RustSec has no advisories for `aes-siv`,
`zeroize`, `data-encoding`, or `unicode-normalization` (`cargo-audit` is not installed on
this host; the advisory DB was queried directly).

### Deviations from the plan

* **`KeyId` charset narrowed to `[a-z0-9_]`.** The plan specified `[a-z0-9_-]`, but the id
  is uppercased into an environment variable name and `-` is not legal there. Folding it to
  `_` would have aliased `k-1` and `k_1` onto one key.
* **`PiiPipeline::with_pseudonymiser(config, pseudonymiser)`** mirrors `new` and loads the
  NER detector. The plan's three-argument shape mirrored `with_detector` instead, which
  would have silently disabled the model on the facade path;
  `with_detector_and_pseudonymiser` covers the explicit-detector case.
* **`Pseudonymiser::with_active`** was added so `RedactionConfig::key_id` can pin the
  minting key rather than inheriting whatever the resolver reports as active. Without it
  the config field had no effect.
* **`RedactionEngine::redact` returns `Result`**, which the plan left open. A custom
  category containing a token delimiter cannot be tokenised, and the alternatives were to
  skip the span (leaking it) or fall back to masking (the silent-degradation bug this
  phase exists to remove).

### A test defect this phase found in its own suite

`should_produce_different_tokens_under_different_keys` compared whole tokens, so the
differing `k1`/`k2` label satisfied it on its own. A mutation replacing the key material
with a constant passed all 167 tests. The suite now asserts on the base32 body, and that
mutation fails 3 tests; a second mutation fixing the associated data fails 6. Any future
test of a keyed property should assert on `body(...)`, not the whole token.

### Known sharp edge, deliberately left

`pii/ner.rs:139` maps arbitrary model labels to `PiiCategory::Custom(label)`. A label
containing `[`, `:` or `]` makes the whole document fail to redact. That is the intended
fail-closed behaviour — the alternative is emitting a document that looks redacted and
still contains the span — but the error surfaces at the first document rather than at
configuration time. Worth a validation pass over configured entity labels in a later phase.
