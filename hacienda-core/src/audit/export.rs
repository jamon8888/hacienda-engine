//! Serialising the audit record for hand-off.
//!
//! Two artefacts live here, and mistaking one for the other is the failure this module is
//! shaped to prevent:
//!
//! |  | evidence envelope — [`export_store`] with [`ExportFormat::Json`] / [`ExportFormat::JsonLines`] | tabular extract — [`export_csv`], [`export_store_csv`] |
//! | --- | --- | --- |
//! | Reader | a regulator, an external auditor | an analyst, a spreadsheet, a SIEM |
//! | Guarantee | re-verifiable offline, seals included | readable, greppable, importable |
//!
//! A flat list of entries cannot be verified offline whatever columns it carries. Every
//! segment's entry chain restarts at [`GENESIS_HASH`] with the sequence number back at
//! zero, and nothing in a table says where those restarts fall, so a verifier can recover
//! neither `seq` nor the predecessor hash. The envelope keeps entries grouped by segment
//! and carries the seals alongside them; that grouping is not presentation, it is what
//! makes verification possible at all.
//!
//! [`GENESIS_HASH`]: crate::audit::GENESIS_HASH

use serde::{Deserialize, Serialize};

use crate::audit::chain::AuditChain;
use crate::audit::cursor::AuditCursor;
use crate::audit::entry::AuditEntry;
use crate::audit::error::AuditError;
use crate::audit::segment::SegmentSeal;
use crate::audit::store::AuditStore;

/// How many entries [`export_store`] pulls per [`AuditStore::history`] call.
///
/// The whole export is materialised in memory anyway (the signature returns `Vec<u8>`), so
/// this is not about bounding memory — it is about not asking a file backend to read every
/// sealed segment in one call while it holds nothing back for the runtime.
const EXPORT_PAGE_SIZE: usize = 1024;

/// Envelope format version, present in every envelope so a verifier written against
/// version 1 can refuse a version it does not understand rather than mis-read it.
pub const EVIDENCE_ENVELOPE_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportFormat {
    /// One JSON record per line: the [`EvidenceHeader`], then one [`EvidenceSegment`].
    /// An evidence envelope — offline-verifiable.
    JsonLines,
    /// A single pretty-printed [`EvidenceEnvelope`]. An evidence envelope —
    /// offline-verifiable.
    Json,
    /// A flat CSV table of entries.
    ///
    /// **This is a tabular extract, not an evidence envelope, and it is not
    /// offline-verifiable.** It has no segment boundaries, so a reader cannot know the
    /// sequence number of any row nor where the chain restarts at
    /// [`GENESIS_HASH`](crate::audit::GENESIS_HASH) — and without those two,
    /// [`compute_chain_hash`](crate::audit::compute_chain_hash) cannot be re-run on a
    /// single row. Adding columns would not fix it: the seals are missing too, so nothing
    /// ties one segment to the next.
    ///
    /// Use it for analysis. For anything handed to a regulator or an external auditor, use
    /// [`Json`](Self::Json) or [`JsonLines`](Self::JsonLines) via [`export_store`].
    Csv,
}

// ── The evidence envelope ─────────────────────────────────────────────────────

/// Everything an offline verifier needs that is not an entry: the format version, the
/// chain head observed at export time, and the seal chain.
///
/// Kept as its own type because JSONL emits it as the first line while JSON flattens it
/// into the envelope object — one definition, two framings, no chance of the two drifting.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvidenceHeader {
    /// Always [`EVIDENCE_ENVELOPE_VERSION`] on export.
    pub version: u32,
    /// The store's chain head when the export was taken.
    ///
    /// Unlike the entries and the seals, nothing hashes this field — it is a claim by the
    /// exporter, useful because a client that recorded a tip alongside an earlier result
    /// can compare the two. It is not evidence on its own.
    pub tip: String,
    /// Every seal the store could see, oldest first. Pass the slice straight to
    /// [`verify_seal_chain`](crate::audit::verify_seal_chain).
    pub seals: Vec<SegmentSeal>,
}

/// One segment's entries, with the sequence number the first of them occupies.
///
/// `first_seq` is what makes the group verifiable: it is the `seq` argument
/// [`compute_chain_hash`](crate::audit::compute_chain_hash) needs for the first entry, and
/// every later entry in the group follows at `first_seq + 1`, `+ 2`, and so on. A whole
/// export starts each group at zero; it is recorded rather than assumed so that a partial
/// export stays verifiable.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvidenceSegment {
    pub segment_id: String,
    pub first_seq: u64,
    pub entries: Vec<AuditEntry>,
}

