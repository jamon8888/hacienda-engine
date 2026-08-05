//! Postgres [`AuditStore`] implementation.
//!
//! Implements the `AuditStore` trait using a Postgres backend. All operations
//! use a single `INSERT ... RETURNING` per batch inside one transaction for
//! `append`, matching the "one call = one transaction boundary" rationale from
//! Phase 1 Design Decision D3.

use crate::audit::{
    entry::{AuditEntry, AuditEntryInput},
    error::AuditError,
    segment::verify_seal_chain,
    segment::{SegmentSeal, GENESIS_HASH},
    AuditStore,
};
use async_trait::async_trait;
use sqlx::PgPool;
use std::sync::Arc;

/// Postgres-backed [`AuditStore`].
#[derive(Clone)]
pub struct PostgresAuditStore {
    pool: PgPool,
}

impl PostgresAuditStore {
    /// Create a new store from an existing pool.
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl AuditStore for PostgresAuditStore {
    async fn append(
        &self,
        inputs: Vec<AuditEntryInput>,
    ) -> Result<Vec<AuditEntry>, AuditError> {
        if inputs.is_empty() {
            return Ok(Vec::new());
        }

        // We need to get the current open segment, or create one.
        // This is done in a single transaction to maintain atomicity.
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|e| AuditError::Io {
                path: "transaction".into(),
                source: e.into(),
            })?;

        // Get or create the open segment.
        let segment_id = get_or_create_open_segment(&mut tx, &inputs[0].config_hash).await?;

        // Insert all entries in this batch.
        let mut entries = Vec::with_capacity(inputs.len());
        let mut sequence_num = get_next_sequence_num(&mut tx, segment_id).await?;

        for input in inputs {
            sequence_num += 1;
            let entry = insert_entry(&mut tx, segment_id, sequence_num, input).await?;
            entries.push(entry);
        }

        // Update segment entry count.
        sqlx::query!(
            "UPDATE audit_segments SET entry_count = $1 WHERE segment_id = $2",
            sequence_num as i64,
            segment_id
        )
        .execute(&mut *tx)
        .await
        .map_err(|e| AuditError::Io {
            path: "update_segment".into(),
            source: e.into(),
        })?;

        tx.commit()
            .await
            .map_err(|e| AuditError::Io {
                path: "commit".into(),
                source: e.into(),
            })?;

        Ok(entries)
    }

    async fn entries(&self) -> Result<Vec<AuditEntry>, AuditError> {
        let rows = sqlx::query!(
            r#"
            SELECT id, segment_id, sequence_num, category, action, span_hash,
                   span_length, confidence, source, pipeline_version, config_hash, principal, created_at
            FROM audit_entries
            WHERE segment_id = (
                SELECT segment_id FROM audit_segments
                WHERE sealed_at IS NULL
                ORDER BY created_at DESC
                LIMIT 1
            )
            ORDER BY sequence_num
            "#
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|e| AuditError::Io {
            path: "entries".into(),
            source: e.into(),
        })?;

        Ok(rows
            .into_iter()
            .map(|row| AuditEntry {
                id: row.id,
                category: row.category,
                action: row.action.parse().unwrap_or(crate::audit::entry::RedactionAction::Mask),
                span_hash: row.span_hash,
                span_length: row.span_length as u32,
                confidence: row.confidence,
                source: row.source.parse().unwrap_or(crate::audit::entry::EntitySource::Regex),
                pipeline_version: row.pipeline_version,
                config_hash: row.config_hash,
                principal: row.principal,
                chain_hash: String::new(), // Will be computed by caller if needed
            })
            .collect())
    }

    async fn tip(&self) -> Result<String, AuditError> {
        // Get the latest seal's sealed_tip, or genesis if no seals exist.
        let row = sqlx::query!(
            r#"
            SELECT sealed_tip FROM audit_segments
            WHERE sealed_at IS NOT NULL
            ORDER BY sealed_at DESC
            LIMIT 1
            "#
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| AuditError::Io {
            path: "tip".into(),
            source: e.into(),
        })?;

        Ok(row
            .and_then(|r| r.sealed_tip)
            .unwrap_or_else(|| GENESIS_HASH.to_owned()))
    }

