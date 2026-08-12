//! Keyed, deterministic, reversible pseudonymisation.
//!
//! See `superpowers/plans/2026-07-28-phase0-pseudonymisation.md` and §12.3 of
//! `superpowers/specs/2026-07-28-hacienda-cli-api-integration-design.md`.

use crate::pii::types::PiiCategory;
use crate::tenancy::{TenantCtx, TenantId, DEFAULT_TENANT};
use aes_siv::siv::Aes256Siv;
use aes_siv::{Key, KeyInit};
use data_encoding::{BASE32_NOPAD, HEXLOWER_PERMISSIVE};
use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use thiserror::Error;
use unicode_normalization::UnicodeNormalization;
use zeroize::Zeroizing;

/// Failures from deriving or reversing a pseudonym token.
///
/// Deliberately coarse on the cryptographic paths. `aes-siv` returns an opaque error by
/// design, and distinguishing "wrong key" from "tampered ciphertext" in a message handed
/// back to a caller would be an oracle. Callers get "this token could not be revealed".
#[derive(Debug, Error)]
pub enum PseudonymError {
    #[error("pseudonym token padding is malformed")]
    MalformedPadding,

    #[error(
        "invalid pseudonym key id '{id}': expected 1-16 characters from [a-z0-9_] \
         (lowercase, no ':' or ']', usable as an environment variable suffix)"
    )]
    InvalidKeyId { id: String },

    #[error(
        "no pseudonym key is configured for id '{id}' \
         (expected environment variable {variable})"
    )]
    KeyNotFound { id: String, variable: String },

    #[error(
        "no active pseudonym key is configured \
         (set HACIENDA_PSEUDONYM_ACTIVE_KEY to the id of the key to mint tokens under)"
    )]
    NoActiveKey,

    #[error("pseudonym key '{id}' is not {KEY_BYTES}-byte lowercase hex")]
    MalformedKeyMaterial { id: String },

    #[error("pseudonym key '{id}' is {actual} bytes, expected {KEY_BYTES}")]
    WrongKeyLength { id: String, actual: usize },

    #[error("not a pseudonym token: expected [CATEGORY:key_id:data]")]
    MalformedToken,

    /// Covers a wrong key, a relabelled category, and a tampered ciphertext alike.
    ///
    /// Distinguishing them would tell an attacker which of their guesses was closer.
    #[error("pseudonym token could not be revealed (wrong key, altered, or forged)")]
    UnreadableToken,

    #[error("category '{category}' cannot appear in a pseudonym token: {reason}")]
    UnsupportedCategory { category: String, reason: String },
}

/// Collapse spelling variants of one value onto a single canonical form.
///
/// This decides **which mentions co-refer**, and it is part of the pseudonym token
/// contract: two values that normalise alike receive the same token, and changing this
/// function later re-derives every token ever emitted (spec §12.3.1). Treat it as a
/// versioned format, not a string utility.
///
/// The bias is deliberate. Failing to link `Jean Dupont` with `J. Dupont` costs recall;
/// merging two different people under one pseudonym is a re-identification failure that
/// reads as a *successful* redaction. When in doubt, do not merge.
///
/// # Known limitation
///
/// Uses Unicode lowercasing ([`str::to_lowercase`]), not full case-folding. This is
/// correct for Latin, Greek and Cyrillic, and imperfect for a handful of scripts where
/// the two differ (e.g. Cherokee). Widening it later is a token-format change.
pub(crate) fn normalize(category: &PiiCategory, text: &str) -> String {
    match category {
        // Separators and spacing are presentation, not identity: `+33 6 12 34 56 78`
        // and `+33612345678` are one subscriber.
        PiiCategory::PhoneNumber => text
            .chars()
            .filter(|c| *c == '+' || c.is_ascii_digit())
            .collect(),
        // The base rule covers everything else, including Email. Lowercasing an email
        // local part is technically lossy under RFC 5321 (which permits case-sensitive
        // local parts) but matches what every real mail system does, and treating
        // `Alice@x.io` and `alice@x.io` as two people would be the worse error.
        _ => base_normalize(text),
    }
}