/// An offline-verifiable record of an audit store's history.
///
/// # How to verify one without the server
///
/// 1. [`verify_seal_chain`](crate::audit::verify_seal_chain) over [`EvidenceHeader::seals`]
///    — this is what makes deletion of a whole segment detectable.
/// 2. For each segment in [`segments`](Self::segments), replay the entry chain from
///    [`GENESIS_HASH`](crate::audit::GENESIS_HASH): the hash of the entry at position `i`
///    must equal `compute_chain_hash(prev, first_seq + i, entry.chain_hash_fields())`.
/// 3. For each seal, the group with the same `segment_id` must hold exactly
///    `entry_count` entries and end on `sealed_tip`.
///
/// # Segments with no entries are omitted
///
/// A segment sealed while empty — the state right after a rotation that recorded nothing —
/// has a seal but no group. A verifier joins groups to seals by `segment_id` and treats a
/// missing group as zero entries, which its seal's `entry_count` confirms.
///
/// # This node's history
///
/// Segments are per-writer and there is no total order between writers, so an envelope
/// records what *this* store holds. Presenting it as a deployment-wide history would
/// repeat, one level up, the mistake that [`AuditStore::history`] exists to close.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvidenceEnvelope {
    #[serde(flatten)]
    pub header: EvidenceHeader,
    /// The history grouped by segment, oldest segment first, entries in chain order.
    pub segments: Vec<EvidenceSegment>,
}

impl EvidenceEnvelope {
    /// Every entry in the envelope, oldest first, with the segment grouping dropped.
    ///
    /// Only for consumers that have already given up on verification — the tabular
    /// extract, or a reader that just wants to count rows.
    pub fn flattened(&self) -> impl Iterator<Item = &AuditEntry> + '_ {
        self.segments.iter().flat_map(|segment| &segment.entries)
    }
}

// ── Exporting from a store ────────────────────────────────────────────────────

/// Export everything an [`AuditStore`] holds.
///
/// # Why a free function and not a method on [`AuditStore`]
///
/// [`ExportFormat`] does not exist on wasm32, where this whole module is compiled out and
/// [`IndexedDbAuditStore`](crate::audit::IndexedDbAuditStore) lives. A trait method taking
/// one would not compile there, and the trait must stay implementable by every backend.
///
/// # Why this does not go through [`AuditChain`]
///
/// It cannot. Each segment's chain restarts at [`GENESIS_HASH`](crate::audit::GENESIS_HASH)
/// and [`AuditChain::append`] rejects any entry that does not extend the current tip, so a
/// single chain cannot hold a multi-segment history — the attempt fails at the first
/// rotation. The envelope is built from [`AuditStore::history`] and [`AuditStore::seals`]
/// instead.
///
/// # Formats
///
/// [`ExportFormat::Json`] and [`ExportFormat::JsonLines`] produce an
/// [`EvidenceEnvelope`], which verifies offline. [`ExportFormat::Csv`] produces a flat
/// table which **does not** — see [`export_store_csv`]. Callers that surface this choice
/// to a user must surface that difference with it.
///
/// # Errors
///
/// Propagates whatever the store reports while reading its history, and
/// [`AuditError::Json`] if a record cannot be serialised.
pub async fn export_store(
    store: &dyn AuditStore,
    format: ExportFormat,
) -> Result<Vec<u8>, AuditError> {
    match format {
        ExportFormat::Json => {
            let envelope = evidence_envelope(store).await?;
            Ok(serde_json::to_vec_pretty(&envelope)?)
        }
        ExportFormat::JsonLines => {
            let envelope = evidence_envelope(store).await?;
            write_evidence_json_lines(&envelope)
        }
        ExportFormat::Csv => export_store_csv(store).await,
    }
}

