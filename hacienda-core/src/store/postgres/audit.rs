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
/// PostgresAuditStore struct
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
        let rows = sqlx::query_as::<_, AuditEntryRow>(
            r#"
            SELECT id, category, action, span_hash,
                   span_length, confidence, source, pipeline_version, config_hash, principal,
                   vertical, model, chain_hash, created_at
            FROM audit_entries
            WHERE segment_id = (
                SELECT segment_id FROM audit_segments
                WHERE sealed_at IS NULL AND tenant_id = $1
                ORDER BY created_at DESC
                LIMIT 1
            )
            ORDER BY sequence_num
            "#,
        )
        .bind(tenant_id)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| AuditError::Backend(format!("entries: fetch open segment for tenant {}: {}", tenant_id, e)))?;

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

        let sealed_extents: Vec<ExtentRow> = sqlx::query_as::<_, ExtentRow>(
            r#"
            SELECT segment_id, entry_count
            FROM audit_segments
            WHERE sealed_at IS NOT NULL AND tenant_id = $1
            ORDER BY created_at
            "#,
        )
        .bind(tenant_id)
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
        let open_row: Option<ExtentRow> = sqlx::query_as::<_, ExtentRow>(
            r#"
            SELECT segment_id, entry_count
            FROM audit_segments
            WHERE sealed_at IS NULL AND tenant_id = $1
            ORDER BY created_at DESC
            LIMIT 1
            "#,
        )
        .bind(tenant_id)
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
            let rows = sqlx::query_as::<_, AuditEntryRow>(
                r#"
                SELECT id, category, action, span_hash, span_length, confidence, source,
                       pipeline_version, config_hash, principal, vertical, model, chain_hash, created_at
                FROM audit_entries
                WHERE segment_id = $1
                ORDER BY sequence_num
                "#,
            )
            .bind(row.segment_id)
            .fetch_all(&mut *tx)
            .await?;
            all_segment_entries.push(decode_segment_rows(row.segment_id, rows)?);
        }

        // Open segment, keyed by the id captured above — not re-queried.
        if let Some(row) = &open_row {
            let open_rows = sqlx::query_as::<_, AuditEntryRow>(
                r#"
                SELECT id, category, action, span_hash, span_length, confidence, source,
                       pipeline_version, config_hash, principal, vertical, model, chain_hash, created_at
                FROM audit_entries
                WHERE segment_id = $1
                ORDER BY sequence_num
                "#,
            )
            .bind(row.segment_id)
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
        let rows = sqlx::query_as::<_, SealRow>(
            r#"
            SELECT segment_id, tenant_id, node_id, config_hash, prev_seal_hash, sealed_tip, seal_hash,
                   entry_count, created_at, sealed_at
            FROM audit_segments
            WHERE sealed_at IS NOT NULL AND tenant_id = $1
            ORDER BY created_at
            "#,
        )
        .bind(tenant_id)
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
            tenant_id,
            &segment_row.node_id,
            &segment_row.config_hash,
            &tip,
            segment_row.entry_count as u64,
            &segment_row.created_at.to_rfc3339(),
            &sealed_at.to_rfc3339(),
        );

        let seal = SegmentSeal {
            segment_id: segment_row.segment_id.to_string(),
            tenant_id: tenant_id.to_string(),
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
            let sealed = sqlx::query_as::<_, SealRow>(
                r#"
                SELECT segment_id, tenant_id, node_id, config_hash, prev_seal_hash, sealed_tip, seal_hash,
                       entry_count, created_at, sealed_at
                FROM audit_segments
                WHERE sealed_at IS NOT NULL AND tenant_id = $1
                ORDER BY sealed_at DESC
                LIMIT 1
                "#,
            )
            .bind(tenant_id)
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
            tenant_id,
            &segment_row.node_id,
            &segment_row.config_hash,
            &tip,
            segment_row.entry_count as u64,
            &segment_row.created_at.to_rfc3339(),
            &sealed_at.to_rfc3339(),
        );

        let seal = SegmentSeal {
            segment_id: segment_row.segment_id.to_string(),
            tenant_id: tenant_id.to_string(),
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

// ── row <-> domain conversions ───────────────────────────────────────

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
        vertical: row.vertical,
        model: row.model,
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
        tenant_id: row.tenant_id,
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

#[derive(sqlx::FromRow, Debug)]
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
    vertical: Option<String>,
    model: Option<String>,
    chain_hash: String,
    created_at: DateTime<Utc>,
}

