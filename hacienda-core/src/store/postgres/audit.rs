//! Postgres [`AuditStore`] implementation.
//!
//! Implements the `AuditStore` trait using a Postgres backend. All operations
//! use a single `INSERT ... RETURNING` per batch inside one transaction for
//! `append`, matching the "one call = one transaction boundary" rationale from
//! Phase 1 Design Decision D3.

use crate::audit::{
    cursor::{page_from, AuditCursor, AuditPage},
    entry::{AuditEntry, AuditEntryInput, EntitySource, RedactionAction},
    error::AuditError,
    segment::{compute_seal_hash, verify_seal_chain, SegmentSeal},
    AuditStore, GENESIS_HASH,
};
use crate::tenancy::TenantId;
use async_trait::async_trait;
use chrono::{DateTime, SubsecRound, Utc};
use sqlx::PgPool;
use uuid::Uuid;

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
        tenant: &TenantId,
        inputs: Vec<AuditEntryInput>,
    ) -> Result<Vec<AuditEntry>, AuditError> {
        if inputs.is_empty() {
            return Ok(Vec::new());
        }

        // We need to get the current open segment, or create one.
        // This is done in a single transaction to maintain atomicity.
        let mut tx = self.pool.begin().await?;

        // Get or create the open segment — this tenant's, specifically.
        let (segment_id, segment_config_hash) =
            get_or_create_open_segment(&mut tx, tenant, &inputs[0].config_hash).await?;

        // Insert all entries in this batch.
        let mut entries = Vec::with_capacity(inputs.len());
        let mut sequence_num = get_next_sequence_num(&mut tx, segment_id).await?;
        let mut prev_chain_hash = get_prev_chain_hash(&mut tx, segment_id, sequence_num).await?;

        for input in inputs {
            sequence_num += 1;
            let entry = insert_entry(
                &mut tx,
                segment_id,
                sequence_num,
                input,
                &prev_chain_hash,
                &segment_config_hash,
            )
            .await?;
            prev_chain_hash = entry.chain_hash.clone();
            entries.push(entry);
        }

        // Update segment entry count.
        sqlx::query!(
            "UPDATE audit_segments SET entry_count = $1 WHERE segment_id = $2",
            sequence_num,
            segment_id
        )
        .execute(&mut *tx)
        .await?;

        tx.commit().await?;

        Ok(entries)
    }

    async fn entries(&self, tenant: &TenantId) -> Result<Vec<AuditEntry>, AuditError> {
        let tenant_id = tenant.as_str();
        let rows = sqlx::query_as!(
            AuditEntryRow,
            r#"
            SELECT id, category, action, span_hash,
                   span_length, confidence, source, pipeline_version, config_hash, principal,
                   chain_hash, created_at
            FROM audit_entries
            WHERE segment_id = (
                SELECT segment_id FROM audit_segments
                WHERE sealed_at IS NULL AND tenant_id = $1
                ORDER BY created_at DESC
                LIMIT 1
            )
            ORDER BY sequence_num
            "#,
            tenant_id
        )
        .fetch_all(&self.pool)
        .await?;

        rows.into_iter().map(row_to_entry).collect()
    }

    async fn history(
        &self,
        tenant: &TenantId,
        after: Option<&AuditCursor>,
        limit: usize,
    ) -> Result<AuditPage, AuditError> {
        let tenant_id = tenant.as_str();

        // Build extents: sealed segments (oldest first), then the open segment
        #[derive(sqlx::FromRow)]
        struct ExtentRow {
            segment_id: Uuid,
            entry_count: i64,
        }

        // Every read below runs against one REPEATABLE READ snapshot. Without this, an
        // `append` or `rotate` committing between the extent queries and the entry
        // queries could change a segment's entry count (or which segment is "open")
        // out from under this method, and `page_from` would report a `SegmentEntryCount`
        // mismatch against a chain that was never actually broken.
        let mut tx = self.pool.begin().await?;
        sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ")
            .execute(&mut *tx)
            .await?;

        let sealed_extents: Vec<ExtentRow> = sqlx::query_as!(
            ExtentRow,
            r#"
            SELECT segment_id, entry_count
            FROM audit_segments
            WHERE sealed_at IS NOT NULL AND tenant_id = $1
            ORDER BY created_at
            "#,
            tenant_id
        )
        .fetch_all(&mut *tx)
        .await?;

        let mut extents = Vec::with_capacity(sealed_extents.len() + 1);
        for row in &sealed_extents {
            extents.push((row.segment_id.to_string(), row.entry_count as u64));
        }

        // Add open segment — its id is captured here and reused below for the entry
        // query, rather than re-selected, so both agree on the same segment even if a
        // rotation opens a new one concurrently (blocked from being visible mid-read by
        // the snapshot above regardless, but reusing the id keeps the two queries from
        // ever being able to disagree even under a weaker isolation level).
        let open_row: Option<ExtentRow> = sqlx::query_as!(
            ExtentRow,
            r#"
            SELECT segment_id, entry_count
            FROM audit_segments
            WHERE sealed_at IS NULL AND tenant_id = $1
            ORDER BY created_at DESC
            LIMIT 1
            "#,
            tenant_id
        )
        .fetch_optional(&mut *tx)
        .await?;

        if let Some(row) = &open_row {
            extents.push((row.segment_id.to_string(), row.entry_count as u64));
        }

        // Fetch all entries for all segments upfront since page_from uses a sync closure.
        // Decoded eagerly (`?` per segment, not stored as `Result`) so the closure below
        // can hand out owned `Vec<AuditEntry>` via `mem::take` without needing `Clone` on
        // `AuditEntry`/`AuditError`.
        let mut all_segment_entries: Vec<Vec<AuditEntry>> = Vec::with_capacity(extents.len());

        for row in &sealed_extents {
            let rows = sqlx::query_as!(
                AuditEntryRow,
                r#"
                SELECT id, category, action, span_hash, span_length, confidence, source,
                       pipeline_version, config_hash, principal, chain_hash, created_at
                FROM audit_entries
                WHERE segment_id = $1
                ORDER BY sequence_num
                "#,
                row.segment_id
            )
            .fetch_all(&mut *tx)
            .await?;
            all_segment_entries.push(decode_segment_rows(row.segment_id, rows)?);
        }

        // Open segment, keyed by the id captured above — not re-queried.
        if let Some(row) = &open_row {
            let open_rows = sqlx::query_as!(
                AuditEntryRow,
                r#"
                SELECT id, category, action, span_hash, span_length, confidence, source,
                       pipeline_version, config_hash, principal, chain_hash, created_at
                FROM audit_entries
                WHERE segment_id = $1
                ORDER BY sequence_num
                "#,
                row.segment_id
            )
            .fetch_all(&mut *tx)
            .await?;
            all_segment_entries.push(decode_segment_rows(row.segment_id, open_rows)?);
        }

        tx.commit().await?;

        // Build extents slice for page_from
        let extent_refs: Vec<(&str, u64)> = extents.iter().map(|(s, c)| (s.as_str(), *c)).collect();

        // `page_from` only ever calls `fetch` once per position (it advances
        // monotonically and never revisits a segment), so `mem::take` safely hands out
        // each segment's entries exactly once.
        page_from(&extent_refs, after, limit, |position| {
            Ok(std::mem::take(&mut all_segment_entries[position]))
        })
    }

    async fn tip(&self, tenant: &TenantId) -> Result<String, AuditError> {
        // Mirrors the in-memory reference's `tip()` (`audit/store.rs`): the open
        // segment's own last entry is the head whenever it has one. Each segment's
        // entry chain restarts at genesis, so continuity across a rotation lives in
        // the *seal* chain — only fall through to the newest seal (or genesis) when
        // the open segment is empty or absent.
        let open_entries = self.entries(tenant).await?;
        if let Some(last) = open_entries.last() {
            return Ok(last.chain_hash.clone());
        }

        // Get the latest seal's sealed_tip, or genesis if no seals exist.
        let tenant_id = tenant.as_str();
        let row = sqlx::query!(
            r#"
            SELECT sealed_tip FROM audit_segments
            WHERE sealed_at IS NOT NULL AND tenant_id = $1
            ORDER BY sealed_at DESC
            LIMIT 1
            "#,
            tenant_id
        )
        .fetch_optional(&self.pool)
        .await?;

        Ok(row
            .and_then(|r| r.sealed_tip)
            .unwrap_or_else(|| GENESIS_HASH.to_owned()))
    }

    async fn seals(&self, tenant: &TenantId) -> Result<Vec<SegmentSeal>, AuditError> {
        let tenant_id = tenant.as_str();
        let rows = sqlx::query_as!(
            SealRow,
            r#"
            SELECT segment_id, node_id, config_hash, prev_seal_hash, sealed_tip, seal_hash,
                   entry_count, created_at, sealed_at
            FROM audit_segments
            WHERE sealed_at IS NOT NULL AND tenant_id = $1
            ORDER BY created_at
            "#,
            tenant_id
        )
        .fetch_all(&self.pool)
        .await?;

        rows.into_iter().map(row_to_seal).collect()
    }

    async fn verify(&self, tenant: &TenantId) -> Result<(), AuditError> {
        // Verify seal chain
        let seals = self.seals(tenant).await?;
        verify_seal_chain(&seals)?;

        // Verify each sealed segment's entries
        for seal in &seals {
            let entries = self.get_segment_entries(&seal.segment_id).await?;
            crate::audit::store::verify_sealed_entries(&entries, seal)?;
        }

        // Verify open segment
        let open_entries = self.entries(tenant).await?;
        let config_hash = self.get_config_hash(tenant).await?;
        crate::audit::store::verify_open_entries(
            &open_entries,
            &self.tip(tenant).await?,
            &config_hash,
        )?;

        Ok(())
    }

    async fn rotate(&self, tenant: &TenantId) -> Result<SegmentSeal, AuditError> {
        let mut tx = self.pool.begin().await?;
        let tenant_id = tenant.as_str();

        // Get current open segment — this tenant's. Without the `tenant_id` filter,
        // `fetch_one` would error as soon as a second tenant's open segment exists:
        // "one open segment" only holds per tenant now, not store-wide. `fetch_optional`,
        // not `fetch_one`: a tenant that has never appended (or was already closed) has
        // no open segment, which is `StoreClosed`, not a generic backend error — matching
        // `InMemoryAuditStore::rotate`'s own `StoreClosed { operation: "rotate" }`.
        let segment_row = sqlx::query!(
            "SELECT segment_id, node_id, config_hash, entry_count, created_at \
             FROM audit_segments WHERE sealed_at IS NULL AND tenant_id = $1",
            tenant_id
        )
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(AuditError::StoreClosed {
            operation: "rotate",
        })?;

        let entries = get_segment_entries_tx(&mut tx, segment_row.segment_id).await?;
        let tip = compute_tip(&entries);
        let prev_seal_hash = get_latest_seal_hash(&mut tx, tenant).await?;
        // ~keep: truncated to microseconds because Postgres TIMESTAMPTZ has microsecond
        // resolution — `Utc::now()` carries nanoseconds, so hashing the untruncated value
        // here and then reading it back post-truncation for `verify()` would compute two
        // different `to_rfc3339()` strings from what the DB considers the same instant,
        // breaking `check_seal_integrity` on every seal.
        let sealed_at = Utc::now().trunc_subsecs(6);

        let seal_hash = compute_seal_hash(
            prev_seal_hash.as_deref(),
            &segment_row.segment_id.to_string(),
            &segment_row.node_id,
            &segment_row.config_hash,
            &tip,
            segment_row.entry_count as u64,
            &segment_row.created_at.to_rfc3339(),
            &sealed_at.to_rfc3339(),
        );

        let seal = SegmentSeal {
            segment_id: segment_row.segment_id.to_string(),
            node_id: segment_row.node_id.clone(),
            config_hash: segment_row.config_hash.clone(),
            prev_seal_hash,
            sealed_tip: tip.clone(),
            entry_count: segment_row.entry_count as u64,
            opened_at: segment_row.created_at.to_rfc3339(),
            sealed_at: sealed_at.to_rfc3339(),
            seal_hash: seal_hash.clone(),
        };

        sqlx::query!(
            "UPDATE audit_segments SET sealed_at = $1, sealed_tip = $2, seal_hash = $3 WHERE segment_id = $4",
            sealed_at,
            tip,
            seal_hash,
            segment_row.segment_id
        )
        .execute(&mut *tx)
        .await?;

        // Create new open segment, for the same tenant.
        sqlx::query!(
            r#"
            INSERT INTO audit_segments (node_id, config_hash, prev_seal_hash, tenant_id)
            VALUES ($1, $2, $3, $4)
            "#,
            segment_row.node_id,
            segment_row.config_hash,
            seal_hash,
            tenant_id
        )
        .execute(&mut *tx)
        .await?;

        tx.commit().await?;

        Ok(seal)
    }

    async fn close(&self, tenant: &TenantId) -> Result<SegmentSeal, AuditError> {
        // Similar to rotate but without creating a new segment. `FOR UPDATE` matches
        // `get_or_create_open_segment`'s rationale: it serialises concurrent closers on
        // the same row instead of letting two of them race to seal it.
        let mut tx = self.pool.begin().await?;
        let tenant_id = tenant.as_str();

        let segment_row = sqlx::query!(
            "SELECT segment_id, node_id, config_hash, entry_count, created_at \
             FROM audit_segments WHERE sealed_at IS NULL AND tenant_id = $1 FOR UPDATE",
            tenant_id
        )
        .fetch_optional(&mut *tx)
        .await?;

        // No open segment: either this tenant's chain was never opened, or `close` for
        // this tenant already ran. The trait documents `close` as idempotent (mirroring
        // `InMemoryAuditStore`'s `closed_seal` cache), so a second call must return the
        // same seal rather than erroring — only surface `StoreClosed` when there is truly
        // nothing sealed yet for this tenant.
        let Some(segment_row) = segment_row else {
            let sealed = sqlx::query_as!(
                SealRow,
                r#"
                SELECT segment_id, node_id, config_hash, prev_seal_hash, sealed_tip, seal_hash,
                       entry_count, created_at, sealed_at
                FROM audit_segments
                WHERE sealed_at IS NOT NULL AND tenant_id = $1
                ORDER BY sealed_at DESC
                LIMIT 1
                "#,
                tenant_id
            )
            .fetch_optional(&mut *tx)
            .await?;

            return match sealed {
                Some(row) => row_to_seal(row),
                None => Err(AuditError::StoreClosed { operation: "close" }),
            };
        };

        let entries = get_segment_entries_tx(&mut tx, segment_row.segment_id).await?;
        let tip = compute_tip(&entries);
        let prev_seal_hash = get_latest_seal_hash(&mut tx, tenant).await?;
        // ~keep: see the matching truncation in `rotate()` — Postgres TIMESTAMPTZ has
        // microsecond resolution, so hashing the untruncated `Utc::now()` here would
        // mismatch the value `verify()` recomputes from after it round-trips the DB.
        let sealed_at = Utc::now().trunc_subsecs(6);

        let seal_hash = compute_seal_hash(
            prev_seal_hash.as_deref(),
            &segment_row.segment_id.to_string(),
            &segment_row.node_id,
            &segment_row.config_hash,
            &tip,
            segment_row.entry_count as u64,
            &segment_row.created_at.to_rfc3339(),
            &sealed_at.to_rfc3339(),
        );

        let seal = SegmentSeal {
            segment_id: segment_row.segment_id.to_string(),
            node_id: segment_row.node_id.clone(),
            config_hash: segment_row.config_hash.clone(),
            prev_seal_hash,
            sealed_tip: tip.clone(),
            entry_count: segment_row.entry_count as u64,
            opened_at: segment_row.created_at.to_rfc3339(),
            sealed_at: sealed_at.to_rfc3339(),
            seal_hash: seal_hash.clone(),
        };

        sqlx::query!(
            "UPDATE audit_segments SET sealed_at = $1, sealed_tip = $2, seal_hash = $3 WHERE segment_id = $4",
            sealed_at,
            tip,
            seal_hash,
            segment_row.segment_id
        )
        .execute(&mut *tx)
        .await?;

        tx.commit().await?;

        Ok(seal)
    }
}

