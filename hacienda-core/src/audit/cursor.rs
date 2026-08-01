//! Cursor-based positions into an audit history, and the paging helper the three
//! backends share.
//!
//! The chain only ever grows, so an offset drifts the moment an entry is appended
//! mid-pagination: page two would re-serve an entry page one already returned, or skip
//! one entirely. An auditor who silently skipped a record is the exact failure the audit
//! module exists to prevent, so the position handed back to a caller is a *pair the chain
//! already commits to* — the segment, and the entry's sequence number inside it — not a
//! count of how many entries happened to precede it at read time.

use std::fmt;
use std::str::FromStr;

use crate::audit::entry::AuditEntry;
use crate::audit::error::AuditError;

// ── AuditCursor ───────────────────────────────────────────────────────────────

/// An immutable position in the chain: the entry at `index` inside segment `segment_id`.
///
/// # Why `(segment_id, index)` and not an entry id or an offset
///
/// `index` is the sequence number [`AuditChain::verify`](crate::audit::AuditChain::verify)
/// already uses when it recomputes an entry's hash — a value the chain commits to, not an
/// incidental array position. Segments never lose or reorder entries, so the pair names
/// the same record forever.
///
/// [`AuditEntry`] carries neither a segment id nor a sequence number, there is no index on
/// disk, and its `id` is a uuid v4 and therefore unsortable. Paginating by entry id would
/// mean rescanning the whole history on every page just to find where to resume;
/// `(segment_id, index)` resolves against the already-ordered seal list and one file open.
///
/// # Opacity
///
/// The wire form is `"{segment_id}:{index}"`, and callers must treat it as opaque: parse
/// it back with [`FromStr`], never construct one. It is not a stable public encoding, and
/// a hand-built cursor whose index has been nudged forwards would skip entries without
/// anything noticing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuditCursor {
    /// The segment holding the entry this cursor names.
    pub segment_id: String,
    /// The entry's 0-based sequence number within that segment.
    pub index: u64,
}

impl fmt::Display for AuditCursor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.segment_id, self.index)
    }
}

impl FromStr for AuditCursor {
    type Err = AuditError;

    /// Parse the `"{segment_id}:{index}"` wire form.
    ///
    /// Split at the **last** colon rather than the first: segment ids are uuid v4 today,
    /// but [`Segment::open_with_id`](crate::audit::Segment) accepts any string, and a
    /// parser that broke on an id containing a colon would fail long after the cursor was
    /// issued, in production, on somebody else's data.
    fn from_str(raw: &str) -> Result<Self, Self::Err> {
        let unresolvable = |reason: &str| AuditError::UnresolvableCursor {
            cursor: raw.to_owned(),
            reason: reason.to_owned(),
        };

        let (segment_id, index) = raw
            .rsplit_once(':')
            .ok_or_else(|| unresolvable("expected the form '{segment_id}:{index}'"))?;

        if segment_id.is_empty() {
            return Err(unresolvable("the segment identifier is empty"));
        }

        let index: u64 = index
            .parse()
            .map_err(|_| unresolvable("the index is not a non-negative integer"))?;

        Ok(Self {
            segment_id: segment_id.to_owned(),
            index,
        })
    }
}

// ── AuditPage ─────────────────────────────────────────────────────────────────

/// One page of audit history, oldest entry first.
#[derive(Debug, Clone)]
pub struct AuditPage {
    /// The entries on this page, in chain order.
    pub entries: Vec<AuditEntry>,
    /// Where the caller resumes: the position of the **last** entry on this page, or
    /// `None` when the page is empty.
    ///
    /// # Why `None` means "this page was empty", not "the history is finished"
    ///
    /// An append-only chain is never finished — it is only momentarily caught up. Had
    /// `next` gone `None` as soon as a page ran short of `limit`, a caller that had
    /// reached the end would be left holding no resumable position at all, and its only
    /// way back would be to re-read from the beginning and de-duplicate. So the rule is:
    /// a non-empty page always yields a cursor, and a caller that wants the whole history
    /// pages until it gets an empty one. A caller that wants to keep following the chain
    /// simply keeps the last cursor it was given and asks again later.
    pub next: Option<AuditCursor>,
}

// ── Shared paging ─────────────────────────────────────────────────────────────