/// NFKC, lowercase, collapse internal whitespace runs, trim.
fn base_normalize(text: &str) -> String {
    text.nfkc()
        .collect::<String>()
        .to_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Padding granularity, in bytes.
///
/// Two jobs. It coarsens the length that a token discloses (spec §12.3.1: deterministic
/// encryption already leaks equality, and leaking exact length on top of that would
/// narrow the candidate set for a short value). And because the smallest padded output
/// is one full bucket, it keeps every plaintext at or above the 16-byte minimum that
/// `aes-siv` requires.
pub(crate) const BUCKET: usize = 16;

/// PKCS#7-pad to the next whole [`BUCKET`].
///
/// Input that is already a whole number of buckets gains an entire extra bucket. That
/// looks wasteful and is not: it is what makes the padding unambiguous, because the
/// final byte can then always be read as the pad length.
pub(crate) fn pad(bytes: &[u8]) -> Vec<u8> {
    let pad_len = BUCKET - (bytes.len() % BUCKET);
    let mut out = Vec::with_capacity(bytes.len() + pad_len);
    out.extend_from_slice(bytes);
    // `pad_len` is in 1..=BUCKET, so it always fits in a u8 for any sane BUCKET.
    out.resize(bytes.len() + pad_len, pad_len as u8);
    out
}

/// Strip PKCS#7 padding, rejecting anything malformed.
///
/// Validates the whole pad run rather than trusting the final byte. A caller reaching
/// this with bad input has either hit a bug or is feeding us a forged token, and
/// returning a silently truncated value would be worse than an error.
pub(crate) fn unpad(bytes: &[u8]) -> Result<Vec<u8>, PseudonymError> {
    if bytes.is_empty() || !bytes.len().is_multiple_of(BUCKET) {
        return Err(PseudonymError::MalformedPadding);
    }
    let pad_len = *bytes.last().ok_or(PseudonymError::MalformedPadding)? as usize;
    if pad_len == 0 || pad_len > BUCKET || pad_len > bytes.len() {
        return Err(PseudonymError::MalformedPadding);
    }
    let split = bytes.len() - pad_len;
    if bytes[split..].iter().any(|b| *b as usize != pad_len) {
        return Err(PseudonymError::MalformedPadding);
    }
    Ok(bytes[..split].to_vec())
}

/// Length of an AES-256-SIV key, in bytes.
///
/// Sixty-four, not thirty-two. SIV splits the key into two independent halves: one drives
/// the CMAC that derives the synthetic IV, the other the CTR encryption. Handing it 32
/// bytes is not "AES-256 with a shorter key", it is a type error the crate will reject.
pub const KEY_BYTES: usize = 64;

/// Maximum length of a [`KeyId`], in characters.
const KEY_ID_MAX: usize = 16;

/// Environment variable naming the key that new tokens are minted under.
pub const ACTIVE_KEY_VAR: &str = "HACIENDA_PSEUDONYM_ACTIVE_KEY";

/// Prefix of the environment variable holding a key's hex-encoded material.
///
/// The full name is this prefix followed by the uppercased [`KeyId`].
pub const KEY_VAR_PREFIX: &str = "HACIENDA_PSEUDONYM_KEY_";

/// Identifies which key a token was minted under.
///
/// The id travels in the clear inside every token, which is what makes rotation additive:
/// a new key changes only what *new* tokens say, and old tokens keep naming the key that
/// can still read them (spec §12.3.1). It is not secret.
///
/// The charset is deliberately narrow — `[a-z0-9_]`, 1 to 16 characters:
///
/// * `:` and `]` are token structure. A key id containing either produces tokens that
///   cannot be parsed back.
/// * `-` is excluded even though it reads naturally as a separator, because the id is
///   uppercased into an environment variable name and `-` is not legal there. Folding it
///   to `_` would silently alias `k-1` and `k_1` onto one key.
/// * Uppercase is rejected rather than folded, so that exactly one spelling of an id maps
///   to one key and audit records cannot disagree about which key was used.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct KeyId(String);

impl KeyId {
    /// Validate and wrap a key id.
    ///
    /// # Errors
    ///
    /// [`PseudonymError::InvalidKeyId`] if `id` is empty, longer than 16 characters, or
    /// contains anything outside `[a-z0-9_]`.
    pub fn new(id: &str) -> Result<Self, PseudonymError> {
        let legal = !id.is_empty()
            && id.len() <= KEY_ID_MAX
            && id
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_');
        if legal {
            Ok(Self(id.to_string()))
        } else {
            Err(PseudonymError::InvalidKeyId { id: id.to_string() })
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Name of the environment variable expected to hold this key's material, for
    /// `tenant`.
    ///
    /// The default tenant keeps the pre-S1, unsuffixed name so an existing
    /// single-tenant deployment's environment does not need to change. Any other
    /// tenant gets an explicit suffix — see [`sanitize_env_suffix`].
    fn env_var(&self, tenant: &TenantId) -> String {
        if tenant.as_str() == DEFAULT_TENANT {
            format!("{KEY_VAR_PREFIX}{}", self.0.to_uppercase())
        } else {
            format!(
                "{KEY_VAR_PREFIX}{}__{}",
                self.0.to_uppercase(),
                sanitize_env_suffix(tenant.as_str())
            )
        }
    }
}

/// Deterministically fold a tenant id into a legal environment-variable-name suffix:
/// uppercase, with every byte outside `[A-Z0-9_]` replaced by `_`.
///
/// Lossy by construction — this resolver is documented as dev/test-only once a
/// KMS-backed resolver ships (S2). Two tenant ids that fold to the same suffix would
/// collide here, which is an acceptable limitation for local development, never for
/// production key material.
fn sanitize_env_suffix(tenant: &str) -> String {
    tenant
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' {
                c.to_ascii_uppercase()
            } else {
                '_'
            }
        })
        .collect()
}

impl std::fmt::Display for KeyId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// A pseudonymisation key: an id, and the secret that reverses every token bearing it.
///
/// # Security
///
/// The material is the crown jewel. Anyone holding it can de-pseudonymise the entire
/// corpus that key ever touched, offline, without access to this system. It is wrapped in
/// [`Zeroizing`] so it is wiped on drop, [`Debug`] is hand-written to redact it, and no
/// `Display`, `Clone`, or `Serialize` impl exists. Do not add one.
pub struct PseudonymKey {
    id: KeyId,
    material: Zeroizing<[u8; KEY_BYTES]>,
}

impl PseudonymKey {
    /// Build a key from raw material.
    ///
    /// # Errors
    ///
    /// [`PseudonymError::WrongKeyLength`] unless `material` is exactly [`KEY_BYTES`]
    /// long. Short keys are rejected rather than stretched: silently padding a 32-byte
    /// secret would produce a key whose second half is a known constant.
    pub fn from_bytes(id: KeyId, material: &[u8]) -> Result<Self, PseudonymError> {
        let material: [u8; KEY_BYTES] =
            material
                .try_into()
                .map_err(|_| PseudonymError::WrongKeyLength {
                    id: id.0.clone(),
                    actual: material.len(),
                })?;
        Ok(Self {
            id,
            material: Zeroizing::new(material),
        })
    }

    pub fn id(&self) -> &KeyId {
        &self.id
    }

    /// The raw key material.
    ///
    /// Crate-internal on purpose: only [`Pseudonymiser`] has a reason to see it, and
    /// exporting it would put the secret one `.material()` call away from any caller.
    pub(crate) fn material(&self) -> &[u8; KEY_BYTES] {
        &self.material
    }
}

/// Redacts the material. Deriving [`Debug`] here would print the secret into the first
/// log line that formats a config struct; `key_tests` guards against that regressing.
impl std::fmt::Debug for PseudonymKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PseudonymKey")
            .field("id", &self.id)
            .field("material", &"<redacted>")
            .finish()
    }
}

/// Where key material comes from.
///
/// A trait rather than a concrete lookup because the secret store is the embedder's
/// choice — environment, a KMS, a sealed file. It is deliberately *not* part of the
/// config precedence chain (spec §6.3): a key must never be readable from a config file
/// that someone might commit.
///
/// `Send + Sync` is required: the facade holding a resolver is shared across the async
/// runtime.
pub trait KeyResolver: Send + Sync {
    /// The key that new tokens are minted under, for `ctx.tenant` (S1 spec §4).
    ///
    /// # Errors
    ///
    /// [`PseudonymError::NoActiveKey`] if none is configured for that tenant. This must
    /// never fall back to a default or generated key: a per-process random key would
    /// produce tokens that no other node agrees with and that nothing can ever reveal.
    /// It must also never fall back to another tenant's key — see decision D-S1-3: each
    /// tenant's material is independent, never derived from a shared master.
    fn active(&self, ctx: &TenantCtx) -> Result<PseudonymKey, PseudonymError>;

    /// The key with a given id, for revealing tokens minted before a rotation, scoped to
    /// `ctx.tenant`.
    ///
    /// # Errors
    ///
    /// [`PseudonymError::KeyNotFound`] if the id is not configured for that tenant.
    fn resolve(&self, ctx: &TenantCtx, id: &KeyId) -> Result<PseudonymKey, PseudonymError>;
}

/// How [`EnvKeyResolver`] reads a named value; `None` means "not configured".
///
/// Takes the variable name rather than a whole key so a secret store only ever has to
/// answer that one question, and so an unset value stays distinguishable from an empty one.
type Lookup = dyn Fn(&str) -> Option<String> + Send + Sync;