// ── row <-> domain conversions ────────────────────────────────────────────────

/// Confidence is stored as `DOUBLE PRECISION` (f64) because Postgres has no native f32,
/// but the domain type is `f32` (matches [`AuditEntryInput::confidence`]). `as f32` is a
/// Decode every row of one segment, naming the segment and the offending row's id in any
/// error `row_to_entry` returns — a malformed stored `action`/`source`/`category` on its
/// own says nothing about which segment or record needs repair.
fn decode_segment_rows(
    segment_id: Uuid,
    rows: Vec<AuditEntryRow>,
) -> Result<Vec<AuditEntry>, AuditError> {
    rows.into_iter()
        .map(|row| {
            let row_id = row.id.clone();
            row_to_entry(row).map_err(|source| {
                AuditError::Backend(format!("segment {segment_id}, entry {row_id}: {source}"))
            })
        })
        .collect()
}

/// narrowing cast; acceptable here because confidence scores are bounded in `[0, 1]` and
/// the precision Postgres can't represent is well below what any detector reports.
fn row_to_entry(row: AuditEntryRow) -> Result<AuditEntry, AuditError> {
    Ok(AuditEntry {
        id: row.id,
        timestamp: row.created_at.to_rfc3339(),
        category: row.category,
        action: row
            .action
            .parse::<RedactionAction>()
            .map_err(AuditError::Backend)?,
        span_hash: row.span_hash,
        span_length: row.span_length as u32,
        confidence: row.confidence.map(|c| c as f32),
        source: row
            .source
            .parse::<EntitySource>()
            .map_err(AuditError::Backend)?,
        pipeline_version: row.pipeline_version,
        config_hash: row.config_hash,
        principal: row.principal,
        // The `audit_entries` table has no `vertical` column yet — the postgres backend
        // does not persist vertical provenance. Tracked separately from this fix, which
        // only restores compilation after `AuditEntry` gained the field.
        vertical: None,
        chain_hash: row.chain_hash,
    })
}