#[derive(sqlx::FromRow, Debug)]
struct SealRow {
    segment_id: Uuid,
    tenant_id: String,
    node_id: String,
    config_hash: String,
    prev_seal_hash: Option<String>,
    sealed_tip: Option<String>,
    seal_hash: Option<String>,
    entry_count: i64,
    created_at: DateTime<Utc>,
    sealed_at: Option<DateTime<Utc>>,
}

// ── helper functions ─────────────────────────────────────────

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

    // No open segment for this tenant, create one. `prev_seal_hash` must link to
    // whatever segment is currently this tenant's latest sealed one — the *only*
    // other place a segment gets created, `rotate()`'s inline "create new open
    // segment" insert, already does this (it has `prev_seal_hash` on hand from
    // computing the segment it just sealed). This path is reached whenever `append`
    // finds no open segment at all — notably right after a bare `close()` (which
    // seals without opening a successor) — and used to leave `prev_seal_hash`
    // unset. That produced a segment whose *own* seal, once this one is itself
    // later sealed, is computed against the true latest seal hash (`rotate`/
    // `close` both derive it fresh via `get_latest_seal_hash`) but whose stored
    // `prev_seal_hash` column — the value `verify()` reads back — stayed `NULL`,
    // permanently failing `check_seal_integrity` for that segment regardless of
    // any tampering.
    let prev_seal_hash = get_latest_seal_hash(tx, tenant).await?;

    let row = sqlx::query!(
        "INSERT INTO audit_segments (node_id, config_hash, tenant_id, prev_seal_hash) \
         VALUES ($1, $2, $3, $4) RETURNING segment_id",
        format!("hacienda-{}", std::process::id()),
        config_hash,
        tenant_id,
        prev_seal_hash
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

    sqlx::query(
        r#"
        INSERT INTO audit_entries (id, segment_id, sequence_num, category, action, span_hash,
                                  span_length, confidence, source, pipeline_version, config_hash,
                                  principal, vertical, model, chain_hash, created_at)
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16)
        "#,
    )
    .bind(entry.id.clone())
    .bind(segment_id)
    .bind(sequence_num)
    .bind(entry.category.clone())
    .bind(entry.action.to_string())
    .bind(entry.span_hash.clone())
    .bind(entry.span_length as i64)
    .bind(entry.confidence.map(|c| c as f64))
    .bind(entry.source.to_string())
    .bind(entry.pipeline_version.clone())
    .bind(entry.config_hash.clone())
    .bind(entry.principal.clone())
    .bind(entry.vertical.clone())
    .bind(entry.model.clone())
    .bind(entry.chain_hash.clone())
    .bind(DateTime::parse_from_rfc3339(&entry.timestamp)
        .map(|t| t.with_timezone(&Utc))
        .unwrap_or_else(|_| Utc::now()))
    .execute(&mut **tx)
    .await?;

    Ok(entry)
}