/// Resolves keys from environment-variable-style names.
///
/// * [`ACTIVE_KEY_VAR`] holds the id of the active key.
/// * `HACIENDA_PSEUDONYM_KEY_<ID>` (see [`KEY_VAR_PREFIX`], id uppercased) holds that
///   key's material as [`KEY_BYTES`] bytes of hex.
///
/// Hex rather than base64 so the value is unambiguous when copied by hand and contains no
/// characters a shell will interpret.
pub struct EnvKeyResolver {
    lookup: Box<Lookup>,
}

impl EnvKeyResolver {
    /// Read from the process environment.
    pub fn new() -> Self {
        Self::with_lookup(|name| std::env::var(name).ok())
    }

    /// Read from an arbitrary source using the same naming scheme.
    ///
    /// The process environment is global mutable state, inherited by children and visible
    /// in `/proc`. This seam lets an embedder keep the naming convention while sourcing
    /// the values from a secret manager instead — and lets tests exercise resolution
    /// without mutating a variable that every other concurrent test can see.
    pub fn with_lookup(lookup: impl Fn(&str) -> Option<String> + Send + Sync + 'static) -> Self {
        Self {
            lookup: Box::new(lookup),
        }
    }
}

impl Default for EnvKeyResolver {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for EnvKeyResolver {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("EnvKeyResolver")
    }
}

impl KeyResolver for EnvKeyResolver {
    fn active(&self, ctx: &TenantCtx) -> Result<PseudonymKey, PseudonymError> {
        let variable = active_key_var(&ctx.tenant);
        let id = (self.lookup)(&variable).ok_or(PseudonymError::NoActiveKey)?;
        self.resolve(ctx, &KeyId::new(id.trim())?)
    }

    fn resolve(&self, ctx: &TenantCtx, id: &KeyId) -> Result<PseudonymKey, PseudonymError> {
        let variable = id.env_var(&ctx.tenant);
        let encoded = (self.lookup)(&variable).ok_or_else(|| PseudonymError::KeyNotFound {
            id: id.0.clone(),
            variable,
        })?;
        // Permissive on case only. The error carries the id, never the value.
        //
        // `Zeroizing` on the intermediate matters: `decode` heap-allocates the raw key,
        // and without this the plaintext secret would be left in freed memory after
        // `from_bytes` copies it out.
        let material = Zeroizing::new(
            HEXLOWER_PERMISSIVE
                .decode(encoded.trim().as_bytes())
                .map_err(|_| PseudonymError::MalformedKeyMaterial { id: id.0.clone() })?,
        );
        PseudonymKey::from_bytes(id.clone(), &material)
    }
}

/// Name of the environment variable holding the active key's id, for `tenant`.
///
/// Same backward-compatibility rule as [`KeyId::env_var`]: the default tenant keeps the
/// unsuffixed [`ACTIVE_KEY_VAR`] name.
fn active_key_var(tenant: &TenantId) -> String {
    if tenant.as_str() == DEFAULT_TENANT {
        ACTIVE_KEY_VAR.to_string()
    } else {
        format!("{ACTIVE_KEY_VAR}__{}", sanitize_env_suffix(tenant.as_str()))
    }
}

/// Render a category into the form used both as the token's label and as the AEAD's
/// associated data.
///
/// One function for both jobs on purpose. The label is what `reveal` parses back out and
/// feeds to the cipher as associated data, so if the two ever diverged, every token would
/// fail to decrypt.
///
/// # Errors
///
/// [`PseudonymError::UnsupportedCategory`] if the rendered form is empty or contains a
/// token delimiter. Only [`PiiCategory::Custom`] can trigger this; the fixed variants are
/// all plain identifiers.
pub(crate) fn category_label(category: &PiiCategory) -> Result<String, PseudonymError> {
    let label = category.to_string().to_uppercase();
    let reason = if label.is_empty() {
        "it is empty"
    } else if label.contains(':') || label.contains(']') || label.contains('[') {
        "it contains a token delimiter ('[', ':' or ']')"
    } else {
        return Ok(label);
    };
    Err(PseudonymError::UnsupportedCategory {
        category: category.to_string(),
        reason: reason.to_string(),
    })
}

/// Mints and reverses pseudonym tokens.
///
/// Tokens have the form `[CATEGORY:key_id:base32]`, where the base32 body is
/// `AES-256-SIV(key, aad = CATEGORY, pad(normalize(text)))`. See spec §12.3.
///
/// The construction is deterministic, which is the point: the same value yields the same
/// token on every node and in every run, so a reader can follow one pseudonymous person
/// through a corpus. The cost is that a token discloses *equality* — an observer learns
/// that two redacted spans held the same value. Padding to [`BUCKET`]-byte blocks keeps it
/// from also disclosing the exact length.
///
/// # Security
///
/// This is pseudonymisation under GDPR Art. 4(5), not anonymisation. The output is still
/// personal data, and anyone holding the key can reverse the whole corpus offline. Do not
/// treat a pseudonymised document as safe to publish.
pub struct Pseudonymiser {
    /// The tenant this instance was resolved for (S1). Not itself a secret — kept only
    /// so [`Pseudonymiser::key`]'s error can name the environment variable a missing
    /// key would need, and so [`Debug`] can show which tenant a cached instance belongs
    /// to without exposing key material.
    tenant: TenantId,
    active: PseudonymKey,
    retired: HashMap<KeyId, PseudonymKey>,
}

impl Pseudonymiser {
    /// Load the active key and any retired keys still needed to read old tokens, for
    /// `ctx.tenant`.
    ///
    /// All keys are resolved eagerly. Discovering at admission that a retired key is
    /// unreadable is a configuration error someone can fix; discovering it when a data
    /// subject exercises a right of access is an incident (decision D-S1-4).
    ///
    /// # Errors
    ///
    /// [`PseudonymError::NoActiveKey`] or [`PseudonymError::KeyNotFound`] if any listed
    /// key is missing, and the parse errors above if any is malformed.
    pub fn new(
        ctx: &TenantCtx,
        resolver: &dyn KeyResolver,
        retired: &[KeyId],
    ) -> Result<Self, PseudonymError> {
        Self::from_keys(ctx, resolver.active(ctx)?, resolver, retired)
    }

    /// As [`Pseudonymiser::new`], but with the active key named explicitly.
    ///
    /// Lets configuration pin the minting key rather than inheriting whatever the
    /// resolver reports as active. That matters during a rotation: the environment can be
    /// updated across a fleet first, and each service switched over deliberately, instead
    /// of every node changing minting key the instant a shared variable changes.
    ///
    /// # Errors
    ///
    /// As [`Pseudonymiser::new`], with [`PseudonymError::KeyNotFound`] for `active` too.
    pub fn with_active(
        ctx: &TenantCtx,
        resolver: &dyn KeyResolver,
        active: KeyId,
        retired: &[KeyId],
    ) -> Result<Self, PseudonymError> {
        let active = resolver.resolve(ctx, &active)?;
        Self::from_keys(ctx, active, resolver, retired)
    }

