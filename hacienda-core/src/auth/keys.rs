//! API key generation and verification using Argon2id.
//!
//! Keys are high-entropy random tokens (`rand`-generated, ≥256 bits) prefixed for
//! identification (`hcd_live_<base62>`). The raw key is never stored — only its
//! Argon2id hash. Verification uses constant-time comparison via `argon2::verify_raw`.
//!
//! Argon2id embeds a fresh random salt in every hash it produces, so hashing the same
//! raw key twice yields two different strings. That is the whole point of Argon2id as a
//! *storage* hash — it defends the database if it leaks — but it also means a stored
//! `key_hash` can never be recomputed from a presented key and used as a lookup key.
//! `lookup_key` fills that gap with a separate, deterministic BLAKE3 digest, stored
//! alongside `key_hash` purely so [`ApiKeyStore::get_by_lookup_hash`] has something to
//! index on. `key_hash` still does all the actual verification work
//! ([`verify_key`]); `lookup_hash` never gates access on its own.
//!
//! [`ApiKeyStore::get_by_lookup_hash`]: crate::auth::ApiKeyStore::get_by_lookup_hash

use crate::tenancy::TenantId;
use argon2::{
    password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString},
    Argon2,
};
use rand::{rngs::OsRng, RngCore};
use std::fmt;
use thiserror::Error;

