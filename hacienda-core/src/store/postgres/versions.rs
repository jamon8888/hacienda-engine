//! Postgres [`DocumentVersionStore`] implementation.

use async_trait::async_trait;
use sqlx::PgPool;
use uuid::Uuid;

/// Error type for version store operations.
#[derive(Debug, thiserror::Error)]
pub enum VersionError {
    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),

    #[error("version not found")]
    NotFound,
}

/// Trait for document version storage.
#[async_trait]
pub trait DocumentVersionStore: Send + Sync {
    /// Create a new version for a document.
    ///
    /// Returns the version sequence number (1-based). If the same content_hash
    /// already exists for the latest version of this document, returns the
    /// existing version number (idempotent re-upload).
    async fn create_version(
        &self,
        document_id: Uuid,
        content_hash: &str,
    ) -> Result<i64, VersionError>;

    /// List all versions for a document, newest first.
    async fn list_versions(&self, document_id: Uuid) -> Result<Vec<DocumentVersion>, VersionError>;

    /// Get a specific version by sequence number.
    async fn get_version(
        &self,
        document_id: Uuid,
        version_sequence: i64,
    ) -> Result<Option<DocumentVersion>, VersionError>;
}

/// A document version record.
#[derive(Debug, Clone)]
pub struct DocumentVersion {
    pub id: Uuid,
    pub document_id: Uuid,
    pub version_sequence: i64,
    pub content_hash: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

/// Postgres-backed [`DocumentVersionStore`].
#[derive(Clone)]
pub struct PostgresDocumentVersionStore {
    pool: PgPool,
}

impl PostgresDocumentVersionStore {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl DocumentVersionStore for PostgresDocumentVersionStore {
    async fn create_version(
        &self,
        document_id: Uuid,
        content_hash: &str,
    ) -> Result<i64, VersionError> {
        // Check if the latest version already has this content hash
        let existing = sqlx::query!(
            r#"
            SELECT version_sequence FROM document_versions
            WHERE document_id = $1
            ORDER BY version_sequence DESC
            LIMIT 1
            "#,
            document_id
        )
        .fetch_optional(&self.pool)
        .await?;

        if let Some(row) = existing {
            // Check if content hash matches
            let latest_hash = sqlx::query!(
                "SELECT content_hash FROM document_versions WHERE document_id = $1 AND version_sequence = $2",
                document_id, row.version_sequence
            )
            .fetch_one(&self.pool)
            .await?;

            if latest_hash.content_hash == content_hash {
                return Ok(row.version_sequence);
            }
        }

        // Insert new version with next sequence number
        let version_sequence = sqlx::query!(
            r#"
            INSERT INTO document_versions (document_id, version_sequence, content_hash)
            SELECT $1, COALESCE(MAX(version_sequence), 0) + 1, $2
            FROM document_versions
            WHERE document_id = $1
            RETURNING version_sequence
            "#,
            document_id,
            content_hash
        )
        .fetch_one(&self.pool)
        .await?
        .version_sequence;

        Ok(version_sequence)
    }

    async fn list_versions(&self, document_id: Uuid) -> Result<Vec<DocumentVersion>, VersionError> {
        let rows = sqlx::query!(
            r#"
            SELECT id, document_id, version_sequence, content_hash, created_at
            FROM document_versions
            WHERE document_id = $1
            ORDER BY version_sequence DESC
            "#,
            document_id
        )
        .fetch_all(&self.pool)
        .await?;

        Ok(rows
            .into_iter()
            .map(|row| DocumentVersion {
                id: row.id,
                document_id: row.document_id,
                version_sequence: row.version_sequence,
                content_hash: row.content_hash,
                created_at: row.created_at,
            })
            .collect())
    }

    async fn get_version(
        &self,
        document_id: Uuid,
        version_sequence: i64,
    ) -> Result<Option<DocumentVersion>, VersionError> {
        let row = sqlx::query!(
            r#"
            SELECT id, document_id, version_sequence, content_hash, created_at
            FROM document_versions
            WHERE document_id = $1 AND version_sequence = $2
            "#,
            document_id,
            version_sequence
        )
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.map(|row| DocumentVersion {
            id: row.id,
            document_id: row.document_id,
            version_sequence: row.version_sequence,
            content_hash: row.content_hash,
            created_at: row.created_at,
        }))
    }
}