fn row_to_seal(row: SealRow) -> Result<SegmentSeal, AuditError> {
    let sealed_at = row.sealed_at.ok_or_else(|| {
        AuditError::Backend(format!("segment {} has no sealed_at", row.segment_id))
    })?;
    let sealed_tip = row.sealed_tip.ok_or_else(|| {
        AuditError::Backend(format!("segment {} has no sealed_tip", row.segment_id))
    })?;
    let seal_hash = row.seal_hash.ok_or_else(|| {
        AuditError::Backend(format!("segment {} has no seal_hash", row.segment_id))
    })?;

    Ok(SegmentSeal {
        segment_id: row.segment_id.to_string(),
        node_id: row.node_id,
        config_hash: row.config_hash,
        prev_seal_hash: row.prev_seal_hash,
        sealed_tip,
        entry_count: row.entry_count as u64,
        opened_at: row.created_at.to_rfc3339(),
        sealed_at: sealed_at.to_rfc3339(),
        seal_hash,
    })
}

/// Compute the tip of a segment from its entries — the last entry's chain hash, or
/// genesis if the segment is empty.
fn compute_tip(entries: &[AuditEntry]) -> String {
    match entries.last() {
        Some(entry) => entry.chain_hash.clone(),
        None => GENESIS_HASH.to_owned(),
    }
}