    fn from_keys(
        ctx: &TenantCtx,
        active: PseudonymKey,
        resolver: &dyn KeyResolver,
        retired: &[KeyId],
    ) -> Result<Self, PseudonymError> {
        let retired = retired
            .iter()
            // The active key is already held; listing it as retired too is harmless
            // configuration noise, not an error worth refusing to start over.
            .filter(|id| **id != *active.id())
            .map(|id| Ok((id.clone(), resolver.resolve(ctx, id)?)))
            .collect::<Result<HashMap<_, _>, PseudonymError>>()?;
        Ok(Self {
            tenant: ctx.tenant.clone(),
            active,
            retired,
        })
    }

    /// Mint the token for `text` in `category`, under the active key.
    ///
    /// `text` is normalised first (see [`normalize`]), so spelling variants of one value
    /// converge on one token. Two different categories never share a token for the same
    /// text.
    ///
    /// # Errors
    ///
    /// [`PseudonymError::UnsupportedCategory`] for a [`PiiCategory::Custom`] whose name
    /// contains a token delimiter.
    pub fn token(&self, category: &PiiCategory, text: &str) -> Result<String, PseudonymError> {
        let label = category_label(category)?;
        let plaintext = Zeroizing::new(pad(normalize(category, text).as_bytes()));
        let ciphertext = Self::cipher(&self.active)
            .encrypt([label.as_bytes()], plaintext.as_slice())
            // Unreachable in practice: SIV encryption fails only on a plaintext under 16
            // bytes, which `pad` makes impossible. Erroring beats `expect` in a library.
            .map_err(|_| PseudonymError::UnreadableToken)?;
        Ok(format!(
            "[{label}:{}:{}]",
            self.active.id(),
            BASE32_NOPAD.encode(&ciphertext)
        ))
    }

    /// Recover the value behind a token.
    ///
    /// Returns the **normalised** form, not the original spelling: casing and spacing are
    /// collapsed before encryption and are not recoverable. `Alice@X.io` reveals as
    /// `alice@x.io`.
    ///
    /// # Security
    ///
    /// This is the de-pseudonymisation path. Reaching it requires the key that minted the
    /// token, and that key reverses every token ever minted under it — not just this one.
    /// Calls belong behind an authorisation check and in the audit log.
    ///
    /// # Errors
    ///
    /// [`PseudonymError::MalformedToken`] if the input is not shaped like a token,
    /// [`PseudonymError::KeyNotFound`] if it names a key this instance does not hold, and
    /// [`PseudonymError::UnreadableToken`] if authentication fails — which covers a wrong
    /// key, an altered category label, and a forged body without distinguishing them.
    pub fn reveal(&self, token: &str) -> Result<String, PseudonymError> {
        let body = token
            .strip_prefix('[')
            .and_then(|t| t.strip_suffix(']'))
            .ok_or(PseudonymError::MalformedToken)?;
        let mut fields = body.splitn(3, ':');
        let (label, key_id, encoded) = match (fields.next(), fields.next(), fields.next()) {
            (Some(l), Some(k), Some(e)) if !l.is_empty() => (l, k, e),
            _ => return Err(PseudonymError::MalformedToken),
        };

        let key_id = KeyId::new(key_id).map_err(|_| PseudonymError::MalformedToken)?;
        let key = self.key(&key_id)?;
        let ciphertext = BASE32_NOPAD
            .decode(encoded.as_bytes())
            .map_err(|_| PseudonymError::MalformedToken)?;

        let padded = Zeroizing::new(
            Self::cipher(key)
                .decrypt([label.as_bytes()], &ciphertext)
                .map_err(|_| PseudonymError::UnreadableToken)?,
        );
        let plaintext = Zeroizing::new(unpad(&padded)?);
        // Every token this crate mints encodes normalised UTF-8, so a non-UTF-8 result
        // means a forged body that happened to authenticate — impossible without the key.
        String::from_utf8(plaintext.to_vec()).map_err(|_| PseudonymError::UnreadableToken)
    }

    fn key(&self, id: &KeyId) -> Result<&PseudonymKey, PseudonymError> {
        if id == self.active.id() {
            return Ok(&self.active);
        }
        self.retired
            .get(id)
            .ok_or_else(|| PseudonymError::KeyNotFound {
                id: id.to_string(),
                variable: id.env_var(&self.tenant),
            })
    }

    /// Build a cipher for one operation.
    ///
    /// `Siv::encrypt` takes `&mut self`, so a cached instance would make `Pseudonymiser`
    /// either non-`Sync` or dependent on a mutex. A mutex here would serialise every span
    /// redaction in the process — the same global-lock shape that §8 gap 4 already flags
    /// on the audit chain, on a far hotter path. Constructing a `Siv` is two AES key
    /// schedules; that is the cheaper end of the trade.
    fn cipher(key: &PseudonymKey) -> Aes256Siv {
        Aes256Siv::new(Key::<Aes256Siv>::from_slice(key.material()))
    }
}

/// Names the tenant and keys held, never key material.
impl std::fmt::Debug for Pseudonymiser {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut retired: Vec<&str> = self.retired.keys().map(KeyId::as_str).collect();
        retired.sort_unstable();
        f.debug_struct("Pseudonymiser")
            .field("tenant", &self.tenant)
            .field("active", &self.active.id())
            .field("retired", &retired)
            .finish()
    }
}

/// Per-tenant cache of resolved [`Pseudonymiser`]s.
///
/// A [`Pseudonymiser`] eagerly resolves and caches all its key material at construction
/// (decision D-S1-4: fail at admission, never at first use). Multi-tenancy widens
/// "construction" from "process startup" to "tenant admission" — this registry is where
/// that admission happens, once per tenant, so a request never pays a resolver
/// round-trip and a tenant with broken key configuration is refused before it can
/// process any content, not discovered mid-request.
#[derive(Default)]
pub struct TenantPseudonymiserRegistry {
    by_tenant: RwLock<HashMap<TenantId, Arc<Pseudonymiser>>>,
}