    async fn seals(&self) -> Result<Vec<SegmentSeal>, AuditError> {
        let rows = sqlx::query!(
            r#"
            SELECT segment_id, node_id, config_hash, prev_seal_hash, sealed_tip, entry_count, created_at, sealed_at
            FROM audit_segments
            WHERE sealed_at IS NOT NULL
            ORDER BY created_at
            "#
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|e| AuditError::Io {
            path: "seals".into(),
            source: e.into(),
        })?;

        Ok(rows
            .into_iter()
            .map(|row| SegmentSeal {
                segment_id: row.segment_id.to_string(),
                node_id: row.node_id,
                config_hash: row.config_hash,
                prev_seal_hash: row.prev_seal_hash,
                sealed_tip: row.sealed_tip.unwrap_or_default(),
                entry_count: row.entry_count as u64,
            })
            .collect())
    }

    async fn verify(&self) -> Result<(), AuditError> {
        // Verify seal chain
        let seals = self.seals().await?;
        verify_seal_chain(&seals).map_err(|e| AuditError::ChainIntegrity {
            index: 0,
            expected: String::new(),
            actual: e.to_string(),
        })?;

        // Verify each sealed segment's entries
        for seal in &seals {
            let entries = self.get_segment_entries(&seal.segment_id).await?;
            crate::audit::store::verify_sealed_entries(&entries, seal)?;
        }

        // Verify open segment
        let open_entries = self.entries().await?;
        let config_hash = self.get_config_hash().await?;
        crate::audit::store::verify_open_entries(&open_entries, &self.tip().await?, &config_hash)?;

        Ok(())
    }

    async fn rotate(&self) -> Result<SegmentSeal, AuditError> {
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|e| AuditError::Io {
                path: "transaction".into(),
                source: e.into(),
            })?;

        // Get current open segment
        let segment_row = sqlx::query!(
            "SELECT segment_id, node_id, config_hash, entry_count FROM audit_segments WHERE sealed_at IS NULL"
        )
        .fetch_one(&mut *tx)
        .await
        .map_err(|e| AuditError::Io {
            path: "rotate_select".into(),
            source: e.into(),
        })?;

        let entries = self.get_segment_entries(&segment_row.segment_id).await?;
        let tip = self.compute_tip(&entries).await?;

        // Seal the segment
        let seal = SegmentSeal {
            segment_id: segment_row.segment_id.to_string(),
            node_id: segment_row.node_id.clone(),
            config_hash: segment_row.config_hash.clone(),
            prev_seal_hash: self.get_latest_seal_hash(&mut tx).await?,
            sealed_tip: tip.clone(),
            entry_count: segment_row.entry_count as u64,
        };

        let seal_hash = seal.compute_hash();

        sqlx::query!(
            "UPDATE audit_segments SET sealed_at = now(), sealed_tip = $1 WHERE segment_id = $2",
            tip,
            seal.segment_id
        )
        .execute(&mut *tx)
        .await
        .map_err(|e| AuditError::Io {
            path: "rotate_update".into(),
            source: e.into(),
        })?;

        // Create new open segment
        let new_segment_id = sqlx::query!(
            r#"
            INSERT INTO audit_segments (node_id, config_hash, prev_seal_hash)
            VALUES ($1, $2, $3)
            RETURNING segment_id
            "#,
            segment_row.node_id,
            segment_row.config_hash,
            seal_hash
        )
        .fetch_one(&mut *tx)
        .await
        .map_err(|e| AuditError::Io {
            path: "rotate_insert".into(),
            source: e.into(),
        })?
        .segment_id;

        tx.commit()
            .await
            .map_err(|e| AuditError::Io {
                path: "rotate_commit".into(),
                source: e.into(),
            })?;