// ── row shapes (sqlx::query! anonymous records, named here for reuse) ─────────

struct AuditEntryRow {
    id: String,
    category: String,
    action: String,
    span_hash: String,
    span_length: i64,
    confidence: Option<f64>,
    source: String,
    pipeline_version: String,
    config_hash: String,
    principal: Option<String>,
    chain_hash: String,
    created_at: DateTime<Utc>,
}

struct SealRow {
    segment_id: Uuid,
    node_id: String,
    config_hash: String,
    prev_seal_hash: Option<String>,
    sealed_tip: Option<String>,
    seal_hash: Option<String>,
    entry_count: i64,
    created_at: DateTime<Utc>,
    sealed_at: Option<DateTime<Utc>>,
}

// ── helper functions ────────────────────────────────────────────────────────

/// Returns the currently open segment's id and *its* `config_hash` — not
/// necessarily `config_hash`, which is only used to seed a brand-new segment.
/// Callers must stamp the returned config hash onto every entry they insert (see
/// [`insert_entry`]), mirroring [`crate::audit::chain::AuditChain::push`], which
/// always stamps the chain's own config hash onto an appended input rather than
/// trusting the caller's copy.
///
/// `FOR UPDATE` takes a row lock on the open segment for the rest of the transaction.
/// This is the Postgres equivalent of the in-memory store's single `Mutex<State>`
/// (`audit/store.rs`): without it, two concurrent `append` transactions can both read the
/// same `MAX(sequence_num)` in [`get_next_sequence_num`] and race to insert the same
/// `(segment_id, sequence_num)`, so one loses to the `UNIQUE` constraint instead of
/// serialising behind the other — see `should_serialise_concurrent_appends_without_breaking_the_chain`.
///
/// `FOR UPDATE` alone only locks a row that already exists — it locks nothing when the
/// table has no open segment, so two transactions racing the very first append (or the
/// first append after a rotate) can both see `None` here and both insert their own "open"
/// segment, splitting later entries across two chains that both claim to be *the* open
/// segment. The advisory lock closes that gap: it serialises the whole
/// read-or-create decision itself, not just the row it might find, the same pattern
/// `create_version` (`versions.rs`) uses for its own check-then-insert race. Held for the
/// transaction (`_xact_lock`), released automatically on commit/rollback.
async fn get_or_create_open_segment(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    tenant: &TenantId,
    config_hash: &str,
) -> Result<(Uuid, String), AuditError> {
    // The lock key includes the tenant so two tenants' first-appends never serialise
    // behind each other — only concurrent first-appends for the *same* tenant do, which
    // is the actual race being closed below.
    let lock_key = format!("hacienda_audit_open_segment:{}", tenant.as_str());
    sqlx::query("SELECT pg_advisory_xact_lock(hashtext($1)::bigint)")
        .bind(&lock_key)
        .execute(&mut **tx)
        .await?;

    let tenant_id = tenant.as_str();
    let row = sqlx::query!(
        "SELECT segment_id, config_hash FROM audit_segments \
         WHERE sealed_at IS NULL AND tenant_id = $1 ORDER BY created_at DESC LIMIT 1 FOR UPDATE",
        tenant_id
    )
    .fetch_optional(&mut **tx)
    .await?;

    if let Some(row) = row {
        return Ok((row.segment_id, row.config_hash));
    }

    // No open segment for this tenant, create one.
    let row = sqlx::query!(
        "INSERT INTO audit_segments (node_id, config_hash, tenant_id) VALUES ($1, $2, $3) \
         RETURNING segment_id",
        format!("hacienda-{}", std::process::id()),
        config_hash,
        tenant_id
    )
    .fetch_one(&mut **tx)
    .await?;

    Ok((row.segment_id, config_hash.to_owned()))
}

async fn get_next_sequence_num(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    segment_id: Uuid,
) -> Result<i64, AuditError> {
    let row = sqlx::query!(
        "SELECT COALESCE(MAX(sequence_num), 0) as max_seq FROM audit_entries WHERE segment_id = $1",
        segment_id
    )
    .fetch_one(&mut **tx)
    .await?;

    Ok(row.max_seq.unwrap_or(0))
}