impl TenantPseudonymiserRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Resolve and cache `ctx.tenant`'s pseudonymiser.
    ///
    /// Idempotent, and safe to call again for an already-admitted tenant — the existing
    /// entry is replaced with a freshly resolved one. That is the path a key rotation
    /// takes (see P3): re-admit the tenant after the resolver reports a new active key.
    ///
    /// # Errors
    ///
    /// Whatever `resolver` returns for a missing or malformed key — see
    /// [`Pseudonymiser::new`] / [`Pseudonymiser::with_active`].
    pub fn admit(
        &self,
        ctx: &TenantCtx,
        resolver: &dyn KeyResolver,
        active: Option<KeyId>,
        retired: &[KeyId],
    ) -> Result<(), PseudonymError> {
        let pseudonymiser = match active {
            Some(id) => Pseudonymiser::with_active(ctx, resolver, id, retired)?,
            None => Pseudonymiser::new(ctx, resolver, retired)?,
        };
        self.by_tenant
            .write()
            .expect("pseudonymiser registry lock poisoned")
            .insert(ctx.tenant.clone(), Arc::new(pseudonymiser));
        Ok(())
    }

    /// The admitted pseudonymiser for `tenant`, if any.
    ///
    /// `None` means the tenant was never admitted (or its last admission attempt
    /// failed and nothing was cached) — a caller must treat this the same as
    /// "pseudonymization is not configured for this tenant", never invent a key.
    pub fn get(&self, tenant: &TenantId) -> Option<Arc<Pseudonymiser>> {
        self.by_tenant
            .read()
            .expect("pseudonymiser registry lock poisoned")
            .get(tenant)
            .cloned()
    }

    /// Whether `tenant` has been admitted.
    pub fn contains(&self, tenant: &TenantId) -> bool {
        self.by_tenant
            .read()
            .expect("pseudonymiser registry lock poisoned")
            .contains_key(tenant)
    }
}

/// Names the admitted tenants, never key material.
impl std::fmt::Debug for TenantPseudonymiserRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut tenants: Vec<String> = self
            .by_tenant
            .read()
            .expect("pseudonymiser registry lock poisoned")
            .keys()
            .map(TenantId::to_string)
            .collect();
        tenants.sort_unstable();
        f.debug_struct("TenantPseudonymiserRegistry")
            .field("admitted_tenants", &tenants)
            .finish()
    }
}

#[cfg(test)]
mod normalize_tests {
    use super::normalize;
    use crate::pii::types::PiiCategory;

    #[test]
    fn should_collapse_case_and_whitespace_variants_to_one_form() {
        let c = PiiCategory::FullName;
        assert_eq!(normalize(&c, "Jean Dupont"), normalize(&c, "jean  dupont"));
        assert_eq!(
            normalize(&c, "  Jean Dupont  "),
            normalize(&c, "Jean Dupont")
        );
    }

    #[test]
    fn should_not_merge_an_abbreviated_name_with_its_full_form() {
        // Merging two distinct identities is a worse failure than failing to link one.
        let c = PiiCategory::FullName;
        assert_ne!(normalize(&c, "Jean Dupont"), normalize(&c, "J. Dupont"));
    }

    #[test]
    fn should_treat_canonically_equivalent_unicode_as_equal() {
        // Precomposed U+00E9 vs decomposed U+0065 U+0301 — the same name typed on
        // two different keyboards must not become two different people.
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
        assert_eq!(
            normalize(&c, "+33 6 12 34 56 78"),
            normalize(&c, "+33612345678")
        );
    }

    #[test]
    fn should_keep_distinct_phone_numbers_distinct() {
        let c = PiiCategory::PhoneNumber;
        assert_ne!(normalize(&c, "+33612345678"), normalize(&c, "+33612345679"));
    }
}

#[cfg(test)]
mod padding_tests {
    use super::{pad, unpad, BUCKET};

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
    fn should_never_produce_a_block_shorter_than_the_cipher_minimum() {
        // aes-siv rejects plaintexts under 16 bytes; padding is what makes that
        // unreachable. See the Task 1 findings in the plan.
        for len in 0..64 {
            assert!(pad(&vec![b'a'; len]).len() >= BUCKET, "length {len}");
        }
    }

    #[test]
    fn should_hide_exact_length_within_a_bucket() {
        assert_eq!(pad(b"a").len(), pad(b"abcdefghijklmno").len());
    }

    #[test]
    fn should_reject_malformed_padding_rather_than_returning_garbage() {
        assert!(unpad(&[]).is_err(), "empty");
        assert!(unpad(&[0u8; 16]).is_err(), "zero pad byte");
        assert!(unpad(&[0xffu8; 16]).is_err(), "pad longer than a block");
        assert!(unpad(&[b'a'; 15]).is_err(), "not a whole number of blocks");

        // Last byte claims 4 bytes of padding, but the run is inconsistent.
        let mut inconsistent = [4u8; 16];
        inconsistent[13] = 9;
        assert!(unpad(&inconsistent).is_err(), "inconsistent pad run");
    }
}

#[cfg(test)]
mod key_tests {
    use super::{EnvKeyResolver, KeyId, KeyResolver, PseudonymError, PseudonymKey, KEY_BYTES};
    use crate::tenancy::{ActorId, TenantCtx};

    fn ctx() -> TenantCtx {
        TenantCtx::default_tenant(ActorId::new("test"))
    }

    #[test]
    fn should_reject_a_key_id_that_would_break_token_parsing() {
        assert!(KeyId::new("k1").is_ok());
        assert!(KeyId::new("k_1").is_ok());
        assert!(KeyId::new("k:1").is_err(), "field separator");
        assert!(KeyId::new("k]1").is_err(), "token terminator");
        assert!(KeyId::new("").is_err(), "empty");
        assert!(KeyId::new(&"k".repeat(17)).is_err(), "too long");
    }

    #[test]
    fn should_reject_a_key_id_that_is_not_a_legal_environment_variable_suffix() {
        // `-` reads as a natural id separator but cannot appear in an env var name, and
        // folding it to `_` would collide `k-1` with `k_1` onto one key.
        assert!(KeyId::new("k-1").is_err());
        assert!(
            KeyId::new("K1").is_err(),
            "case is not significant, so reject it"
        );
    }

    #[test]
    fn should_report_a_missing_key_rather_than_panicking() {
        let resolver = EnvKeyResolver::with_lookup(|_| None);
        let id = KeyId::new("absent").unwrap();
        assert!(matches!(
            resolver.resolve(&ctx(), &id),
            Err(PseudonymError::KeyNotFound { .. })
        ));
    }

    #[test]
    fn should_report_a_missing_active_key_rather_than_inventing_one() {
        let resolver = EnvKeyResolver::with_lookup(|_| None);
        assert!(matches!(
            resolver.active(&ctx()),
            Err(PseudonymError::NoActiveKey)
        ));
    }

    #[test]
    fn should_reject_key_material_of_the_wrong_length() {
        let id = KeyId::new("k1").unwrap();
        // AES-256-SIV takes a 64-byte key (two 32-byte halves), not 32. Accepting a
        // short key by padding it would silently halve the security of every token.
        assert!(PseudonymKey::from_bytes(id.clone(), &[7u8; 32]).is_err());
        assert!(PseudonymKey::from_bytes(id.clone(), &[7u8; 65]).is_err());
        assert!(PseudonymKey::from_bytes(id, &[7u8; KEY_BYTES]).is_ok());
    }