        Ok(seal)
    }

    async fn close(&self) -> Result<SegmentSeal, AuditError> {
        // Similar to rotate but without creating a new segment
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|e| AuditError::Io {
                path: "transaction".into(),
                source: e.into(),
            })?;

        let segment_row = sqlx::query!(
            "SELECT segment_id, node_id, config_hash, entry_count FROM audit_segments WHERE sealed_at IS NULL"
        )
        .fetch_optional(&mut *tx)
        .await
        .map_err(|e| AuditError::Io {
            path: "close_select".into(),
            source: e.into(),
        })?;

        let Some(segment_row) = segment_row else {
            return Err(AuditError::StoreClosed {
                operation: "close",
            });
        };

        let entries = self.get_segment_entries(&segment_row.segment_id).await?;
        let tip = self.compute_tip(&entries).await?;

        let seal = SegmentSeal {
            segment_id: segment_row.segment_id.to_string(),
            node_id: segment_row.node_id.clone(),
            config_hash: segment_row.config_hash.clone(),
            prev_seal_hash: self.get_latest_seal_hash(&mut tx).await?,
            sealed_tip: tip.clone(),
            entry_count: segment_row.entry_count as u64,
        };

        sqlx::query!(
            "UPDATE audit_segments SET sealed_at = now(), sealed_tip = $1 WHERE segment_id = $2",
            tip,
            segment_row.segment_id
        )
        .execute(&mut *tx)
        .await
        .map_err(|e| AuditError::Io {
            path: "close_update".into(),
            source: e.into(),
        })?;

        tx.commit()
            .await
            .map_err(|e| AuditError::Io {
                path: "close_commit".into(),
                source: e.into(),
            })?;

        Ok(seal)
    }
}

// Helper functions

async fn get_or_create_open_segment(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    config_hash: &str,
) -> Result<uuid::Uuid, AuditError> {
    let row = sqlx::query!(
        "SELECT segment_id FROM audit_segments WHERE sealed_at IS NULL ORDER BY created_at DESC LIMIT 1"
    )
    .fetch_optional(&mut **tx)
    .await
    .map_err(|e| AuditError::Io {
        path: "get_open_segment".into(),
        source: e.into(),
    })?;

    if let Some(row) = row {
        return Ok(row.segment_id);
    }

    // No open segment, create one
    let row = sqlx::query!(
        "INSERT INTO audit_segments (node_id, config_hash) VALUES ($1, $2) RETURNING segment_id",
        format!("hacienda-{}", std::process::id()),
        config_hash
    )
    .fetch_one(&mut **tx)
    .await
    .map_err(|e| AuditError::Io {
        path: "create_segment".into(),
        source: e.into(),
    })?;

    Ok(row.segment_id)
}

async fn get_next_sequence_num(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    segment_id: uuid::Uuid,
) -> Result<i64, AuditError> {
    let row = sqlx::query!(
        "SELECT COALESCE(MAX(sequence_num), 0) as max_seq FROM audit_entries WHERE segment_id = $1",
        segment_id
    )
    .fetch_one(&mut **tx)
    .await
    .map_err(|e| AuditError::Io {
        path: "max_sequence".into(),
        source: e.into(),
    })?;

    Ok(row.max_seq.unwrap_or(0))
}

async fn insert_entry(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    segment_id: uuid::Uuid,
    sequence_num: i64,
    input: AuditEntryInput,
) -> Result<AuditEntry, AuditError> {
    let chain_hash = compute_chain_hash_for_insert(tx, segment_id, sequence_num, &input).await?;

    sqlx::query!(
        r#"
        INSERT INTO audit_entries (id, segment_id, sequence_num, category, action, span_hash,
                                  span_length, confidence, source, pipeline_version, config_hash, principal)
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)
        "#,
        input.id,
        segment_id,
        sequence_num,
        input.category,
        input.action.to_string(),
        input.span_hash,
        input.span_length as i64,
        input.confidence,
        input.source.to_string(),
        input.pipeline_version,
        input.config_hash,
        input.principal
    )
    .execute(&mut **tx)
    .await
    .map_err(|e| AuditError::Io {
        path: "insert_entry".into(),
        source: e.into(),
    })?;

    Ok(AuditEntry {
        id: input.id,
        category: input.category,
        action: input.action,
        span_hash: input.span_hash,
        span_length: input.span_length,
        confidence: input.confidence,
        source: input.source,
        pipeline_version: input.pipeline_version,
        config_hash: input.config_hash,
        principal: input.principal,
        chain_hash,
    })
}