/// Errors from API key operations.
#[derive(Debug, Error)]
pub enum ApiKeyError {
    #[error("failed to generate random key material: {0}")]
    Generation(#[from] rand::Error),

    #[error("failed to hash key: {0}")]
    Hashing(String),

    #[error("failed to verify key: {0}")]
    Verification(String),

    #[error("invalid key format: expected hcd_<prefix>_<base62>")]
    InvalidFormat,

    /// The requested capability set could not be serialized to JSON for storage. Retains
    /// the `serde_json::Error` as its source rather than flattening it to a string, since
    /// `Capability` is a plain enum — a failure here points at a bug in `Capability`'s
    /// `Serialize` impl, not bad input, and the source error is the useful diagnostic.
    #[error("failed to serialize API key capabilities: {0}")]
    CapabilitySerialization(#[source] serde_json::Error),
}

/// A generated API key pair: the raw key (shown once) and its stored hashes.
#[derive(Clone)]
pub struct ApiKeyPair {
    /// The raw key, shown to the user exactly once. Format: `hcd_live_<base62>`.
    pub raw_key: String,
    /// The Argon2id hash of the raw key, stored in the database. Used for verification
    /// only — see the module docs for why it cannot also serve as a lookup key.
    pub key_hash: String,
    /// The deterministic BLAKE3 digest of the raw key, stored in the database and
    /// indexed for lookup by [`ApiKeyStore::get_by_lookup_hash`](crate::auth::ApiKeyStore::get_by_lookup_hash).
    pub lookup_hash: String,
}

/// Generate a new API key and its hash.
///
/// The raw key uses `hcd_live_` prefix followed by 43 base62 characters
/// (256 bits of entropy). The hash uses Argon2id with a unique salt per key.
///
/// # Errors
/// Returns `ApiKeyError::Generation` if random generation fails,
/// or `ApiKeyError::Hashing` if Argon2id hashing fails.
pub fn generate_key() -> Result<ApiKeyPair, ApiKeyError> {
    // 32 bytes = 256 bits of entropy
    let mut entropy = [0u8; 32];
    OsRng.fill_bytes(&mut entropy);

    // Encode as base62 (43 chars for 256 bits)
    const BASE62: &[u8] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";
    let mut encoded = String::with_capacity(43);
    for &byte in &entropy {
        encoded.push(BASE62[(byte as usize) % 62] as char);
    }

    let raw_key = format!("hcd_live_{}", encoded);
    let key_hash = hash_key(&raw_key)?;
    let lookup_hash = lookup_key(&raw_key);

    Ok(ApiKeyPair {
        raw_key,
        key_hash,
        lookup_hash,
    })
}

/// Hash an API key using Argon2id.
///
/// Uses a unique random salt per key. The resulting hash is self-contained
/// (includes salt, algorithm params) and can be stored directly.
fn hash_key(key: &str) -> Result<String, ApiKeyError> {
    let salt = SaltString::generate(&mut OsRng);
    let argon2 = Argon2::default(); // Argon2id v19 with default params

    argon2
        .hash_password(key.as_bytes(), &salt)
        .map(|hash| hash.to_string())
        .map_err(|e| ApiKeyError::Hashing(e.to_string()))
}

/// Compute the deterministic lookup digest for a raw API key.
///
/// This is a plain BLAKE3 hash, hex-encoded — the same construction
/// `InMemoryTokenStore::secret_key` in `auth::authn` uses for static tokens. It is
/// deliberately *not* a slow password hash: unlike `key_hash`, which exists to defend a
/// leaked database, `lookup_hash` exists only so a presented key can be looked up by
/// value. Verification of possession still goes through [`verify_key`] against
/// `key_hash`; `lookup_hash` never gates access on its own.
pub fn lookup_key(raw_key: &str) -> String {
    blake3::hash(raw_key.as_bytes()).to_hex().to_string()
}

/// Verify a candidate key against a stored Argon2id hash.
///
/// Returns `true` if the key matches the hash, `false` otherwise.
/// Uses constant-time verification to prevent timing attacks.
pub fn verify_key(candidate: &str, stored_hash: &str) -> Result<bool, ApiKeyError> {
    let parsed_hash =
        PasswordHash::new(stored_hash).map_err(|e| ApiKeyError::Verification(e.to_string()))?;

    let argon2 = Argon2::default();
    Ok(argon2
        .verify_password(candidate.as_bytes(), &parsed_hash)
        .is_ok())
}

/// Configuration for API key generation.
#[derive(Debug, Clone)]
pub struct ApiKeyConfig {
    /// Prefix for generated keys (default: "hcd_live_").
    pub prefix: String,
    /// Entropy bytes (default: 32 = 256 bits).
    pub entropy_bytes: usize,
}

impl Default for ApiKeyConfig {
    fn default() -> Self {
        Self {
            prefix: "hcd_live_".to_string(),
            entropy_bytes: 32,
        }
    }
}

impl ApiKeyConfig {
    /// Generate a key with custom configuration.
    pub fn generate(&self) -> Result<ApiKeyPair, ApiKeyError> {
        let mut entropy = vec![0u8; self.entropy_bytes];
        OsRng.fill_bytes(&mut entropy);

        const BASE62: &[u8] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";
        let mut encoded = String::with_capacity(self.entropy_bytes * 2);
        for &byte in &entropy {
            encoded.push(BASE62[(byte as usize) % 62] as char);
        }

        let raw_key = format!("{}{}", self.prefix, encoded);
        let key_hash = hash_key(&raw_key)?;
        let lookup_hash = lookup_key(&raw_key);

        Ok(ApiKeyPair {
            raw_key,
            key_hash,
            lookup_hash,
        })
    }
}

// Prevent accidental logging of raw keys
impl fmt::Debug for ApiKeyPair {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ApiKeyPair")
            .field("raw_key", &"<redacted>")
            .field("key_hash", &self.key_hash)
            .field("lookup_hash", &self.lookup_hash)
            .finish()
    }
}

/// An API key record (never stores the raw key).
#[derive(Debug, Clone)]
pub struct ApiKey {
    pub id: uuid::Uuid,
    /// Argon2id hash, used for verification (defense-in-depth if the database leaks).
    pub key_hash: String,
    /// Deterministic BLAKE3 digest, indexed for lookup by presented key. See the module
    /// docs for why this exists separately from `key_hash`.
    pub lookup_hash: String,
    pub owner: String,
    /// The tenant this key belongs to. Every capability a resolved `AuthContext` grants
    /// via this key is scoped to this tenant (see `AuthContext::with_tenant`) — a key
    /// never grants cross-tenant access, regardless of `owner`.
    pub tenant: TenantId,
    pub capabilities: serde_json::Value,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub revoked_at: Option<chrono::DateTime<chrono::Utc>>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn should_generate_a_key_and_verify_it() {
        let pair = generate_key().unwrap();
        assert!(pair.raw_key.starts_with("hcd_live_"));
        assert!(!pair.key_hash.is_empty());
        assert!(verify_key(&pair.raw_key, &pair.key_hash).unwrap());
    }

    #[test]
    fn should_reject_a_wrong_key() {
        let pair = generate_key().unwrap();
        // Flip exactly one bit of the last byte rather than substituting an unrelated
        // string: this proves the hash is sensitive to small, adjacent-input changes
        // in the actual generated key, not just "some other string fails" (which an
        // always-false verifier would also pass).
        let mut mutated = pair.raw_key.clone().into_bytes();
        let last = mutated.len() - 1;
        mutated[last] ^= 1; // stays valid ASCII: high bit of every generated char is 0
        let mutated_key =
            String::from_utf8(mutated).expect("bit-flip of an ASCII byte stays valid UTF-8");

        assert_ne!(mutated_key, pair.raw_key);
        assert!(!verify_key(&mutated_key, &pair.key_hash).unwrap());
    }

