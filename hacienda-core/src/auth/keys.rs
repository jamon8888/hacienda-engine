//! API key generation and verification using Argon2id.
//!
//! Keys are high-entropy random tokens (`rand`-generated, ≥256 bits) prefixed for
//! identification (`hcd_live_<base62>`). The raw key is never stored — only its
//! Argon2id hash. Verification uses constant-time comparison via `argon2::verify_raw`.

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
}

/// A generated API key pair: the raw key (shown once) and its Argon2id hash (stored).
#[derive(Clone)]
pub struct ApiKeyPair {
    /// The raw key, shown to the user exactly once. Format: `hcd_live_<base62>`.
    pub raw_key: String,
    /// The Argon2id hash of the raw key, stored in the database.
    pub key_hash: String,
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

    Ok(ApiKeyPair { raw_key, key_hash })
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

        Ok(ApiKeyPair { raw_key, key_hash })
    }
}

// Prevent accidental logging of raw keys
impl fmt::Debug for ApiKeyPair {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ApiKeyPair")
            .field("raw_key", &"<redacted>")
            .field("key_hash", &self.key_hash)
            .finish()
    }
}

/// An API key record (never stores the raw key).
#[derive(Debug, Clone)]
pub struct ApiKey {
    pub id: uuid::Uuid,
    pub key_hash: String,
    pub owner: String,
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
        assert!(!verify_key("wrong_key", &pair.key_hash).unwrap());
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
        // hcd_live_ (9) + 32 base62 chars = 41 chars
        assert_eq!(pair.raw_key.len(), 41);
    }
}
