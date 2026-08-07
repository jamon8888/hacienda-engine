//! Postgres [`DocumentVersionStore`] implementation.

use async_trait::async_trait;
use serde_json::Value;
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
    /// `content` is the redacted output (never raw input, per Decision 1 of the
    /// integration spec) and `entities_json` is the detected-entity metadata that
    /// accompanies it — both are stored so `GET /v1/documents/{id}` and
    /// `GET /v1/documents/{id}/diff` can retrieve and compare them, not just prove
    /// their identity via `content_hash`.
    ///
    /// Returns the version sequence number (1-based). If the same content_hash
    /// already exists for the latest version of this document, returns the
    /// existing version number (idempotent re-upload) without inserting a new row.
    async fn create_version(
        &self,
        document_id: Uuid,
        content_hash: &str,
        content: &str,
        entities_json: Value,
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
    pub content: String,
    pub entities_json: Value,
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
        content: &str,
        entities_json: Value,
    ) -> Result<i64, VersionError> {
        let mut tx = self.pool.begin().await?;

        // Serialize concurrent version creation for the same document. Without this,
        // two transactions racing "SELECT MAX(version_sequence)+1" under READ COMMITTED
        // isolation can compute the same next sequence number and one loses to the
        // UNIQUE (document_id, version_sequence) constraint instead of getting a clean
        // sequential number. The lock is scoped per-document (hashtext of the UUID
        // text) and held only for this transaction — uncontended callers pay nothing.
        sqlx::query!(
            "SELECT pg_advisory_xact_lock(hashtext($1)::bigint)",
            document_id.to_string()
        )
        .execute(&mut *tx)
        .await?;

        // Check if the latest version already has this content hash (idempotent re-upload).
        let existing = sqlx::query!(
            r#"
            SELECT version_sequence, content_hash FROM document_versions
            WHERE document_id = $1
            ORDER BY version_sequence DESC
            LIMIT 1
            "#,
            document_id
        )
        .fetch_optional(&mut *tx)
        .await?;

        if let Some(row) = &existing {
            if row.content_hash == content_hash {
                tx.commit().await?;
                return Ok(row.version_sequence);
            }
        }

        let next_sequence = existing.map(|row| row.version_sequence).unwrap_or(0) + 1;

        let version_sequence = sqlx::query!(
            r#"
            INSERT INTO document_versions
                (document_id, version_sequence, content_hash, content, entities_json)
            VALUES ($1, $2, $3, $4, $5)
            RETURNING version_sequence
            "#,
            document_id,
            next_sequence,
            content_hash,
            content,
            entities_json
        )
        .fetch_one(&mut *tx)
        .await?
        .version_sequence;

        tx.commit().await?;

        Ok(version_sequence)
    }

    async fn list_versions(&self, document_id: Uuid) -> Result<Vec<DocumentVersion>, VersionError> {
        let rows = sqlx::query!(
            r#"
            SELECT id, document_id, version_sequence, content_hash, content, entities_json, created_at
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
                content: row.content,
                entities_json: row.entities_json,
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
            SELECT id, document_id, version_sequence, content_hash, content, entities_json, created_at
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
            content: row.content,
            entities_json: row.entities_json,
            created_at: row.created_at,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::postgres::test_support;
    use serde_json::json;
    use std::collections::HashSet;
    use std::sync::Arc;

    // Ignored by default — shares one Postgres instance with the other postgres-feature
    // test modules (see `test_support::shared`), so needs `--test-threads=1`. Run with:
    //   cargo test -p hacienda-core --features postgres \
    //     --lib store::postgres::versions -- --ignored --test-threads=1

    async fn test_store() -> PostgresDocumentVersionStore {
        PostgresDocumentVersionStore::new(test_support::shared().await.pool())
    }

    #[test]
    #[ignore]
    fn should_create_and_list_versions_round_trip() {
        test_support::block_on_shared(async {
            let store = test_store().await;
            let document_id = Uuid::new_v4();

            let v1 = store
                .create_version(document_id, "hash-1", "redacted content v1", json!([]))
                .await
                .expect("create_version failed");
            let v2 = store
                .create_version(
                    document_id,
                    "hash-2",
                    "redacted content v2",
                    json!([{"type": "EMAIL", "start": 0, "end": 5}]),
                )
                .await
                .expect("create_version failed");
            assert_eq!(v1, 1);
            assert_eq!(v2, 2);

            // Re-uploading the same content as the latest version is idempotent.
            let v2_again = store
                .create_version(
                    document_id,
                    "hash-2",
                    "redacted content v2",
                    json!([{"type": "EMAIL", "start": 0, "end": 5}]),
                )
                .await
                .expect("create_version failed");
            assert_eq!(v2_again, 2);

            let versions = store
                .list_versions(document_id)
                .await
                .expect("list_versions failed");
            assert_eq!(versions.len(), 2, "idempotent re-upload must not add a row");
            assert_eq!(versions[0].version_sequence, 2, "newest first");
            assert_eq!(versions[0].content, "redacted content v2");
            assert_eq!(
                versions[0].entities_json,
                json!([{"type": "EMAIL", "start": 0, "end": 5}])
            );
            assert_eq!(versions[1].version_sequence, 1);
            assert_eq!(versions[1].content, "redacted content v1");
            assert_eq!(versions[1].entities_json, json!([]));
        });
    }

    #[test]
    #[ignore]
    fn should_assign_sequential_version_numbers_with_no_gaps_or_duplicates_under_concurrency() {
        test_support::block_on_shared(async {
            let store = Arc::new(test_store().await);
            let document_id = Uuid::new_v4();
            const N: i64 = 10;

            let mut handles = Vec::new();
            for i in 0..N {
                let store = Arc::clone(&store);
                handles.push(tokio::spawn(async move {
                    store
                        .create_version(
                            document_id,
                            &format!("hash-{i}"),
                            &format!("content-{i}"),
                            json!([]),
                        )
                        .await
                }));
            }

            let mut sequences = Vec::new();
            for handle in handles {
                sequences.push(
                    handle
                        .await
                        .expect("task panicked")
                        .expect("create_version failed"),
                );
            }

            let unique: HashSet<i64> = sequences.iter().copied().collect();
            assert_eq!(
                unique.len(),
                sequences.len(),
                "no two concurrent creates may receive the same version_sequence"
            );

            let mut sorted = sequences.clone();
            sorted.sort_unstable();
            let expected: Vec<i64> = (1..=N).collect();
            assert_eq!(
                sorted, expected,
                "version numbers must be sequential with no gaps"
            );
        });
    }
}
