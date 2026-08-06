//! S3-compatible [`ObjectStore`] implementation.
//!
//! Works against AWS S3 and any S3-compatible endpoint (MinIO, R2, GCS's S3-compat API)
//! by pointing `S3Config::endpoint` at the deployment's own storage. Presigning is pure
//! computation via `rusty-s3` (no HTTP client of its own — see the workspace `Cargo.toml`
//! comment on why not `aws-sdk-s3`); the only outbound request this module makes itself
//! is the `HEAD` in `head()`, used by `confirm_upload` to verify a client's PUT actually
//! landed.

use async_trait::async_trait;
use rusty_s3::{Bucket, BucketError, Credentials, S3Action, UrlStyle};
use std::collections::BTreeMap;
use std::time::Duration;

use super::{ObjectMetadata, ObjectStore, ObjectStoreError, PresignedPut};

/// Configuration for an S3-compatible object store.
///
/// No `HaciendaConfig`/CLI wiring exists yet for this (same as `DocumentVersionStore`'s
/// and `PresetStore`'s Postgres pool: Phase 9 never wired `DATABASE_URL` into
/// `hacienda-cli` either — see that phase's plan notes). Construct this directly; a
/// caller wanting env-var-driven config reads the vars itself, matching the existing
/// `EnvKeyResolver` pattern in `redaction/pseudonym.rs`.
#[derive(Debug, Clone)]
pub struct S3Config {
    pub endpoint: url::Url,
    pub bucket: String,
    pub region: String,
    pub access_key: String,
    pub secret_key: String,
    /// `true` for path-style URLs (`https://host/bucket/key`, what MinIO and most
    /// self-hosted S3-compatible servers expect), `false` for virtual-host style
    /// (`https://bucket.host/key`, AWS's default).
    pub path_style: bool,
}

/// Error constructing an [`S3ObjectStore`] from an [`S3Config`].
#[derive(Debug, thiserror::Error)]
pub enum S3ConfigError {
    #[error("invalid S3 endpoint or bucket name: {0:?}")]
    InvalidBucket(BucketError),

    #[error("failed to build HTTP client: {0}")]
    Client(#[source] reqwest::Error),
}

pub struct S3ObjectStore {
    bucket: Bucket,
    credentials: Credentials,
    client: reqwest::Client,
}

impl S3ObjectStore {
    pub fn new(config: S3Config) -> Result<Self, S3ConfigError> {
        let path_style = if config.path_style {
            UrlStyle::Path
        } else {
            UrlStyle::VirtualHost
        };
        let bucket = Bucket::new(config.endpoint, path_style, config.bucket, config.region)
            .map_err(S3ConfigError::InvalidBucket)?;
        let credentials = Credentials::new(config.access_key, config.secret_key);
        let client = reqwest::Client::builder()
            .build()
            .map_err(S3ConfigError::Client)?;

        Ok(Self {
            bucket,
            credentials,
            client,
        })
    }
}

#[async_trait]
impl ObjectStore for S3ObjectStore {
    fn presign_put(&self, key: &str, content_type: &str, expires_in: Duration) -> PresignedPut {
        let mut action = self.bucket.put_object(Some(&self.credentials), key);
        action.headers_mut().insert("content-type", content_type);
        let url = action.sign(expires_in);

        let mut required_headers = BTreeMap::new();
        required_headers.insert("content-type".to_string(), content_type.to_string());

        PresignedPut {
            url: url.to_string(),
            required_headers,
            expires_at: chrono::Utc::now()
                + chrono::Duration::from_std(expires_in).unwrap_or(chrono::Duration::zero()),
        }
    }

    async fn head(&self, key: &str) -> Result<Option<ObjectMetadata>, ObjectStoreError> {
        // A short, server-side-only expiry: this URL is signed, used, and discarded
        // within the same request, never handed to a client.
        let action = self.bucket.head_object(Some(&self.credentials), key);
        let url = action.sign(Duration::from_secs(60));

        let response = self
            .client
            .head(url.as_str())
            .send()
            .await
            .map_err(|e| ObjectStoreError::Request(e.to_string()))?;

        if response.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }
        if !response.status().is_success() {
            return Err(ObjectStoreError::Request(format!(
                "object store returned unexpected status {}",
                response.status()
            )));
        }

        let size_bytes = response
            .headers()
            .get(reqwest::header::CONTENT_LENGTH)
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);
        let content_type = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .map(str::to_string);

        Ok(Some(ObjectMetadata {
            size_bytes,
            content_type,
        }))
    }
}