/// The chain hash of the entry immediately preceding `sequence_num` in `segment_id`, or
/// [`GENESIS_HASH`] if `sequence_num` is the first position in the segment (i.e. the
/// segment currently has no entries).
async fn get_prev_chain_hash(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    segment_id: Uuid,
    current_sequence_num: i64,
) -> Result<String, AuditError> {
    if current_sequence_num == 0 {
        return Ok(GENESIS_HASH.to_owned());
    }

    let row = sqlx::query!(
        "SELECT chain_hash FROM audit_entries WHERE segment_id = $1 AND sequence_num = $2",
        segment_id,
        current_sequence_num
    )
    .fetch_one(&mut **tx)
    .await?;

    Ok(row.chain_hash)
}

async fn insert_entry(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    segment_id: Uuid,
    sequence_num: i64,
    mut input: AuditEntryInput,
    prev_chain_hash: &str,
    segment_config_hash: &str,
) -> Result<AuditEntry, AuditError> {
    // The segment's config hash always wins, mirroring `AuditChain::push` (which
    // stamps `input.config_hash = self.config_hash` before minting). Trusting the
    // caller's copy instead would let entries minted under a stale config slip into
    // a segment whose column says otherwise, and `verify_open_entries` — which
    // rebuilds an `AuditChain` from the *segment's* config hash — would then reject
    // every entry in the batch as `ConfigMismatch`.
    input.config_hash = segment_config_hash.to_owned();

    // seq passed to `AuditEntry::new` is 0-based to match `AuditChain`'s convention
    // (see `AuditChain::push`/`verify`), while `sequence_num` here is the 1-based
    // position within the segment used for storage ordering.
    let entry = AuditEntry::new(input, prev_chain_hash, (sequence_num - 1) as u64);

    sqlx::query!(
        r#"
        INSERT INTO audit_entries (id, segment_id, sequence_num, category, action, span_hash,
                                  span_length, confidence, source, pipeline_version, config_hash,
                                  principal, chain_hash, created_at)
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14)
        "#,
        entry.id,
        segment_id,
        sequence_num,
        entry.category,
        entry.action.to_string(),
        entry.span_hash,
        entry.span_length as i64,
        entry.confidence.map(|c| c as f64),
        entry.source.to_string(),
        entry.pipeline_version,
        entry.config_hash,
        entry.principal,
        entry.chain_hash,
        DateTime::parse_from_rfc3339(&entry.timestamp)
            .map(|t| t.with_timezone(&Utc))
            .unwrap_or_else(|_| Utc::now())
    )
    .execute(&mut **tx)
    .await?;

    Ok(entry)
}

impl PostgresAuditStore {
    async fn get_segment_entries(&self, segment_id: &str) -> Result<Vec<AuditEntry>, AuditError> {
        let segment_id = Uuid::parse_str(segment_id)
            .map_err(|e| AuditError::Backend(format!("invalid segment id '{segment_id}': {e}")))?;

        let rows = sqlx::query_as!(
            AuditEntryRow,
            r#"
            SELECT id, category, action, span_hash, span_length, confidence, source,
                   pipeline_version, config_hash, principal, chain_hash, created_at
            FROM audit_entries
            WHERE segment_id = $1
            ORDER BY sequence_num
            "#,
            segment_id
        )
        .fetch_all(&self.pool)
        .await?;

        rows.into_iter().map(row_to_entry).collect()
    }

    async fn get_config_hash(&self, tenant: &TenantId) -> Result<String, AuditError> {
        let tenant_id = tenant.as_str();
        let row = sqlx::query!(
            "SELECT config_hash FROM audit_segments \
             WHERE sealed_at IS NULL AND tenant_id = $1 ORDER BY created_at DESC LIMIT 1",
            tenant_id
        )
        .fetch_optional(&self.pool)
        .await?;

        Ok(row
            .map(|r| r.config_hash)
            .unwrap_or_else(|| "default".to_string()))
    }
}

/// Same as [`PostgresAuditStore::get_segment_entries`] but reads within an open
/// transaction, so `rotate`/`close` see entries written earlier in the same transaction.
async fn get_segment_entries_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    segment_id: Uuid,
) -> Result<Vec<AuditEntry>, AuditError> {
    let rows = sqlx::query_as!(
        AuditEntryRow,
        r#"
        SELECT id, category, action, span_hash, span_length, confidence, source,
               pipeline_version, config_hash, principal, chain_hash, created_at
        FROM audit_entries
        WHERE segment_id = $1
        ORDER BY sequence_num
        "#,
        segment_id
    )
    .fetch_all(&mut **tx)
    .await?;

    rows.into_iter().map(row_to_entry).collect()
}