/// Assemble one page from an ordered run of segments, oldest first.
///
/// `extents` is `(segment_id, entry_count)` for every segment in the history, sealed ones
/// first and the open one — if there is one — last. `fetch(position)` produces the entries
/// of `extents[position]`, and is only ever called for a segment the page actually needs;
/// that laziness is what lets [`FileAuditStore`](crate::audit::FileAuditStore) serve a
/// page without reading every sealed file on disk.
///
/// The three backends differ only in where their entries come from, so the ordering,
/// cursor arithmetic, and the count check below live here once rather than three times.
///
/// # Why `fetch` is a plain closure and not `async`
///
/// Every backend's entries are either already in memory or come from a synchronous file
/// read, and an `async` callback would force a boxed future through a trait method that
/// must stay object-safe. This keeps the whole helper a pure function of its inputs.
pub(crate) fn page_from(
    extents: &[(&str, u64)],
    after: Option<&AuditCursor>,
    limit: usize,
    mut fetch: impl FnMut(usize) -> Result<Vec<AuditEntry>, AuditError>,
) -> Result<AuditPage, AuditError> {
    let (mut position, mut skip) = resolve(extents, after)?;

    let mut entries: Vec<AuditEntry> = Vec::new();
    let mut next: Option<AuditCursor> = None;

    while position < extents.len() && entries.len() < limit {
        let (segment_id, recorded_count) = extents[position];

        // The cursor named the last entry of this segment: nothing left here, move on.
        if skip >= recorded_count {
            position += 1;
            skip = 0;
            continue;
        }

        let segment_entries = fetch(position)?;

        // A sealed segment whose file disagrees with the count its seal recorded is
        // tampering, and serving a page out of it would launder the discrepancy into a
        // response that looks authoritative. `verify` reports the same condition the same
        // way; a read path must not be more forgiving than the checker.
        if segment_entries.len() as u64 != recorded_count {
            return Err(AuditError::SegmentEntryCount {
                segment_id: segment_id.to_owned(),
                expected: recorded_count,
                actual: segment_entries.len() as u64,
            });
        }

        for (index, entry) in segment_entries.into_iter().enumerate().skip(skip as usize) {
            entries.push(entry);
            next = Some(AuditCursor {
                segment_id: segment_id.to_owned(),
                index: index as u64,
            });
            if entries.len() >= limit {
                break;
            }
        }

        position += 1;
        skip = 0;
    }

    Ok(AuditPage { entries, next })
}

/// Turn `after` into the `(segment position, entries to skip)` where the next page starts.
///
/// # Why an unresolvable cursor is an error
///
/// The tempting alternative — fall back to the start of the history — silently re-serves
/// every entry the caller already has, and a caller that trusted the cursor has no way to
/// tell the duplicate run from new activity. An audit reader that double-counts is worse
/// than one that stops and says so.
fn resolve(
    extents: &[(&str, u64)],
    after: Option<&AuditCursor>,
) -> Result<(usize, u64), AuditError> {
    let Some(cursor) = after else {
        return Ok((0, 0));
    };

    let position = extents
        .iter()
        .position(|(segment_id, _)| *segment_id == cursor.segment_id)
        .ok_or_else(|| AuditError::UnresolvableCursor {
            cursor: cursor.to_string(),
            reason: "this store holds no segment with that identifier".to_owned(),
        })?;

    let count = extents[position].1;
    if cursor.index >= count {
        return Err(AuditError::UnresolvableCursor {
            cursor: cursor.to_string(),
            reason: format!("that segment holds {count} entries, so it has no entry at that index"),
        });
    }

    // The page starts *after* the entry the cursor names.
    Ok((position, cursor.index + 1))
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// The wire form is what a client stores and hands back on the next request, so a
    /// value that does not survive the round trip would strand a paginating caller.
    #[test]
    fn should_round_trip_a_cursor_through_its_wire_form() {
        let cursor = AuditCursor {
            segment_id: "3f8b1c2e-0000-4000-8000-000000000001".into(),
            index: 42,
        };
        let parsed: AuditCursor = cursor.to_string().parse().expect("round trip");
        assert_eq!(parsed, cursor);
    }

    /// Segment ids are uuid v4 today but `Segment::open_with_id` takes any string, so
    /// splitting on the first colon would silently mis-parse a legitimate id later.
    #[test]
    fn should_split_a_segment_id_containing_a_colon_at_the_last_colon() {
        let cursor = AuditCursor {
            segment_id: "node:a:segment".into(),
            index: 7,
        };
        let parsed: AuditCursor = cursor.to_string().parse().expect("round trip");
        assert_eq!(parsed.segment_id, "node:a:segment");
        assert_eq!(parsed.index, 7);
    }

    #[test]
    fn should_reject_a_malformed_cursor_rather_than_guessing_a_position() {
        for raw in ["", "no-colon", ":4", "seg:", "seg:-1", "seg:not-a-number"] {
            let err = raw
                .parse::<AuditCursor>()
                .expect_err("a malformed cursor must not parse: {raw}");
            assert!(
                matches!(err, AuditError::UnresolvableCursor { .. }),
                "expected UnresolvableCursor for {raw:?}, got {err:?}"
            );
        }
    }
}