/// Read a store's whole history into an [`EvidenceEnvelope`].
///
/// # Read order, and what a concurrent writer can do to the result
///
/// Seals are read first, then the history, then the tip. That order matters. Reading the
/// seals *after* the entries would let a rotation that happened mid-export contribute a
/// seal whose `entry_count` covers entries appended after the history was read — and an
/// offline verifier would report that shortfall as tampering. Reading them first can only
/// leave a trailing group without a seal, which is indistinguishable from the ordinary
/// case of an open segment and raises no alarm. A false "this evidence was tampered with"
/// is far more expensive than a segment that has yet to be sealed.
///
/// # Errors
///
/// Propagates read errors from the store, including
/// [`AuditError::SegmentEntryCount`] when a sealed segment on disk disagrees with its seal.
pub async fn evidence_envelope(store: &dyn AuditStore) -> Result<EvidenceEnvelope, AuditError> {
    let seals = store.seals().await?;

    let mut segments: Vec<EvidenceSegment> = Vec::new();
    let mut cursor: Option<AuditCursor> = None;

    loop {
        let page = attributed_page(store, cursor.as_ref(), EXPORT_PAGE_SIZE).await?;
        let Some(last) = page.last() else {
            break;
        };

        cursor = Some(AuditCursor {
            segment_id: last.segment_id.clone(),
            index: last.first_seq + last.entries.len() as u64 - 1,
        });

        for group in page {
            match segments.last_mut() {
                Some(open) if open.segment_id == group.segment_id => {
                    open.entries.extend(group.entries);
                }
                _ => segments.push(group),
            }
        }
    }

    let tip = store.tip().await?;

    Ok(EvidenceEnvelope {
        header: EvidenceHeader {
            version: EVIDENCE_ENVELOPE_VERSION,
            tip,
            seals,
        },
        segments,
    })
}

/// One page of history, split into runs and each run attributed to its segment.
///
/// # Why this needs more than one call per page
///
/// [`AuditPage`](crate::audit::AuditPage) names only the position of its *last* entry, and
/// a page may straddle a rotation. But the position it does name is enough: the last entry
/// sits at `next.index` of `next.segment_id`, and a segment's indices are contiguous, so
/// the final `min(page_len, next.index + 1)` entries of the page all belong to that
/// segment. Whatever is left over came from earlier segments, and re-requesting exactly
/// that many entries from the same cursor names the segment they came from in turn. Each
/// pass strictly shrinks, so the loop terminates — and in the common case, where the page
/// does not straddle anything, it runs once.
///
/// Deriving the grouping from cursors rather than from seal `entry_count`s keeps it correct
/// when a rotation lands mid-export: a newly sealed segment simply shows up as its own run.
async fn attributed_page(
    store: &dyn AuditStore,
    after: Option<&AuditCursor>,
    limit: usize,
) -> Result<Vec<EvidenceSegment>, AuditError> {
    let mut runs: Vec<EvidenceSegment> = Vec::new();
    let mut remaining = limit;

    while remaining > 0 {
        let page = store.history(after, remaining).await?;
        let count = page.entries.len() as u64;
        if count == 0 {
            break;
        }

        // A non-empty page always carries the position of its last entry; a backend that
        // returns entries without one has broken the contract, and guessing a position
        // here would attribute entries to the wrong segment — which is precisely the
        // corruption the envelope exists to make detectable.
        let next = page.next.ok_or_else(|| {
            AuditError::Backend(format!(
                "history returned {count} entries with no cursor, so they cannot be \
                 attributed to a segment"
            ))
        })?;

        let in_segment = count.min(next.index + 1);
        let head = (count - in_segment) as usize;

        let mut entries = page.entries;
        runs.push(EvidenceSegment {
            segment_id: next.segment_id,
            first_seq: next.index + 1 - in_segment,
            entries: entries.split_off(head),
        });

        remaining = head;
    }

    // Built newest run first, because each pass resolves the tail of what is left.
    runs.reverse();
    Ok(runs)
}

/// One JSON record per line: the [`EvidenceHeader`] first, then one [`EvidenceSegment`]
/// per line.
///
/// Line-per-segment rather than line-per-entry: a line that carried a bare entry would
/// have lost the grouping that makes the entry verifiable, which would leave a JSONL
/// envelope weaker than the JSON one under the same name.
///
/// # Errors
///
/// [`AuditError::Json`] if a record cannot be serialised.
pub fn write_evidence_json_lines(envelope: &EvidenceEnvelope) -> Result<Vec<u8>, AuditError> {
    let mut output = Vec::new();

    serde_json::to_writer(&mut output, &envelope.header)?;
    output.push(b'\n');

    for segment in &envelope.segments {
        serde_json::to_writer(&mut output, segment)?;
        output.push(b'\n');
    }

    Ok(output)
}