    #[test]
    fn should_reject_key_material_that_is_not_hex() {
        let resolver = EnvKeyResolver::with_lookup(|name| {
            (name == "HACIENDA_PSEUDONYM_KEY_K1").then(|| "zzzz".to_string())
        });
        let id = KeyId::new("k1").unwrap();
        assert!(matches!(
            resolver.resolve(&ctx(), &id),
            Err(PseudonymError::MalformedKeyMaterial { .. })
        ));
    }

    #[test]
    fn should_resolve_a_key_by_the_documented_environment_variable_name() {
        let hex = "07".repeat(KEY_BYTES);
        let resolver = EnvKeyResolver::with_lookup(move |name| match name {
            "HACIENDA_PSEUDONYM_ACTIVE_KEY" => Some("k1".to_string()),
            "HACIENDA_PSEUDONYM_KEY_K1" => Some(hex.clone()),
            _ => None,
        });
        let active = resolver.active(&ctx()).unwrap();
        assert_eq!(active.id().as_str(), "k1");
        assert_eq!(active.material(), &[7u8; KEY_BYTES]);
    }

    #[test]
    fn should_resolve_a_non_default_tenant_from_its_suffixed_environment_variable() {
        let hex = "07".repeat(KEY_BYTES);
        let resolver = EnvKeyResolver::with_lookup(move |name| match name {
            "HACIENDA_PSEUDONYM_ACTIVE_KEY__ACME" => Some("k1".to_string()),
            "HACIENDA_PSEUDONYM_KEY_K1__ACME" => Some(hex.clone()),
            _ => None,
        });
        let ctx = TenantCtx::new(crate::tenancy::TenantId::new("acme"), ActorId::new("u"));
        let active = resolver.active(&ctx).unwrap();
        assert_eq!(active.id().as_str(), "k1");
    }

    #[test]
    fn default_tenant_and_non_default_tenant_read_different_environment_variables() {
        // A deployment migrating a single pre-existing tenant to `default` (spec §8)
        // must keep working against its existing, unsuffixed env vars.
        let resolver = EnvKeyResolver::with_lookup(|name| match name {
            "HACIENDA_PSEUDONYM_ACTIVE_KEY" => Some("k1".to_string()),
            "HACIENDA_PSEUDONYM_KEY_K1" => Some("07".repeat(KEY_BYTES)),
            _ => None,
        });
        assert!(resolver.active(&ctx()).is_ok());
        let other = TenantCtx::new(crate::tenancy::TenantId::new("acme"), ActorId::new("u"));
        assert!(matches!(
            resolver.active(&other),
            Err(PseudonymError::NoActiveKey)
        ));
    }

    #[test]
    fn should_not_expose_key_material_in_debug_output() {
        // This key de-pseudonymises an entire corpus. A derived `Debug` would print it
        // into the first error log that formats a config struct.
        let key = PseudonymKey::from_bytes(KeyId::new("k1").unwrap(), &[7u8; KEY_BYTES]).unwrap();
        let rendered = format!("{key:?}");
        assert!(!rendered.contains('7'), "{rendered}");
        assert!(rendered.contains("redacted"), "{rendered}");
        assert!(rendered.contains("k1"), "the id is not secret: {rendered}");
    }
}

#[cfg(test)]
mod pseudonymiser_tests {
    use super::{
        EnvKeyResolver, KeyId, PseudonymError, Pseudonymiser, ACTIVE_KEY_VAR, KEY_BYTES,
        KEY_VAR_PREFIX,
    };
    use crate::pii::types::PiiCategory;
    use crate::tenancy::{ActorId, TenantCtx};

    fn ctx() -> TenantCtx {
        TenantCtx::default_tenant(ActorId::new("test"))
    }

    /// A resolver holding `k1` and `k2` with distinct material, active id as given.
    fn resolver(active: &'static str) -> EnvKeyResolver {
        EnvKeyResolver::with_lookup(move |name| match name {
            ACTIVE_KEY_VAR => Some(active.to_string()),
            "HACIENDA_PSEUDONYM_KEY_K1" => Some("07".repeat(KEY_BYTES)),
            "HACIENDA_PSEUDONYM_KEY_K2" => Some("a9".repeat(KEY_BYTES)),
            _ => None,
        })
    }

    fn k(id: &str) -> KeyId {
        KeyId::new(id).unwrap()
    }

    fn built(active: &'static str, retired: &[&str]) -> Pseudonymiser {
        let retired: Vec<KeyId> = retired.iter().map(|r| k(r)).collect();
        Pseudonymiser::new(&ctx(), &resolver(active), &retired).unwrap()
    }