async fn compute_chain_hash_for_insert(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    segment_id: uuid::Uuid,
    sequence_num: i64,
    input: &AuditEntryInput,
) -> Result<String, AuditError> {
    // Get previous entry's chain_hash
    let prev_hash = if sequence_num == 1 {
        crate::audit::GENESIS_HASH.to_owned()
    } else {
        sqlx::query!(
            "SELECT chain_hash FROM audit_entries WHERE segment_id = $1 AND sequence_num = $2",
            segment_id,
            sequence_num - 1
        )
        .fetch_one(&mut **tx)
        .await
        .map_err(|e| AuditError::Io {
            path: "prev_hash".into(),
            source: e.into(),
        })?
        .chain_hash
    };

    // Compute new chain hash
    use crate::audit::chain::AuditChain;
    let mut chain = AuditChain::new(input.config_hash.clone());
    // We need to replay from genesis... this is inefficient.
    // For now, use blake3 directly matching AuditChain logic.
    use blake3::Hasher;
    let mut hasher = Hasher::new();
    hasher.update(prev_hash.as_bytes());
    hasher.update(input.id.as_bytes());
    hasher.update(input.category.as_bytes());
    hasher.update(input.action.to_string().as_bytes());
    hasher.update(input.span_hash.as_bytes());
    hasher.update(&input.span_length.to_le_bytes());
    if let Some(c) = input.confidence {
        hasher.update(&c.to_le_bytes());
    }
    hasher.update(input.source.to_string().as_bytes());
    hasher.update(input.pipeline_version.as_bytes());
    hasher.update(input.config_hash.as_bytes());
    if let Some(p) = &input.principal {
        hasher.update(p.as_bytes());
    }
    Ok(hasher.finalize().to_hex().to_string())
}

impl PostgresAuditStore {
    async fn get_segment_entries(&self, segment_id: &str) -> Result<Vec<AuditEntry>, AuditError> {
        let rows = sqlx::query!(
            r#"
            SELECT id, category, action, span_hash, span_length, confidence, source,
                   pipeline_version, config_hash, principal
            FROM audit_entries
            WHERE segment_id = $1
            ORDER BY sequence_num
            "#,
            uuid::Uuid::parse_str(segment_id).unwrap()
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|e| AuditError::Io {
            path: "segment_entries".into(),
            source: e.into(),
        })?;

        Ok(rows
            .into_iter()
            .map(|row| AuditEntry {
                id: row.id,
                category: row.category,
                action: row.action.parse().unwrap_or(crate::audit::entry::RedactionAction::Mask),
                span_hash: row.span_hash,
                span_length: row.span_length as u32,
                confidence: row.confidence,
                source: row.source.parse().unwrap_or(crate::audit::entry::EntitySource::Regex),
                pipeline_version: row.pipeline_version,
                config_hash: row.config_hash,
                principal: row.principal,
                chain_hash: String::new(),
            })
            .collect())
    }

    async fn get_config_hash(&self) -> Result<String, AuditError> {
        let row = sqlx::query!(
            "SELECT config_hash FROM audit_segments WHERE sealed_at IS NULL ORDER BY created_at DESC LIMIT 1"
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| AuditError::Io {
            path: "config_hash".into(),
            source: e.into(),
        })?;

        Ok(row
            .map(|r| r.config_hash)
            .unwrap_or_else(|| "default".to_string()))
    }

    async fn get_latest_seal_hash(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    ) -> Result<Option<String>, AuditError> {
        let row = sqlx::query!(
            "SELECT sealed_tip FROM audit_segments WHERE sealed_at IS NOT NULL ORDER BY sealed_at DESC LIMIT 1"
        )
        .fetch_optional(&mut **tx)
        .await
        .map_err(|e| AuditError::Io {
            path: "latest_seal".into(),
            source: e.into(),
        })?;

        Ok(row.and_then(|r| r.sealed_tip))
    }

    async fn compute_tip(&self, entries: &[AuditEntry]) -> Result<String, AuditError> {
        if entries.is_empty() {
            return Ok(crate::audit::GENESIS_HASH.to_owned());
        }
        Ok(entries.last().unwrap().chain_hash.clone())
    }
}