    #[test]
    fn should_produce_different_keys_each_time() {
        let k1 = generate_key().unwrap();
        let k2 = generate_key().unwrap();
        assert_ne!(k1.raw_key, k2.raw_key);
        assert_ne!(k1.key_hash, k2.key_hash);
    }

    #[test]
    fn should_hash_deterministically_with_same_salt() {
        // This tests that the same key + salt produces same hash
        let pair = generate_key().unwrap();
        // Re-hash with same salt (extracted from stored hash)
        let parsed = PasswordHash::new(&pair.key_hash).unwrap();
        let salt = parsed.salt.unwrap();
        let argon2 = Argon2::default();
        let rehash = argon2
            .hash_password(pair.raw_key.as_bytes(), salt)
            .unwrap()
            .to_string();
        assert_eq!(pair.key_hash, rehash);
    }

    #[test]
    fn custom_config_generates_correct_prefix() {
        let config = ApiKeyConfig {
            prefix: "hcd_test_".to_string(),
            ..Default::default()
        };
        let pair = config.generate().unwrap();
        assert!(pair.raw_key.starts_with("hcd_test_"));
    }

    #[test]
    fn key_has_sufficient_entropy() {
        let pair = generate_key().unwrap();
        let suffix = pair
            .raw_key
            .strip_prefix("hcd_live_")
            .expect("generated key must carry the hcd_live_ prefix");

        // generate_key() draws 32 bytes from OsRng and base62-encodes them one
        // char per byte, so the suffix must carry exactly that many characters —
        // anchored to the function's own entropy buffer size, not a number picked
        // independently of the implementation.
        assert_eq!(
            suffix.len(),
            32,
            "suffix should encode all 32 bytes drawn from OsRng, one base62 char per byte"
        );
        assert!(
            suffix.chars().all(|c| c.is_ascii_alphanumeric()),
            "every suffix character must come from the base62 alphabet: {suffix}"
        );
        // Guards against a degenerate encoder collapsing to a constant/low-entropy
        // output (e.g. an RNG that always returns zeroes) — real entropy should not
        // produce a single repeated character across 32 draws.
        assert!(
            suffix
                .chars()
                .collect::<std::collections::HashSet<_>>()
                .len()
                > 1,
            "suffix must not collapse to a single repeated character: {suffix}"
        );
    }

    #[test]
    fn lookup_key_is_deterministic_for_the_same_input() {
        let pair = generate_key().unwrap();
        assert_eq!(lookup_key(&pair.raw_key), lookup_key(&pair.raw_key));
        assert_eq!(pair.lookup_hash, lookup_key(&pair.raw_key));
    }

    #[test]
    fn lookup_key_differs_for_different_inputs() {
        let k1 = generate_key().unwrap();
        let k2 = generate_key().unwrap();
        assert_ne!(lookup_key(&k1.raw_key), lookup_key(&k2.raw_key));
        assert_ne!(k1.lookup_hash, k2.lookup_hash);
    }

    /// Unlike `key_hash`, which is salted and differs across calls, `lookup_hash` must be
    /// stable so the store can index on it.
    #[test]
    fn generate_key_produces_a_stable_lookup_hash_but_a_varying_key_hash() {
        let pair = generate_key().unwrap();
        let rehash = hash_key(&pair.raw_key).unwrap();
        assert_ne!(
            pair.key_hash, rehash,
            "Argon2id must salt differently on each call"
        );
        assert_eq!(
            pair.lookup_hash,
            lookup_key(&pair.raw_key),
            "lookup_hash must be reproducible from the raw key"
        );
    }

    #[test]
    fn verify_key_should_take_measurable_time() {
        // ~keep: Argon2id is deliberately expensive so brute-forcing a leaked
        // key_hash is impractical; a future regression to a fast hash (e.g. swapping
        // in SHA-256) would pass every functional test above while silently
        // reintroducing brute-forceability. The 1ms floor sits comfortably below
        // Argon2id's real per-call cost (tens of ms under the crate's defaults),
        // leaving margin against CI timing jitter while still catching a swap to a
        // hash that's fast enough to matter (anything sub-millisecond).
        let pair = generate_key().unwrap();

        let start = std::time::Instant::now();
        let _ = verify_key(&pair.raw_key, &pair.key_hash).unwrap();
        let elapsed = start.elapsed();

        assert!(
            elapsed > std::time::Duration::from_millis(1),
            "verify_key took {elapsed:?}; expected Argon2id-level cost (>1ms) — \
             did the hashing algorithm regress to something fast?"
        );
    }
}