    #[test]
    fn should_produce_the_same_token_for_the_same_value_every_time() {
        let p = built("k1", &[]);
        let a = p.token(&PiiCategory::Email, "alice@example.com").unwrap();
        let b = p.token(&PiiCategory::Email, "alice@example.com").unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn should_produce_the_same_token_from_two_independently_built_instances() {
        // Cross-node consistency. Without it, horizontal scaling produces a corpus in
        // which the same person carries a different pseudonym per worker.
        let node_a = built("k1", &[]);
        let node_b = built("k1", &[]);
        assert_eq!(
            node_a
                .token(&PiiCategory::Email, "alice@example.com")
                .unwrap(),
            node_b
                .token(&PiiCategory::Email, "alice@example.com")
                .unwrap()
        );
    }

    #[test]
    fn should_produce_the_same_token_for_case_and_whitespace_variants() {
        let p = built("k1", &[]);
        assert_eq!(
            p.token(&PiiCategory::Email, "Alice@X.io").unwrap(),
            p.token(&PiiCategory::Email, "  alice@x.io ").unwrap()
        );
    }

    #[test]
    fn should_produce_different_tokens_for_the_same_text_in_different_categories() {
        // The category is associated data, so a value that appears both as a name and as
        // a username does not collapse into one identity.
        let p = built("k1", &[]);
        assert_ne!(
            p.token(&PiiCategory::Username, "same value").unwrap(),
            p.token(&PiiCategory::FullName, "same value").unwrap()
        );
    }

    #[test]
    fn should_produce_different_tokens_under_different_keys() {
        assert_ne!(
            built("k1", &[])
                .token(&PiiCategory::Email, "a@b.io")
                .unwrap(),
            built("k2", &[])
                .token(&PiiCategory::Email, "a@b.io")
                .unwrap()
        );
    }

    /// The base32 ciphertext, with the category and key-id labels stripped.
    ///
    /// Comparing whole tokens is not enough to prove the key is used: `[EMAIL:k1:X]` and
    /// `[EMAIL:k2:X]` differ in the label alone. A mutation that ignored key material
    /// entirely passed this whole suite until the assertions below looked at the body.
    fn body(token: &str) -> &str {
        token.trim_end_matches(']').rsplit_once(':').expect(token).1
    }

    #[test]
    fn should_encrypt_the_body_differently_under_different_keys() {
        let under_k1 = built("k1", &[])
            .token(&PiiCategory::Email, "a@b.io")
            .unwrap();
        let under_k2 = built("k2", &[])
            .token(&PiiCategory::Email, "a@b.io")
            .unwrap();
        assert_ne!(body(&under_k1), body(&under_k2));
    }

    #[test]
    fn should_refuse_to_reveal_a_token_whose_key_id_matches_but_material_does_not() {
        // The sharpest test that key material is really in play: same id, same category,
        // same value — only the bytes behind `k1` differ.
        let ours = built("k1", &[])
            .token(&PiiCategory::Email, "a@b.io")
            .unwrap();
        let impostor = Pseudonymiser::new(
            &ctx(),
            &EnvKeyResolver::with_lookup(|name| match name {
                ACTIVE_KEY_VAR => Some("k1".to_string()),
                "HACIENDA_PSEUDONYM_KEY_K1" => Some("5c".repeat(KEY_BYTES)),
                _ => None,
            }),
            &[],
        )
        .unwrap();
        assert_ne!(
            body(&ours),
            body(&impostor.token(&PiiCategory::Email, "a@b.io").unwrap())
        );
        assert!(matches!(
            impostor.reveal(&ours),
            Err(PseudonymError::UnreadableToken)
        ));
    }

    #[test]
    fn should_encrypt_the_body_differently_per_category() {
        // As above: the category label alone makes the tokens differ, so compare the
        // ciphertext to prove the associated data actually reaches the cipher.
        let p = built("k1", &[]);
        assert_ne!(
            body(&p.token(&PiiCategory::Username, "same value").unwrap()),
            body(&p.token(&PiiCategory::FullName, "same value").unwrap())
        );
    }

    #[test]
    fn should_encrypt_the_body_differently_after_a_rotation() {
        let before = built("k1", &[])
            .token(&PiiCategory::Email, "a@b.io")
            .unwrap();
        let after = built("k2", &["k1"])
            .token(&PiiCategory::Email, "a@b.io")
            .unwrap();
        assert_ne!(body(&before), body(&after));
    }

    #[test]
    fn should_label_a_token_with_its_category_and_key() {
        let p = built("k1", &[]);
        let t = p.token(&PiiCategory::Email, "a@b.io").unwrap();
        assert!(t.starts_with("[EMAIL:k1:"), "{t}");
        assert!(t.ends_with(']'), "{t}");
    }

    #[test]
    fn should_reveal_the_normalised_value_not_the_original_spelling() {
        // Reveal undoes pseudonymisation, not normalisation: the original casing and
        // spacing are destroyed before encryption and are not recoverable.
        let p = built("k1", &[]);
        let t = p.token(&PiiCategory::Email, "Alice@X.io").unwrap();
        assert_eq!(p.reveal(&t).unwrap(), "alice@x.io");
    }

    #[test]
    fn should_reveal_a_token_minted_under_a_retired_key() {
        // Rotation is additive. A k1 token must stay readable after k2 becomes active,
        // otherwise rotating a key silently destroys the corpus's reversibility.
        let old = built("k1", &[])
            .token(&PiiCategory::Email, "a@b.io")
            .unwrap();
        let after_rotation = built("k2", &["k1"]);
        assert_eq!(after_rotation.reveal(&old).unwrap(), "a@b.io");
    }

    #[test]
    fn should_mint_new_tokens_under_the_active_key_after_rotation() {
        let after_rotation = built("k2", &["k1"]);
        let t = after_rotation.token(&PiiCategory::Email, "a@b.io").unwrap();
        assert!(t.starts_with("[EMAIL:k2:"), "{t}");
    }

    #[test]
    fn should_reject_a_token_whose_key_is_unknown() {
        let old = built("k1", &[])
            .token(&PiiCategory::Email, "a@b.io")
            .unwrap();
        // k1 was rotated out and *not* retained.
        let p = built("k2", &[]);
        assert!(matches!(
            p.reveal(&old),
            Err(PseudonymError::KeyNotFound { .. })
        ));
    }

    #[test]
    fn should_reject_a_tampered_token_rather_than_returning_garbage() {
        let p = built("k1", &[]);
        let t = p.token(&PiiCategory::Email, "alice@example.com").unwrap();
        let head = "[EMAIL:k1:".len();
        let mut bytes = t.into_bytes();
        // Flip one base32 character of the ciphertext body.
        bytes[head] = if bytes[head] == b'A' { b'B' } else { b'A' };
        let tampered = String::from_utf8(bytes).unwrap();
        assert!(matches!(
            p.reveal(&tampered),
            Err(PseudonymError::UnreadableToken)
        ));
    }

    #[test]
    fn should_reject_a_token_whose_category_label_was_swapped() {
        // The category is authenticated associated data, so relabelling a token is
        // detected rather than silently decrypting under the wrong domain.
        let p = built("k1", &[]);
        let t = p.token(&PiiCategory::Email, "alice@example.com").unwrap();
        let swapped = t.replace("[EMAIL:", "[USERNAME:");
        assert!(matches!(
            p.reveal(&swapped),
            Err(PseudonymError::UnreadableToken)
        ));
    }

    #[test]
    fn should_reject_a_malformed_token() {
        let p = built("k1", &[]);
        for bad in [
            "",
            "[EMAIL]",
            "[EMAIL:k1]",
            "not-a-token",
            "[EMAIL:k1:!!!]",
            "[EMAIL:k1:AAAA]",
            "[EMAIL:K1:AAAA]",
            "EMAIL:k1:AAAA]",
            "[EMAIL:k1:AAAA",
            "[:k1:AAAA]",
        ] {
            assert!(p.reveal(bad).is_err(), "expected rejection of {bad:?}");
        }
    }

    #[test]
    fn should_refuse_a_category_that_cannot_be_encoded_in_a_token() {
        // `Custom` carries arbitrary text; one containing a delimiter would mint a token
        // that cannot be parsed back. Fail at mint time rather than emit it.
        let p = built("k1", &[]);
        let bad = PiiCategory::Custom("has:colon".to_string());
        assert!(matches!(
            p.token(&bad, "value"),
            Err(PseudonymError::UnsupportedCategory { .. })
        ));
    }

    #[test]
    fn should_round_trip_a_custom_category() {
        let p = built("k1", &[]);
        let c = PiiCategory::Custom("employee_id".to_string());
        let t = p.token(&c, "E-1234").unwrap();
        assert!(t.starts_with("[EMPLOYEE_ID:k1:"), "{t}");
        assert_eq!(p.reveal(&t).unwrap(), "e-1234");
    }

    #[test]
    fn should_not_expose_key_material_in_debug_output() {
        let rendered = format!("{:?}", built("k1", &["k1"]));
        assert!(!rendered.contains('7'), "{rendered}");
    }

    #[test]
    fn should_fail_to_build_when_no_active_key_is_configured() {
        let empty = EnvKeyResolver::with_lookup(|_| None);
        assert!(matches!(
            Pseudonymiser::new(&ctx(), &empty, &[]),
            Err(PseudonymError::NoActiveKey)
        ));
    }

    #[test]
    fn should_fail_to_build_when_a_retired_key_is_missing() {
        // Starting up with an unreadable retired key would mean discovering the corpus is
        // partly irreversible only when someone exercises a right of access.
        assert!(matches!(
            Pseudonymiser::new(&ctx(), &resolver("k1"), &[k("gone")]),
            Err(PseudonymError::KeyNotFound { .. })
        ));
    }

    #[test]
    fn two_tenants_admitted_from_the_same_env_resolver_get_independent_keys() {
        // Registry-level analogue of S1 spec §9's
        // `two_tenants_same_value_get_different_tokens`: two tenants pointed at the
        // same resolver still end up with distinct key material, because each supplies
        // its own suffixed env vars.
        let resolver = EnvKeyResolver::with_lookup(|name| match name {
            "HACIENDA_PSEUDONYM_ACTIVE_KEY__TENANT_A" => Some("k1".to_string()),
            "HACIENDA_PSEUDONYM_KEY_K1__TENANT_A" => Some("07".repeat(KEY_BYTES)),
            "HACIENDA_PSEUDONYM_ACTIVE_KEY__TENANT_B" => Some("k1".to_string()),
            "HACIENDA_PSEUDONYM_KEY_K1__TENANT_B" => Some("a9".repeat(KEY_BYTES)),
            _ => None,
        });
        let registry = super::TenantPseudonymiserRegistry::new();
        let ctx_a = TenantCtx::new(crate::tenancy::TenantId::new("tenant_a"), ActorId::new("u"));
        let ctx_b = TenantCtx::new(crate::tenancy::TenantId::new("tenant_b"), ActorId::new("u"));
        registry.admit(&ctx_a, &resolver, None, &[]).unwrap();
        registry.admit(&ctx_b, &resolver, None, &[]).unwrap();

        let a = registry.get(&ctx_a.tenant).unwrap();
        let b = registry.get(&ctx_b.tenant).unwrap();
        assert_ne!(
            a.token(&PiiCategory::Email, "same@value.io").unwrap(),
            b.token(&PiiCategory::Email, "same@value.io").unwrap()
        );
    }

    /// S1 spec §9: `retired_key_of_one_tenant_reveals_nothing_of_another`.
    #[test]
    fn retired_key_of_one_tenant_reveals_nothing_of_another() {
        // Two tenants each name a key "k1", but with independent material via separate
        // suffixed env vars — a shared key *id* must never imply shared key *material*
        // across tenants. Tenant A's token, minted under k1 and still revealable after
        // k1 is retired in favour of k2, must stay unreadable to tenant B even though B
        // also has a key literally named k1.
        let resolver = EnvKeyResolver::with_lookup(|name| match name {
            "HACIENDA_PSEUDONYM_ACTIVE_KEY__TENANT_A" => Some("k2".to_string()),
            "HACIENDA_PSEUDONYM_KEY_K1__TENANT_A" => Some("07".repeat(KEY_BYTES)),
            "HACIENDA_PSEUDONYM_KEY_K2__TENANT_A" => Some("11".repeat(KEY_BYTES)),
            "HACIENDA_PSEUDONYM_ACTIVE_KEY__TENANT_B" => Some("k1".to_string()),
            "HACIENDA_PSEUDONYM_KEY_K1__TENANT_B" => Some("a9".repeat(KEY_BYTES)),
            _ => None,
        });
        let ctx_a = TenantCtx::new(crate::tenancy::TenantId::new("tenant_a"), ActorId::new("u"));
        let ctx_b = TenantCtx::new(crate::tenancy::TenantId::new("tenant_b"), ActorId::new("u"));

        // Mint under k1 while it is (explicitly) the minting key, then build a second
        // instance where k1 has been retired in favour of k2 — rotation is additive, so
        // the token must still be revealable by tenant A.
        let a_minting = Pseudonymiser::with_active(&ctx_a, &resolver, k("k1"), &[]).unwrap();
        let token = a_minting.token(&PiiCategory::Email, "a@b.io").unwrap();
        let a_after_rotation = Pseudonymiser::new(&ctx_a, &resolver, &[k("k1")]).unwrap();
        assert_eq!(a_after_rotation.reveal(&token).unwrap(), "a@b.io");

        // Tenant B has a key with the same id "k1", but it is a different key: different
        // material, resolved from tenant B's own suffixed env vars.
        let b = Pseudonymiser::new(&ctx_b, &resolver, &[]).unwrap();
        assert!(
            b.reveal(&token).is_err(),
            "tenant B must not be able to reveal a token minted under tenant A's key material"
        );
    }

    /// S1 spec §9: `missing_tenant_key_fails_admission_not_first_request` (D-S1-4).
    #[test]
    fn missing_tenant_key_fails_admission_not_first_request() {
        // A tenant admitted with unresolvable key material must fail at admission time,
        // not surface only when the first token()/reveal() call for that tenant happens
        // — discovering a corpus is unreadable at the moment of a right-of-access request
        // is exactly the failure mode D-S1-4 forbids.
        let empty = EnvKeyResolver::with_lookup(|_| None);
        let ctx = TenantCtx::new(
            crate::tenancy::TenantId::new("ghost_tenant"),
            ActorId::new("u"),
        );

        let registry = super::TenantPseudonymiserRegistry::new();
        let result = registry.admit(&ctx, &empty, None, &[]);

        assert!(
            result.is_err(),
            "admission must fail immediately when the tenant's key material cannot be resolved"
        );
        assert!(
            !registry.contains(&ctx.tenant),
            "a failed admission must not leave a usable (or partially usable) entry behind"
        );
    }

    #[test]
    fn registry_get_is_none_for_a_tenant_that_was_never_admitted() {
        let registry = super::TenantPseudonymiserRegistry::new();
        let unknown = crate::tenancy::TenantId::new("ghost");
        assert!(registry.get(&unknown).is_none());
        assert!(!registry.contains(&unknown));
    }

    #[test]
    fn should_expose_the_key_variable_names_it_reads() {
        assert_eq!(ACTIVE_KEY_VAR, "HACIENDA_PSEUDONYM_ACTIVE_KEY");
        assert_eq!(KEY_VAR_PREFIX, "HACIENDA_PSEUDONYM_KEY_");
    }
}