/// A store's whole history as a flat CSV table.
///
/// **Not an evidence envelope.** The rows carry no segment boundary and no sequence
/// number, so no row's `chain_hash` can be recomputed, and no seal accompanies them, so
/// nothing links one segment to the next. It is offered because a spreadsheet or a SIEM is
/// a legitimate destination for an audit extract — not because it proves anything. Use
/// [`export_store`] with JSON or JSONL for that.
///
/// # Errors
///
/// Propagates read errors from the store, and [`AuditError::Json`] if an action cannot be
/// serialised.
pub async fn export_store_csv(store: &dyn AuditStore) -> Result<Vec<u8>, AuditError> {
    let envelope = evidence_envelope(store).await?;
    let entries: Vec<&AuditEntry> = envelope.flattened().collect();
    write_csv(entries.into_iter())
}

// ── Exporting a single chain ──────────────────────────────────────────────────

/// Export one [`AuditChain`] in `format`.
///
/// A chain is a single segment, so this cannot represent a store's history across
/// rotations — [`export_store`] is the entry point for that.
///
/// # Errors
///
/// Returns [`AuditError::Json`] if an entry cannot be serialised.
pub fn export(chain: &AuditChain, format: ExportFormat) -> Result<Vec<u8>, AuditError> {
    match format {
        ExportFormat::JsonLines => export_json_lines(chain),
        ExportFormat::Json => export_json(chain),
        ExportFormat::Csv => export_csv(chain),
    }
}

/// One JSON object per line, in chain order.
pub fn export_json_lines(chain: &AuditChain) -> Result<Vec<u8>, AuditError> {
    let mut output = Vec::new();
    for entry in chain.entries() {
        serde_json::to_writer(&mut output, entry)?;
        output.push(b'\n');
    }
    Ok(output)
}

/// A single pretty-printed JSON array.
pub fn export_json(chain: &AuditChain) -> Result<Vec<u8>, AuditError> {
    Ok(serde_json::to_vec_pretty(chain.entries())?)
}

/// RFC 4180 CSV with a header row — a tabular extract, **not** an evidence envelope.
///
/// The rows are not offline-verifiable and cannot be made so: a table has no segment
/// boundaries, so a reader knows neither an entry's sequence number nor where the chain
/// restarts at [`GENESIS_HASH`](crate::audit::GENESIS_HASH), and
/// [`compute_chain_hash`](crate::audit::compute_chain_hash) needs both. The seals are
/// absent too. Hand a regulator [`export_store`] with JSON or JSONL instead.
///
/// # Errors
///
/// Returns [`AuditError::Json`] if an entry's action cannot be serialised.
pub fn export_csv(chain: &AuditChain) -> Result<Vec<u8>, AuditError> {
    write_csv(chain.entries().iter())
}

/// The one CSV writer, shared by the chain and store extracts so the two cannot grow
/// different column sets.
///
/// `principal` is a column because "who revealed this value" is the question the extract
/// exists to answer. Dropping it left every row attributable to nobody while the
/// `chain_hash` beside it silently covered the field — an extract that answered a
/// different, weaker question than it appeared to.
fn write_csv<'a>(entries: impl Iterator<Item = &'a AuditEntry>) -> Result<Vec<u8>, AuditError> {
    let mut output = Vec::new();

    output.extend_from_slice(
        b"id,timestamp,category,action,span_hash,span_length,confidence,source,pipeline_version,config_hash,principal,chain_hash\n",
    );

    for entry in entries {
        let action = serde_json::to_string(&entry.action)?;
        let confidence = entry.confidence.map(|c| c.to_string()).unwrap_or_default();
        let span_length = entry.span_length.to_string();
        let source = entry.source.to_string();
        // An absent principal is an empty cell, matching how `compute_chain_hash` treats
        // it: a sentinel such as "none" would be indistinguishable from a principal of
        // that name.
        let principal = entry.principal.as_deref().unwrap_or_default();

        write_csv_row(
            &mut output,
            &[
                &entry.id,
                &entry.timestamp,
                &entry.category,
                &action,
                &entry.span_hash,
                &span_length,
                &confidence,
                &source,
                &entry.pipeline_version,
                &entry.config_hash,
                principal,
                &entry.chain_hash,
            ],
        );
    }

    Ok(output)
}