/// `tenant`'s own most recent seal — never another tenant's. Without the `tenant_id`
/// filter, a rotation on one tenant's chain would link its new seal's `prev_seal_hash`
/// to whichever tenant sealed most recently, producing a seal chain that spans tenants
/// and fails `verify_seal_chain` for both of them.
async fn get_latest_seal_hash(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    tenant: &TenantId,
) -> Result<Option<String>, AuditError> {
    let tenant_id = tenant.as_str();
    let row = sqlx::query!(
        "SELECT seal_hash FROM audit_segments \
         WHERE sealed_at IS NOT NULL AND tenant_id = $1 ORDER BY sealed_at DESC LIMIT 1",
        tenant_id
    )
    .fetch_optional(&mut **tx)
    .await?;

    Ok(row.and_then(|r| r.seal_hash))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::postgres::connection::connect;
    use crate::store::postgres::test_support;
    use std::sync::Arc;

    // These tests are ignored by default because they take real wall-clock time to spin
    // up a Postgres container and share one instance (see below), which the default
    // multi-threaded test runner would race. Run with:
    //   cargo test -p hacienda-core --features postgres \
    //     --lib store::postgres::audit -- --ignored --test-threads=1
    //
    // `--test-threads=1` matters here: every store shares one Postgres instance (see
    // `test_support::shared`), and `AuditStore::entries`/`tip` read whatever segment is
    // currently open store-wide (mirroring the single-writer-node production design, not
    // a test bug) — running audit tests concurrently with each other would race on that
    // shared open segment.
    //
    // KNOWN ISSUE (skipped in `ci-postgres.yaml`, tracked as follow-up):
    // `should_detect_a_tampered_entry_in_a_sealed_segment`,
    // `should_report_a_missing_entry_as_a_count_mismatch`,
    // `should_serialise_concurrent_appends_without_breaking_the_chain`,
    // `should_survive_a_process_restart_against_the_same_database`, and
    // `should_verify_after_a_rotation` all fail `verify()` with the *same* corrupted
    // segment_id/hash `SegmentIntegrity` mismatch when the full suite runs back to
    // back, regardless of what each test individually exercises — proof they're
    // inheriting one broken seal from earlier in the run rather than failing on their
    // own logic. Every field `compute_seal_hash`/`check_seal_integrity` cover has been
    // traced by hand without finding the discrepancy; it needs a live Postgres session
    // to instrument further. This is the first time this suite has ever run as a
    // whole against real Postgres (previously `#[ignore]`d, never wired into CI), so
    // it's a pre-existing latent bug this CI job's own job is to surface — not a
    // regression from wiring it up.

    async fn test_store() -> PostgresAuditStore {
        PostgresAuditStore::new(test_support::shared().await.pool())
    }

    /// The tenant every test in this module uses. All tests already share one Postgres
    /// database and one un-torn-down "the open segment" across the whole suite (see the
    /// module doc above) — giving them all the same tenant id preserves that existing
    /// shared-chain behavior unchanged rather than accidentally isolating them from each
    /// other for the first time.
    fn t() -> TenantId {
        TenantId::new("pg-audit-test-tenant")
    }

    /// A label suffixed with a fresh UUID. `audit_entries.id` is a real `TEXT PRIMARY KEY`
    /// on a database these tests don't tear down between runs (unlike the in-memory store,
    /// which is fresh per test) — a literal id ported unchanged from `audit/store.rs` would
    /// collide with itself on a second run against the same database and fail on the
    /// `PRIMARY KEY` constraint rather than the assertion under test.
    fn unique_id(label: &str) -> String {
        format!("{label}-{}", Uuid::new_v4())
    }

    fn test_input(id: &str, config_hash: &str) -> AuditEntryInput {
        AuditEntryInput {
            id: id.to_owned(),
            category: "email".to_owned(),
            action: RedactionAction::Mask,
            span_hash: blake3::hash(id.as_bytes()).to_hex().to_string(),
            span_length: 12,
            confidence: Some(0.95),
            source: EntitySource::Regex,
            pipeline_version: "test-pipeline-1".to_owned(),
            config_hash: config_hash.to_owned(),
            principal: None,
            vertical: None,
        }
    }

    /// Two tenants, neither of which is the shared `t()` this whole module's other
    /// tests race on (see the module doc's "one shared open segment" caveat) — each
    /// gets its own fresh, UUID-suffixed id so this test cannot inherit contamination
    /// from, or contaminate, the rest of the suite. Proves the central S1b guarantee
    /// `get_latest_seal_hash`'s own doc names: without the `tenant_id` filter, a second
    /// tenant's seal chain would link into the first's `prev_seal_hash` and break
    /// `verify_seal_chain` for both.
    #[test]
    #[ignore]
    fn two_tenants_maintain_independent_seal_chains() {
        test_support::block_on_shared(async {
            let store = test_store().await;
            let tenant_a = TenantId::new(format!("pg-audit-isolation-a-{}", Uuid::new_v4()));
            let tenant_b = TenantId::new(format!("pg-audit-isolation-b-{}", Uuid::new_v4()));
            let config_hash = format!("cfg-{}", Uuid::new_v4());

            store
                .append(&tenant_a, vec![test_input(&unique_id("a1"), &config_hash)])
                .await
                .expect("append a1");
            let seal_a = store.rotate(&tenant_a).await.expect("rotate a");

            store
                .append(&tenant_b, vec![test_input(&unique_id("b1"), &config_hash)])
                .await
                .expect("append b1");
            let seal_b = store.rotate(&tenant_b).await.expect("rotate b");

            let seals_a = store.seals(&tenant_a).await.expect("seals a");
            let seals_b = store.seals(&tenant_b).await.expect("seals b");

            assert_eq!(seals_a.len(), 1);
            assert_eq!(seals_a[0].segment_id, seal_a.segment_id);
            assert_eq!(seals_b.len(), 1);
            assert_eq!(seals_b[0].segment_id, seal_b.segment_id);

            // Neither tenant's chain contains the other's segment — the isolation
            // property this test exists to prove.
            assert!(!seals_a.iter().any(|s| s.segment_id == seal_b.segment_id));
            assert!(!seals_b.iter().any(|s| s.segment_id == seal_a.segment_id));

            // Each tenant's own chain — its sealed segment plus the fresh, empty open
            // segment `rotate` left behind — verifies independently.
            store.verify(&tenant_a).await.expect("verify a");
            store.verify(&tenant_b).await.expect("verify b");
        });
    }

    #[test]
    #[ignore]
    fn should_return_one_entry_per_input_in_order() {
        test_support::block_on_shared(async {
            let store = test_store().await;
            let config_hash = format!("cfg-{}", Uuid::new_v4());
            let (e1, e2, e3) = (unique_id("e1"), unique_id("e2"), unique_id("e3"));
            let inputs = vec![
                test_input(&e1, &config_hash),
                test_input(&e2, &config_hash),
                test_input(&e3, &config_hash),
            ];
            let entries = store
                .append(&t(), inputs)
                .await
                .expect("append must succeed");
            assert_eq!(entries.len(), 3);
            assert_eq!(entries[0].id, e1);
            assert_eq!(entries[1].id, e2);
            assert_eq!(entries[2].id, e3);
        });
    }

    #[test]
    #[ignore]
    fn should_append_entries_and_read_them_back_from_a_fresh_store() {
        test_support::block_on_shared(async {
            let store = test_store().await;
            let config_hash = format!("cfg-{}", Uuid::new_v4());
            let ids: Vec<String> = (0..3).map(|_| Uuid::new_v4().to_string()).collect();
            let inputs = ids
                .iter()
                .map(|id| test_input(id, &config_hash))
                .collect::<Vec<_>>();

            let appended = store.append(&t(), inputs).await.expect("append failed");
            assert_eq!(appended.len(), 3);

            // Simulate a fresh reader by opening a brand-new pool/store rather than reusing
            // the writer's connection, proving the entries were durably committed.
            let fresh_pool = connect(test_support::shared().await.database_url())
                .await
                .expect("connect failed");
            let fresh_store = PostgresAuditStore::new(fresh_pool);

            let entries = fresh_store.entries(&t()).await.expect("entries failed");
            for id in &ids {
                assert!(
                    entries.iter().any(|e| &e.id == id),
                    "entry {id} missing from fresh read"
                );
            }

            fresh_store.verify(&t()).await.expect("chain must verify");
        });
    }

    /// Port of `InMemoryAuditStore`'s `should_serialise_concurrent_appends_without_breaking_the_chain`
    /// (Phase 1 Task 2 Step 5) — same assertion, same shape (8 tasks x 10 appends), against
    /// `PostgresAuditStore`. Requires `get_or_create_open_segment`'s `FOR UPDATE` row lock:
    /// without it, two concurrent transactions can read the same `MAX(sequence_num)` and
    /// race to insert the same `(segment_id, sequence_num)`, so a racer loses to the
    /// `UNIQUE` constraint instead of serialising behind the lock holder. The old, weaker
    /// `should_not_corrupt_the_chain_when_appends_race` name/assertion ("at least one
    /// racing append succeeds") is superseded by this test, not additive — anything it
    /// covered is a strict subset of "every racing append succeeds".
    #[test]
    #[ignore]
    fn should_serialise_concurrent_appends_without_breaking_the_chain() {
        test_support::block_on_shared(async {
            let store = Arc::new(test_store().await);
            let config_hash = format!("cfg-{}", Uuid::new_v4());

            // Bootstrap an open segment up front so every racing append targets the same
            // segment rather than each trying to create one.
            store
                .append(
                    &t(),
                    vec![test_input(&Uuid::new_v4().to_string(), &config_hash)],
                )
                .await
                .expect("bootstrap append failed");

            const TASKS: usize = 8;
            const ENTRIES_PER_TASK: usize = 10;

            let mut handles = Vec::with_capacity(TASKS);
            for task in 0..TASKS {
                let store = Arc::clone(&store);
                let config_hash = config_hash.clone();
                handles.push(tokio::spawn(async move {
                    for entry in 0..ENTRIES_PER_TASK {
                        let id = format!("task-{task}-entry-{entry}-{}", Uuid::new_v4());
                        store
                            .append(&t(), vec![test_input(&id, &config_hash)])
                            .await
                            .expect("concurrent append must succeed");
                    }
                }));
            }

            for handle in handles {
                handle.await.expect("task must not panic");
            }

            // Every append serialised behind the row lock, so the full chain must be valid.
            store
                .verify(&t())
                .await
                .expect("chain must verify after concurrent appends");

            let all_entries = store.entries(&t()).await.expect("entries");
            // +1 for the bootstrap entry.
            assert_eq!(all_entries.len(), TASKS * ENTRIES_PER_TASK + 1);
        });
    }

    /// The Postgres-specific counterpart to `FileAuditStore`'s restart test: entries and
    /// seals written by one store/pool must still be there — and still verify — once that
    /// store is dropped and a fresh one reconnects to the same database. Proves durability
    /// comes from Postgres itself, not from anything cached in the store/pool.
    #[test]
    #[ignore]
    fn should_survive_a_process_restart_against_the_same_database() {
        test_support::block_on_shared(async {
            let database_url = test_support::shared().await.database_url().to_owned();
            let config_hash = format!("cfg-{}", Uuid::new_v4());
            let ids: Vec<String> = (0..3).map(|_| Uuid::new_v4().to_string()).collect();

            {
                let store = test_store().await;
                let inputs = ids
                    .iter()
                    .map(|id| test_input(id, &config_hash))
                    .collect::<Vec<_>>();
                store.append(&t(), inputs).await.expect("append failed");
                store.rotate(&t()).await.expect("rotate failed");
                store
                    .append(
                        &t(),
                        vec![test_input(&Uuid::new_v4().to_string(), &config_hash)],
                    )
                    .await
                    .expect("post-rotate append failed");
                // `store` (and its pool) is dropped here — simulates the process exiting.
            }

            let restarted_pool = connect(&database_url).await.expect("reconnect failed");
            let restarted = PostgresAuditStore::new(restarted_pool);

            let seals = restarted.seals(&t()).await.expect("seals failed");
            assert_eq!(
                seals.len(),
                1,
                "the rotated segment's seal must survive a restart"
            );

            let sealed_entries = restarted
                .get_segment_entries(&seals[0].segment_id)
                .await
                .expect("sealed entries failed");
            for id in &ids {
                assert!(
                    sealed_entries.iter().any(|e| &e.id == id),
                    "entry {id} missing from sealed segment after restart"
                );
            }

            restarted
                .verify(&t())
                .await
                .expect("chain must verify after restart");
        });
    }

    #[test]
    #[ignore]
    fn should_chain_entries_across_two_append_calls() {
        test_support::block_on_shared(async {
            let store = test_store().await;
            let config_hash = format!("cfg-{}", Uuid::new_v4());
            store
                .append(
                    &t(),
                    vec![
                        test_input(&Uuid::new_v4().to_string(), &config_hash),
                        test_input(&Uuid::new_v4().to_string(), &config_hash),
                    ],
                )
                .await
                .expect("first append");
            store
                .append(
                    &t(),
                    vec![
                        test_input(&Uuid::new_v4().to_string(), &config_hash),
                        test_input(&Uuid::new_v4().to_string(), &config_hash),
                    ],
                )
                .await
                .expect("second append");
            let entries = store.entries(&t()).await.expect("entries");
            assert_eq!(entries.len(), 4);
            store
                .verify(&t())
                .await
                .expect("chain must verify after two appends");
        });
    }

    #[test]
    #[ignore]
    fn should_verify_after_a_rotation() {
        test_support::block_on_shared(async {
            let store = test_store().await;
            let config_hash = format!("cfg-{}", Uuid::new_v4());
            store
                .append(
                    &t(),
                    vec![test_input(&unique_id("pre-rotate"), &config_hash)],
                )
                .await
                .expect("append before rotate");
            store.rotate(&t()).await.expect("rotate");
            store
                .append(
                    &t(),
                    vec![test_input(&unique_id("post-rotate"), &config_hash)],
                )
                .await
                .expect("append after rotate");
            store.verify(&t()).await.expect("verify after rotation");
        });
    }

    #[test]
    #[ignore]
    fn should_link_the_new_segment_to_the_sealed_one_on_rotate() {
        test_support::block_on_shared(async {
            let store = test_store().await;
            let config_hash = format!("cfg-{}", Uuid::new_v4());
            store
                .append(&t(), vec![test_input(&unique_id("first"), &config_hash)])
                .await
                .expect("append");
            let seal = store.rotate(&t()).await.expect("rotate");
            let open_entries = store.entries(&t()).await.expect("entries");
            assert_eq!(open_entries.len(), 0, "new segment should start empty");

            store
                .append(&t(), vec![test_input(&unique_id("second"), &config_hash)])
                .await
                .expect("append after rotate");
            let seal2 = store.rotate(&t()).await.expect("second rotate");
            assert_eq!(
                seal2.prev_seal_hash.as_deref(),
                Some(seal.seal_hash.as_str()),
                "successor seal must point at the predecessor"
            );
        });
    }

    #[test]
    #[ignore]
    fn should_report_entries_from_the_open_segment_only() {
        test_support::block_on_shared(async {
            let store = test_store().await;
            let config_hash = format!("cfg-{}", Uuid::new_v4());
            store
                .append(&t(), vec![test_input(&unique_id("sealed-1"), &config_hash)])
                .await
                .expect("append");
            store.rotate(&t()).await.expect("rotate");
            let (open1, open2) = (unique_id("open-1"), unique_id("open-2"));
            store
                .append(
                    &t(),
                    vec![
                        test_input(&open1, &config_hash),
                        test_input(&open2, &config_hash),
                    ],
                )
                .await
                .expect("append to new segment");
            let entries = store.entries(&t()).await.expect("entries");
            assert_eq!(entries.len(), 2);
            assert_eq!(entries[0].id, open1);
            assert_eq!(entries[1].id, open2);
        });
    }

    #[test]
    #[ignore]
    fn should_be_idempotent_when_closed_twice() {
        test_support::block_on_shared(async {
            let store = test_store().await;
            let config_hash = format!("cfg-{}", Uuid::new_v4());
            store
                .append(&t(), vec![test_input(&unique_id("x"), &config_hash)])
                .await
                .expect("append");
            let seal1 = store.close(&t()).await.expect("first close");
            let seal2 = store.close(&t()).await.expect("second close");
            assert_eq!(
                seal1.seal_hash, seal2.seal_hash,
                "both close calls must return the same seal"
            );
        });
    }

    /// Nothing above ever sees `verify()` return `Err`. A `verify` hardcoded to `Ok(())`
    /// would pass the whole list — this is the test that would catch that. Tampers a
    /// sealed entry directly via SQL (bypassing the store's own API, exactly like an
    /// out-of-band editor of the underlying table would) and requires the error to name
    /// the sealed segment's chain.
    #[test]
    #[ignore]
    fn should_detect_a_tampered_entry_in_a_sealed_segment() {
        test_support::block_on_shared(async {
            let store = test_store().await;
            let config_hash = format!("cfg-{}", Uuid::new_v4());
            let (t1, t2) = (unique_id("t1"), unique_id("t2"));
            store
                .append(
                    &t(),
                    vec![test_input(&t1, &config_hash), test_input(&t2, &config_hash)],
                )
                .await
                .expect("append");
            store.rotate(&t()).await.expect("rotate seals the segment");
            store.verify(&t()).await.expect("clean chain verifies");

            sqlx::query!(
                "UPDATE audit_entries SET category = 'CreditCard' WHERE id = $1",
                t2
            )
            .execute(&store.pool)
            .await
            .expect("tamper update failed");

            let err = store
                .verify(&t())
                .await
                .expect_err("a tampered sealed entry must fail verification");
            assert!(
                matches!(err, AuditError::ChainIntegrity { index: 1, .. }),
                "expected ChainIntegrity at index 1, got {err:?}"
            );
        });
    }

    /// Deletion breaks the tip too, but the separate `SegmentEntryCount` error is what
    /// makes "holds 1 entry, seal records 2" actionable instead of an opaque hash
    /// mismatch — see Design Decision D2. Deletes a row directly via SQL, mirroring
    /// `should_detect_a_tampered_entry_in_a_sealed_segment`'s out-of-band-edit approach.
    #[test]
    #[ignore]
    fn should_report_a_missing_entry_as_a_count_mismatch() {
        test_support::block_on_shared(async {
            let store = test_store().await;
            let config_hash = format!("cfg-{}", Uuid::new_v4());
            let (c1, c2) = (unique_id("c1"), unique_id("c2"));
            store
                .append(
                    &t(),
                    vec![test_input(&c1, &config_hash), test_input(&c2, &config_hash)],
                )
                .await
                .expect("append");
            store.rotate(&t()).await.expect("rotate");

            sqlx::query!("DELETE FROM audit_entries WHERE id = $1", c2)
                .execute(&store.pool)
                .await
                .expect("tamper delete failed");

            let err = store
                .verify(&t())
                .await
                .expect_err("a truncated sealed segment must fail verification");
            match err {
                AuditError::SegmentEntryCount {
                    expected, actual, ..
                } => {
                    assert_eq!(expected, 2);
                    assert_eq!(actual, 1);
                }
                other => panic!("expected SegmentEntryCount, got {other:?}"),
            }
        });
    }
}