impl PostgresAuditStore {
    async fn get_segment_entries(&self, segment_id: &str) -> Result<Vec<AuditEntry>, AuditError> {
        let segment_id = Uuid::parse_str(segment_id)
            .map_err(|e| AuditError::Backend(format!("invalid segment id '{segment_id}': {e}")))?;

        let rows = sqlx::query_as::<_, AuditEntryRow>(
            r#"
            SELECT id, category, action, span_hash, span_length, confidence, source,
                   pipeline_version, config_hash, principal, vertical, model, chain_hash, created_at
            FROM audit_entries
            WHERE segment_id = $1
            ORDER BY sequence_num
            "#,
        )
        .bind(segment_id)
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
    let rows = sqlx::query_as::<_, AuditEntryRow>(
        r#"
        SELECT id, category, action, span_hash, span_length, confidence, source,
               pipeline_version, config_hash, principal, vertical, model, chain_hash, created_at
        FROM audit_entries
        WHERE segment_id = $1
        ORDER BY sequence_num
        "#,
    )
    .bind(segment_id)
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
    use sqlx::Row;
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
    // Two independent bugs used to make this suite fail as a whole (all five tests
    // below the count previously listed here). Both are fixed now:
    //
    // 1. `get_or_create_open_segment` (production code) never set the new segment's
    //    `prev_seal_hash` column when creating one outside of `rotate`'s own inline
    //    path — notably right after a bare `close()`, which seals without opening a
    //    successor. That segment's *own* seal, computed later by `rotate`/`close` via
    //    `get_latest_seal_hash` (the true latest seal at sealing time), then never
    //    matched what `verify()` read back from the stale-`NULL` column — a permanent
    //    `SegmentIntegrity` mismatch with no tampering involved. Fixed by having
    //    `get_or_create_open_segment` populate `prev_seal_hash` the same way `rotate`
    //    already does.
    // 2. Tests below that assert on an entry's *position* within its segment (e.g.
    //    "the tampered entry is at index 1") assumed they owned a fresh, empty open
    //    segment — not true in this shared-database suite (see below), where an
    //    earlier test's un-rotated entries are still sitting in the same open segment.
    //    Fixed by [`start_fresh_segment`] sealing away any such leftovers before a test
    //    that needs exclusive ownership of what it appends next.

    async fn test_store() -> PostgresAuditStore {
        let pool = test_support::shared().await.pool();
        // `fk_audit_segments_tenant` (migration 0005) requires the tenant to already
        // exist — only `default` is seeded by migration, so this module's shared
        // literal tenant must be admitted before any test's first `append`.
        test_support::ensure_tenant(&pool, &t()).await;
        PostgresAuditStore::new(pool)
    }

    /// The tenant every test in this module uses. All tests already share one Postgres
    /// database and one un-torn-down "the open segment" across the whole suite (see the
    /// module doc above) — giving them all the same tenant id preserves that existing
    /// shared-chain behavior unchanged rather than accidentally isolating them from each
    /// other for the first time.
    fn t() -> TenantId {
        TenantId::new("pg-audit-test-tenant")
    }

    /// Seals away any leftover open-segment entries from an earlier test in this
    /// shared-database suite (see the module doc above) so a test that asserts on an
    /// entry's *position* within its segment can rely on starting from an empty one.
    async fn start_fresh_segment(store: &PostgresAuditStore) {
        if !store
            .entries(&t())
            .await
            .expect("reading the open segment")
            .is_empty()
        {
            store
                .rotate(&t())
                .await
                .expect("sealing away a leftover open segment");
        }
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
            model: None,
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
            let pool = test_support::shared().await.pool();
            let tenant_a = TenantId::new(format!("pg-audit-isolation-a-{}", Uuid::new_v4()));
            let tenant_b = TenantId::new(format!("pg-audit-isolation-b-{}", Uuid::new_v4()));
            test_support::ensure_tenant(&pool, &tenant_a).await;
            test_support::ensure_tenant(&pool, &tenant_b).await;
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
            let store = test_store().await;
            start_fresh_segment(&store).await;
            let store = Arc::new(store);
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

            let rotated_segment_id;
            {
                let store = test_store().await;
                start_fresh_segment(&store).await;
                let inputs = ids
                    .iter()
                    .map(|id| test_input(id, &config_hash))
                    .collect::<Vec<_>>();
                store.append(&t(), inputs).await.expect("append failed");
                let seal = store.rotate(&t()).await.expect("rotate failed");
                rotated_segment_id = seal.segment_id;
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

            // Not `seals.len() == 1`: this suite shares one Postgres instance across
            // every test in the module (see the module doc comment), so other tests'
            // sealed segments legitimately coexist here. What must be true is that
            // *this* test's own rotated segment is among them.
            let seals = restarted.seals(&t()).await.expect("seals failed");
            let seal = seals
                .iter()
                .find(|s| s.segment_id == rotated_segment_id)
                .expect("the rotated segment's seal must survive a restart");

            let sealed_entries = restarted
                .get_segment_entries(&seal.segment_id)
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
            start_fresh_segment(&store).await;
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
    ///
    /// Restores the tampered row before returning. `entries()`/`verify()` deliberately
    /// scan the *whole* database, store-wide (mirroring the single-writer production
    /// design — see this module's doc comment), not just this test's own segment; every
    /// test in this file shares one Postgres instance (`test_support::shared`), so a
    /// tamper left in place here would fail every later test's `verify()` on a
    /// corruption that has nothing to do with what that test exercises. This was the
    /// root cause of the `SegmentIntegrity`-mismatch bug this suite used to hit as a
    /// whole (see `ci-postgres.yaml`'s former `--skip` list for these five tests) — not a
    /// bug in the seal-chain logic itself, which `store_file.rs`'s equivalent test (a
    /// fresh `TempDir` per test, so nothing to leak) already proved sound.
    #[test]
    #[ignore]
    fn should_detect_a_tampered_entry_in_a_sealed_segment() {
        test_support::block_on_shared(async {
            let store = test_store().await;
            start_fresh_segment(&store).await;
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

            // Plain (non-macro) query: no `.sqlx` offline-cache entry needed for a
            // query this narrowly test-only, unlike the tamper/restore pair below it.
            sqlx::query("UPDATE audit_entries SET category = 'CreditCard' WHERE id = $1")
                .bind(&t2)
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

            // Restore — see the doc comment above for why this must not be skipped.
            // `test_input`'s `category` is always `"email"`; this is the one and only
            // value this test ever tampers it to away from, so restoring to that
            // literal (rather than capturing-and-restoring, as the delete test below
            // must) is exact and sufficient.
            sqlx::query("UPDATE audit_entries SET category = 'email' WHERE id = $1")
                .bind(&t2)
                .execute(&store.pool)
                .await
                .expect("restore after tamper failed");
            store
                .verify(&t())
                .await
                .expect("chain must verify again once the tamper is undone");
        });
    }

    /// Deletion breaks the tip too, but the separate `SegmentEntryCount` error is what
    /// makes "holds 1 entry, seal records 2" actionable instead of an opaque hash
    /// mismatch — see Design Decision D2. Deletes a row directly via SQL, mirroring
    /// `should_detect_a_tampered_entry_in_a_sealed_segment`'s out-of-band-edit approach.
    ///
    /// Restores the deleted row before returning — see the sibling test's doc comment
    /// for why leaving a tamper in place corrupts every later test in this shared-database
    /// suite, not just this one. Deletion can't be undone with a literal like the sibling
    /// test's `UPDATE ... SET category = 'email'`: the whole row is gone, so this captures
    /// every column beforehand and re-inserts it verbatim afterward.
    #[test]
    #[ignore]
    fn should_report_a_missing_entry_as_a_count_mismatch() {
        test_support::block_on_shared(async {
            let store = test_store().await;
            start_fresh_segment(&store).await;
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

            let captured = sqlx::query(
                "SELECT id, segment_id, sequence_num, category, action, span_hash, \
                 span_length, confidence, source, pipeline_version, config_hash, \
                 principal, chain_hash, created_at \
                 FROM audit_entries WHERE id = $1",
            )
            .bind(&c2)
            .fetch_one(&store.pool)
            .await
            .expect("capturing the row before deleting it failed");

            sqlx::query("DELETE FROM audit_entries WHERE id = $1")
                .bind(&c2)
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

            // Restore — see the doc comment above.
            sqlx::query(
                "INSERT INTO audit_entries \
                 (id, segment_id, sequence_num, category, action, span_hash, span_length, \
                  confidence, source, pipeline_version, config_hash, principal, chain_hash, \
                  created_at) \
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14)",
            )
            .bind(captured.get::<String, _>("id"))
            .bind(captured.get::<Uuid, _>("segment_id"))
            .bind(captured.get::<i64, _>("sequence_num"))
            .bind(captured.get::<String, _>("category"))
            .bind(captured.get::<String, _>("action"))
            .bind(captured.get::<String, _>("span_hash"))
            .bind(captured.get::<i64, _>("span_length"))
            .bind(captured.get::<Option<f64>, _>("confidence"))
            .bind(captured.get::<String, _>("source"))
            .bind(captured.get::<String, _>("pipeline_version"))
            .bind(captured.get::<String, _>("config_hash"))
            .bind(captured.get::<Option<String>, _>("principal"))
            .bind(captured.get::<String, _>("chain_hash"))
            .bind(captured.get::<DateTime<Utc>, _>("created_at"))
            .execute(&store.pool)
            .await
            .expect("restoring the deleted row failed");
            store
                .verify(&t())
                .await
                .expect("chain must verify again once the row is restored");
        });
    }
}