fn write_csv_row(output: &mut Vec<u8>, fields: &[&str]) {
    for (i, field) in fields.iter().enumerate() {
        if i > 0 {
            output.push(b',');
        }
        if field.contains([',', '"', '\n']) {
            output.push(b'"');
            for b in field.bytes() {
                if b == b'"' {
                    output.push(b'"');
                }
                output.push(b);
            }
            output.push(b'"');
        } else {
            output.extend_from_slice(field.as_bytes());
        }
    }
    output.push(b'\n');
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audit::entry::{compute_chain_hash, AuditEntryInput, EntitySource, RedactionAction};
    use crate::audit::segment::verify_seal_chain;
    use crate::audit::store::InMemoryAuditStore;
    use crate::audit::GENESIS_HASH;

    fn input(id: &str) -> AuditEntryInput {
        AuditEntryInput {
            id: id.into(),
            category: "Email".into(),
            action: RedactionAction::Mask,
            span_hash: "abc".into(),
            span_length: 10,
            confidence: Some(0.9),
            source: EntitySource::Regex,
            pipeline_version: "1.0".into(),
            config_hash: String::new(),
            principal: None,
        }
    }

    fn chain_with(count: usize) -> AuditChain {
        let mut chain = AuditChain::new("cfg");
        for i in 0..count {
            chain.push(input(&format!("id-{i}")));
        }
        chain
    }

    /// Three segments: two sealed by rotation, one still open.
    async fn store_with_two_rotations() -> InMemoryAuditStore {
        let store = InMemoryAuditStore::new("cfg");

        for batch in 0..3 {
            let inputs: Vec<AuditEntryInput> = (0..3)
                .map(|i| AuditEntryInput {
                    principal: Some(format!("avocat-{batch}")),
                    ..input(&format!("id-{batch}-{i}"))
                })
                .collect();
            store.append(inputs).await.expect("append");
            if batch < 2 {
                store.rotate().await.expect("rotate");
            }
        }

        store
    }

    /// The offline verifier: everything a regulator can do with the bytes and nothing
    /// else. It never touches the store.
    fn verify_offline(envelope: &EvidenceEnvelope) -> Result<(), AuditError> {
        verify_seal_chain(&envelope.header.seals)?;

        for segment in &envelope.segments {
            let mut prev = GENESIS_HASH.to_owned();
            for (offset, entry) in segment.entries.iter().enumerate() {
                let seq = segment.first_seq + offset as u64;
                let expected = compute_chain_hash(&prev, seq, entry.chain_hash_fields());
                if entry.chain_hash != expected {
                    return Err(AuditError::ChainIntegrity {
                        index: seq,
                        expected,
                        actual: entry.chain_hash.clone(),
                    });
                }
                prev = entry.chain_hash.clone();
            }
        }

        for seal in &envelope.header.seals {
            // A segment sealed while empty has no group; its seal records that.
            let entries = envelope
                .segments
                .iter()
                .find(|segment| segment.segment_id == seal.segment_id)
                .map(|segment| segment.entries.as_slice())
                .unwrap_or_default();

            if entries.len() as u64 != seal.entry_count {
                return Err(AuditError::SegmentEntryCount {
                    segment_id: seal.segment_id.clone(),
                    expected: seal.entry_count,
                    actual: entries.len() as u64,
                });
            }

            let sealed_tip = entries
                .last()
                .map(|entry| entry.chain_hash.clone())
                .unwrap_or_else(|| GENESIS_HASH.to_owned());

            if sealed_tip != seal.sealed_tip {
                return Err(AuditError::ChainIntegrity {
                    index: seal.entry_count,
                    expected: seal.sealed_tip.clone(),
                    actual: sealed_tip,
                });
            }
        }

        Ok(())
    }

    fn parse_json_lines(bytes: &[u8]) -> EvidenceEnvelope {
        let text = String::from_utf8(bytes.to_vec()).expect("utf-8");
        let mut lines = text.lines();
        let header: EvidenceHeader =
            serde_json::from_str(lines.next().expect("a header line")).expect("header parses");
        let segments = lines
            .map(|line| serde_json::from_str(line).expect("segment parses"))
            .collect();
        EvidenceEnvelope { header, segments }
    }

    // ── The evidence envelope ─────────────────────────────────────────────────

    /// The test the envelope exists to pass: two rotations, then the whole chain
    /// re-verified from the exported bytes alone — per-segment entry hashes recomputed
    /// with `compute_chain_hash`, and the seal chain checked with `verify_seal_chain`.
    /// Nothing here reads the store.
    #[tokio::test]
    async fn should_verify_an_export_offline_after_two_rotations() {
        let store = store_with_two_rotations().await;
        let bytes = export_store(&store, ExportFormat::Json)
            .await
            .expect("export");

        let envelope: EvidenceEnvelope = serde_json::from_slice(&bytes).expect("envelope parses");

        assert_eq!(envelope.header.version, EVIDENCE_ENVELOPE_VERSION);
        assert_eq!(envelope.header.seals.len(), 2, "two rotations, two seals");
        assert_eq!(envelope.segments.len(), 3, "two sealed plus the open one");
        assert_eq!(envelope.flattened().count(), 9);

        verify_offline(&envelope).expect("the export verifies offline");
    }

    /// The same guarantee through the line-oriented format, parsed the way a streaming
    /// verifier would: header line, then one segment per line.
    #[tokio::test]
    async fn should_verify_a_json_lines_export_offline_after_two_rotations() {
        let store = store_with_two_rotations().await;
        let bytes = export_store(&store, ExportFormat::JsonLines)
            .await
            .expect("export");

        assert_eq!(
            bytes.iter().filter(|b| **b == b'\n').count(),
            4,
            "one header line plus one line per segment"
        );

        let envelope = parse_json_lines(&bytes);
        assert_eq!(envelope.segments.len(), 3);
        verify_offline(&envelope).expect("the export verifies offline");
    }

    /// Without this the offline check could pass vacuously. Editing a field the chain hash
    /// covers must be caught by the bytes alone, with the segment's own sequence number.
    #[tokio::test]
    async fn should_reject_an_envelope_whose_entry_was_edited_after_export() {
        let store = store_with_two_rotations().await;
        let bytes = export_store(&store, ExportFormat::Json)
            .await
            .expect("export");
        let mut envelope: EvidenceEnvelope =
            serde_json::from_slice(&bytes).expect("envelope parses");

        envelope.segments[1].entries[1].category = "CreditCard".into();

        match verify_offline(&envelope) {
            Err(AuditError::ChainIntegrity { index, .. }) => assert_eq!(index, 1),
            other => panic!("expected a chain integrity failure at index 1, got {other:?}"),
        }
    }

    /// Dropping a whole segment is the attack the seals exist to catch, and it must be
    /// caught from the export rather than only at the store.
    #[tokio::test]
    async fn should_reject_an_envelope_whose_first_seal_was_removed() {
        let store = store_with_two_rotations().await;
        let bytes = export_store(&store, ExportFormat::Json)
            .await
            .expect("export");
        let mut envelope: EvidenceEnvelope =
            serde_json::from_slice(&bytes).expect("envelope parses");

        envelope.header.seals.remove(0);
        envelope.segments.remove(0);

        assert!(
            matches!(
                verify_offline(&envelope),
                Err(AuditError::SegmentLink { .. })
            ),
            "a truncated seal run must not verify"
        );
    }

    /// The grouping is the load-bearing part: entries must land under the segment that
    /// actually holds them, with a sequence number that restarts per segment.
    #[tokio::test]
    async fn should_group_exported_entries_by_segment_across_rotations() {
        let store = store_with_two_rotations().await;
        let envelope = evidence_envelope(&store).await.expect("envelope");

        for segment in &envelope.segments {
            assert_eq!(segment.entries.len(), 3);
            assert_eq!(
                segment.first_seq, 0,
                "each segment's chain restarts at zero"
            );
        }

        let seal_ids: Vec<&str> = envelope
            .header
            .seals
            .iter()
            .map(|seal| seal.segment_id.as_str())
            .collect();
        let group_ids: Vec<&str> = envelope
            .segments
            .iter()
            .map(|segment| segment.segment_id.as_str())
            .collect();
        assert_eq!(
            &group_ids[..2],
            &seal_ids[..],
            "sealed groups come first, in seal order"
        );
        assert_ne!(
            group_ids[2], group_ids[1],
            "the open segment is its own group"
        );
    }

    /// A page boundary that falls inside a segment must not invent a group boundary. Nine
    /// entries over three segments paged 1024 at a time never straddles anything, so this
    /// forces the crossing case by making every segment longer than nothing and checking
    /// the run-splitting arithmetic against a page smaller than the history.
    #[tokio::test]
    async fn should_attribute_a_page_that_straddles_a_rotation() {
        let store = store_with_two_rotations().await;

        // Four entries starting from the beginning: three from segment one, one from
        // segment two.
        let runs = attributed_page(&store, None, 4).await.expect("page");

        assert_eq!(runs.len(), 2, "the page straddles one rotation");
        assert_eq!(runs[0].entries.len(), 3);
        assert_eq!(runs[0].first_seq, 0);
        assert_eq!(runs[1].entries.len(), 1);
        assert_eq!(
            runs[1].first_seq, 0,
            "the successor segment restarts at zero"
        );
        assert_ne!(runs[0].segment_id, runs[1].segment_id);
    }

    /// An export taken while nothing else is writing must end on the head the store
    /// reports, or the tip in the header is a claim about a different chain state.
    #[tokio::test]
    async fn should_carry_the_current_chain_head_in_the_header() {
        let store = store_with_two_rotations().await;
        let envelope = evidence_envelope(&store).await.expect("envelope");

        let last = envelope
            .segments
            .last()
            .and_then(|segment| segment.entries.last())
            .expect("a last entry");
        assert_eq!(envelope.header.tip, last.chain_hash);
    }

    /// A store that has recorded nothing is a legitimate state, not an error, and its
    /// export must still parse and verify.
    #[tokio::test]
    async fn should_export_an_empty_store_as_an_envelope_with_no_segments() {
        let store = InMemoryAuditStore::new("cfg");
        let bytes = export_store(&store, ExportFormat::Json)
            .await
            .expect("export");
        let envelope: EvidenceEnvelope = serde_json::from_slice(&bytes).expect("envelope parses");

        assert!(envelope.segments.is_empty());
        assert!(envelope.header.seals.is_empty());
        assert_eq!(envelope.header.tip, GENESIS_HASH);
        verify_offline(&envelope).expect("an empty history verifies");
    }

    /// A segment sealed with nothing in it has a seal and no group. The verifier resolves
    /// that pairing by `segment_id`, so the case has to be exercised or the omission
    /// would only surface in front of an auditor.
    #[tokio::test]
    async fn should_verify_an_export_containing_a_segment_that_was_sealed_empty() {
        let store = InMemoryAuditStore::new("cfg");
        store.rotate().await.expect("seal an empty segment");
        store.append(vec![input("id-0")]).await.expect("append");

        let envelope = evidence_envelope(&store).await.expect("envelope");

        assert_eq!(envelope.header.seals.len(), 1);
        assert_eq!(envelope.header.seals[0].entry_count, 0);
        assert_eq!(envelope.segments.len(), 1, "the empty segment has no group");
        verify_offline(&envelope).expect("the export verifies offline");
    }

    /// The store's own closing seal must be inside the export; `entries()` goes empty
    /// after `close`, and an export that inherited that would hand over a truncated
    /// history.
    #[tokio::test]
    async fn should_export_the_final_segment_after_the_store_is_closed() {
        let store = store_with_two_rotations().await;
        store.close().await.expect("close");

        let envelope = evidence_envelope(&store).await.expect("envelope");

        assert_eq!(envelope.header.seals.len(), 3);
        assert_eq!(envelope.flattened().count(), 9);
        verify_offline(&envelope).expect("the export verifies offline");
    }

    // ── The tabular extract ───────────────────────────────────────────────────

    /// "Who revealed this value" is the question the extract exists to answer, so the
    /// column has to be there and has to carry the value the chain hash covers.
    #[tokio::test]
    async fn should_include_the_principal_column_in_a_store_csv_extract() {
        let store = store_with_two_rotations().await;
        let bytes = export_store(&store, ExportFormat::Csv)
            .await
            .expect("export");
        let text = String::from_utf8(bytes).expect("utf-8");

        let mut lines = text.lines();
        let header: Vec<&str> = lines.next().expect("a header row").split(',').collect();
        let principal_column = header
            .iter()
            .position(|column| *column == "principal")
            .expect("a principal column");
        assert_eq!(
            header[principal_column + 1],
            "chain_hash",
            "principal sits beside the hash that covers it"
        );

        let rows: Vec<Vec<&str>> = lines.map(|line| line.split(',').collect()).collect();
        assert_eq!(rows.len(), 9, "every segment's entries, flattened");
        assert_eq!(rows[0][principal_column], "avocat-0");
        assert_eq!(rows[8][principal_column], "avocat-2");
    }

    #[test]
    fn should_write_an_empty_cell_for_an_absent_principal() {
        let bytes = export_csv(&chain_with(1)).expect("csv");
        let text = String::from_utf8(bytes).expect("utf-8");
        let header: Vec<&str> = text.lines().next().expect("header").split(',').collect();
        let row: Vec<&str> = text.lines().nth(1).expect("row").split(',').collect();
        let principal_column = header
            .iter()
            .position(|column| *column == "principal")
            .expect("a principal column");
        assert_eq!(row[principal_column], "");
    }

    /// Every column must round-trip, not just the new one: a row whose fields shifted by
    /// one would still have the right count.
    #[test]
    fn should_round_trip_every_entry_field_through_the_csv() {
        let mut chain = AuditChain::new("cfg");
        chain.push(AuditEntryInput {
            principal: Some("avocat-7".into()),
            ..input("id-0")
        });
        let entry = chain.entries()[0].clone();

        let bytes = export_csv(&chain).expect("csv");
        let text = String::from_utf8(bytes).expect("utf-8");
        let header: Vec<&str> = text.lines().next().expect("header").split(',').collect();
        let row: Vec<&str> = text.lines().nth(1).expect("row").split(',').collect();
        // RFC 4180 unquoting. `action` is a JSON string, so its own quotes are doubled on
        // the way out; a test that asserted the escaped form would be pinning the escape
        // rather than the value.
        let cell = |name: &str| {
            let index = header
                .iter()
                .position(|column| *column == name)
                .unwrap_or_else(|| panic!("a {name} column"));
            match row[index]
                .strip_prefix('"')
                .and_then(|f| f.strip_suffix('"'))
            {
                Some(inner) => inner.replace("\"\"", "\""),
                None => row[index].to_owned(),
            }
        };

        assert_eq!(header.len(), row.len());
        assert_eq!(cell("id"), entry.id);
        assert_eq!(cell("timestamp"), entry.timestamp);
        assert_eq!(cell("category"), entry.category);
        assert_eq!(cell("action"), "\"mask\"");
        assert_eq!(cell("span_hash"), entry.span_hash);
        assert_eq!(cell("span_length"), entry.span_length.to_string());
        assert_eq!(cell("confidence"), "0.9");
        assert_eq!(cell("source"), "regex");
        assert_eq!(cell("pipeline_version"), entry.pipeline_version);
        assert_eq!(cell("config_hash"), entry.config_hash);
        assert_eq!(cell("principal"), "avocat-7");
        assert_eq!(cell("chain_hash"), entry.chain_hash);
    }

    // ── Single-chain export ───────────────────────────────────────────────────

    #[test]
    fn should_write_one_json_object_per_line() {
        let bytes = export_json_lines(&chain_with(3)).unwrap();
        let text = String::from_utf8(bytes).unwrap();
        let lines: Vec<_> = text.lines().collect();
        assert_eq!(lines.len(), 3);
        for line in lines {
            let value: serde_json::Value = serde_json::from_str(line).unwrap();
            assert!(value.get("chain_hash").is_some());
        }
    }

    #[test]
    fn should_write_a_json_array_of_every_entry() {
        let bytes = export_json(&chain_with(2)).unwrap();
        let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(value.as_array().unwrap().len(), 2);
    }

    #[test]
    fn should_write_a_csv_header_plus_one_row_per_entry() {
        let bytes = export_csv(&chain_with(2)).unwrap();
        let text = String::from_utf8(bytes).unwrap();
        let lines: Vec<_> = text.lines().collect();
        assert_eq!(lines.len(), 3);
        assert!(lines[0].starts_with("id,timestamp,category"));
        assert!(lines[0].ends_with("config_hash,principal,chain_hash"));
    }

    #[test]
    fn should_quote_and_escape_csv_fields_containing_delimiters() {
        let mut output = Vec::new();
        write_csv_row(&mut output, &["plain", "has,comma", "has\"quote"]);
        assert_eq!(
            String::from_utf8(output).unwrap(),
            "plain,\"has,comma\",\"has\"\"quote\"\n"
        );
    }

    #[test]
    fn should_export_an_empty_chain_as_a_header_only_csv() {
        let bytes = export_csv(&chain_with(0)).unwrap();
        assert_eq!(String::from_utf8(bytes).unwrap().lines().count(), 1);
    }
}
