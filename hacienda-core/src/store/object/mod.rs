//! Object storage backend for presigned uploads (Phase 13 Task 4).
//!
//! Gated behind the `s3` feature. Only one implementation exists ([`s3::S3ObjectStore`]),
//! but the trait still exists as a seam — mirroring `DocumentVersionStore`/`PresetStore`'s
//! shape — so `hacienda-api`'s route layer can be tested against a fake without a live
//! bucket, and so a second backend (e.g. a local filesystem store) can be added later
//! without touching the API layer.

use async_trait::async_trait;
use std::collections::BTreeMap;
use std::time::Duration;

/// Module s3
pub mod s3;

/// Error type for object store operations.
#[derive(Debug, thiserror::Error)]
/// ObjectStoreError enum
pub enum ObjectStoreError {
    /// The outbound request to the object store failed (network error, TLS error, etc.)
    /// or the store returned a status this client doesn't know how to interpret.
    #[error("object storage request failed: {0}")]
    Request(String),
}

/// A presigned `PUT` the caller should hand to a client for a direct, out-of-band upload.
///
/// The client never talks to hacienda's own server for the upload itself — it PUTs bytes
/// straight to `url` (xberg's own pattern, confirmed in the Dart SDK's
/// `presignUpload`/`confirmUpload` pair; see spec §5). `required_headers` must be sent
/// verbatim by the client's PUT request — rusty-s3 signs them into the URL, so a mismatch
/// (e.g. a different `content-type`) makes the object store reject the upload with a
/// signature-mismatch error, not silently accept a different content type than presigned.
#[derive(Debug, Clone)]
/// PresignedPut struct
pub struct PresignedPut {
    pub url: String,
    pub required_headers: BTreeMap<String, String>,
    pub expires_at: chrono::DateTime<chrono::Utc>,
}

/// Metadata about an object confirmed to exist in the store.
#[derive(Debug, Clone, PartialEq, Eq)]
/// ObjectMetadata struct
pub struct ObjectMetadata {
    pub size_bytes: u64,
    pub content_type: Option<String>,
}

/// Trait for object storage used by the presigned-upload flow.
#[async_trait]
/// ObjectStore trait
pub trait ObjectStore: Send + Sync {
    /// Compute a presigned PUT URL for `key`, constrained to `content_type`.
    ///
    /// Pure computation (SigV4 signing) — no I/O, so this is not `async`.
    fn presign_put(&self, key: &str, content_type: &str, expires_in: Duration) -> PresignedPut;

    /// Confirm an object exists by issuing a signed `HEAD` request against hacienda's own
    /// bucket, keyed by `key` — never a client-supplied URL (spec §5's SSRF analysis).
    ///
    /// Returns `Ok(None)` if the store reports the object doesn't exist (a 404), so the
    /// caller can distinguish "not uploaded yet" from a genuine backend failure.
    async fn head(&self, key: &str) -> Result<Option<ObjectMetadata>, ObjectStoreError>;
}
