//! Postgres [`DocumentVersionStore`] implementation.

use crate::tenancy::TenantId;
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
    ///
    /// `tenant` scopes both the idempotency check and the insert (S1b): two tenants
    /// creating a version for the same `document_id` — a UUID the caller supplies, not
    /// one the store assigns, so a collision is entirely plausible — get independent
    /// version sequences starting at 1, never sharing or racing over one sequence.
    async fn create_version(
        &self,
        tenant: &TenantId,
        document_id: Uuid,
        content_hash: &str,
        content: &str,
        entities_json: Value,
    ) -> Result<i64, VersionError>;

    /// List all versions for `document_id` belonging to `tenant`, newest first. A
    /// `document_id` that exists only under a different tenant returns an empty list —
    /// indistinguishable from a `document_id` that was never used at all (D-S1b-1).
    async fn list_versions(
        &self,
        tenant: &TenantId,
        document_id: Uuid,
    ) -> Result<Vec<DocumentVersion>, VersionError>;

    /// Get a specific version by sequence number, returning `None` if it does not exist
    /// **or belongs to a different tenant** — the two are indistinguishable to the caller
    /// (D-S1b-1: a cross-tenant id is reported not-found, never forbidden).
    async fn get_version(
        &self,
        tenant: &TenantId,
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
        tenant: &TenantId,
        document_id: Uuid,
        content_hash: &str,
        content: &str,
        entities_json: Value,
    ) -> Result<i64, VersionError> {
        let tenant_id = tenant.as_str();
        let mut tx = self.pool.begin().await?;

        // Serialize concurrent version creation for the same document. Without this,
        // two transactions racing "SELECT MAX(version_sequence)+1" under READ COMMITTED
        // isolation can compute the same next sequence number and one loses to the
        // UNIQUE (tenant_id, document_id, version_sequence) constraint instead of
        // getting a clean sequential number. The lock is scoped per-document (hashtext
        // of the UUID text) and held only for this transaction — uncontended callers
        // pay nothing. Not scoped by tenant: harmless, and simpler than it needs to be
        // now that the constraint is tenant-scoped (S1b) — two tenants racing on the
        // same document_id still serialize against each other here, even though they no
        // longer could collide at the constraint itself. A tenant-scoped lock key would
        // remove that unnecessary cross-tenant contention; not done here since nothing
        // observed it as a real bottleneck.

        sqlx::query!(
            "SELECT pg_advisory_xact_lock(hashtext($1)::bigint)",
            document_id.to_string()
        )
        .execute(&mut *tx)
        .await?;

        // Check if the latest version already has this content hash (idempotent re-upload).
        // Scoped to this tenant's own versions: two tenants racing to create the first
        // version of the same document_id must each start their own sequence at 1, not
        // observe (and idempotently short-circuit against) the other tenant's version.
        let existing = sqlx::query!(
            r#"
            SELECT version_sequence, content_hash FROM document_versions
            WHERE document_id = $1 AND tenant_id = $2
            ORDER BY version_sequence DESC
            LIMIT 1
            "#,
            document_id,
            tenant_id
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
                (document_id, tenant_id, version_sequence, content_hash, content, entities_json)
            VALUES ($1, $2, $3, $4, $5, $6)
            RETURNING version_sequence
            "#,
            document_id,
            tenant_id,
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

    async fn list_versions(
        &self,
        tenant: &TenantId,
        document_id: Uuid,
    ) -> Result<Vec<DocumentVersion>, VersionError> {
        let rows = sqlx::query!(
            r#"
            SELECT id, document_id, version_sequence, content_hash, content, entities_json, created_at
            FROM document_versions
            WHERE document_id = $1 AND tenant_id = $2
            ORDER BY version_sequence DESC
            "#,
            document_id,
            tenant.as_str()
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
        tenant: &TenantId,
        document_id: Uuid,
        version_sequence: i64,
    ) -> Result<Option<DocumentVersion>, VersionError> {
        let row = sqlx::query!(
            r#"
            SELECT id, document_id, version_sequence, content_hash, content, entities_json, created_at
            FROM document_versions
            WHERE document_id = $1 AND version_sequence = $2 AND tenant_id = $3
            "#,
            document_id,
            version_sequence,
            tenant.as_str()
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

    /// The tenant every test in this module uses. Mirrors `postgres::jobs::tests::t`:
    /// these tests already share one un-torn-down Postgres database, so a fixed tenant
    /// id keeps that existing shared behaviour rather than accidentally isolating them.
    fn t() -> TenantId {
        TenantId::new("pg-versions-test-tenant")
    }

    #[test]
    #[ignore]
    fn should_create_and_list_versions_round_trip() {
        test_support::block_on_shared(async {
            let store = test_store().await;
            let document_id = Uuid::new_v4();

            let v1 = store
                .create_version(
                    &t(),
                    document_id,
                    "hash-1",
                    "redacted content v1",
                    json!([]),
                )
                .await
                .expect("create_version failed");
            let v2 = store
                .create_version(
                    &t(),
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
                    &t(),
                    document_id,
                    "hash-2",
                    "redacted content v2",
                    json!([{"type": "EMAIL", "start": 0, "end": 5}]),
                )
                .await
                .expect("create_version failed");
            assert_eq!(v2_again, 2);

            let versions = store
                .list_versions(&t(), document_id)
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
                            &t(),
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

    /// D-S1b-1: a `document_id`/`version_sequence` that belongs to a different tenant
    /// must be reported exactly like one that does not exist at all — `None`/empty,
    /// never a distinguishable error.
    #[test]
    #[ignore]
    fn version_get_and_list_for_another_tenants_document_id_is_not_found() {
        test_support::block_on_shared(async {
            let store = test_store().await;
            let other_tenant = TenantId::new("pg-versions-test-tenant-other");
            let document_id = Uuid::new_v4();

            store
                .create_version(&t(), document_id, "hash-1", "content", json!([]))
                .await
                .expect("create_version failed");

            assert!(store
                .get_version(&t(), document_id, 1)
                .await
                .expect("get_version failed")
                .is_some());
            assert!(store
                .get_version(&other_tenant, document_id, 1)
                .await
                .expect("get_version failed")
                .is_none());

            let other_tenant_versions = store
                .list_versions(&other_tenant, document_id)
                .await
                .expect("list_versions failed");
            assert!(other_tenant_versions.is_empty());
        });
    }

    /// `document_id` is caller-supplied, so two tenants choosing the same id is
    /// plausible, not hypothetical. Each tenant's `version_sequence` must start at 1
    /// independently — the old bare `UNIQUE(document_id, version_sequence)` would make
    /// the second tenant's first `create_version` collide with the first tenant's.
    #[test]
    #[ignore]
    fn two_tenants_can_independently_version_the_same_document_id() {
        test_support::block_on_shared(async {
            let store = test_store().await;
            let other_tenant = TenantId::new("pg-versions-test-tenant-other2");
            let document_id = Uuid::new_v4();

            let v1 = store
                .create_version(&t(), document_id, "hash-a-1", "tenant a content", json!([]))
                .await
                .expect("tenant a create_version failed");
            let v1_other = store
                .create_version(
                    &other_tenant,
                    document_id,
                    "hash-b-1",
                    "tenant b content",
                    json!([]),
                )
                .await
                .expect("tenant b create_version failed");

            assert_eq!(v1, 1, "tenant a's first version is sequence 1");
            assert_eq!(v1_other, 1, "tenant b's first version is also sequence 1");

            let a_version = store
                .get_version(&t(), document_id, 1)
                .await
                .expect("get_version failed")
                .expect("tenant a version must exist");
            assert_eq!(a_version.content, "tenant a content");

            let b_version = store
                .get_version(&other_tenant, document_id, 1)
                .await
                .expect("get_version failed")
                .expect("tenant b version must exist");
            assert_eq!(b_version.content, "tenant b content");
        });
    }
}
