"""Measure the two GLiNER2 behaviours that destroy training signal without raising.

Neither shows up as an error. They surface only as metrics that are worse than they
should be, at which point the natural — and wrong — response is to train longer.
Both are cheap to measure on CPU before a run, so they are measured here (spec
2026-08-15 §1.7, §3.3):

- **Width.** `model.py:594-604` sets a span label only when
  `0 <= width < scores.shape[3]`, where `width = end_word_idx - start_word_idx` and
  the bound is `max_width = 8`. An N-token mention has width `N - 1`, so mentions
  *over* `max_width` tokens are dropped with no error and can never be learned. A
  vertical whose key entity is routinely wider needs a different plan, not more
  epochs — so this reports the affected labels, not just a count.
- **Resolvability.** GLiNER2 resolves mentions by contiguous sublist search over
  lowercased tokens (`SchemaTransformer._find_sublist`). `sanitize()` drops the
  **entire entity type** for a record on a single miss (`data.py:756`, `:796`), so one
  malformed mention silently deletes a whole type's supervision for that record.

Width and resolvability *report*: they are corpus facts needing a human decision, and
raising would push callers toward silently filtering the corpus instead of looking at
it. Split leakage *raises*: it is always a bug.
"""

import re
from typing import NamedTuple

#: GLiNER2's span width cap, in tokens (`ExtractorConfig.max_width`).
MAX_WIDTH = 8

# Verbatim copy of GLiNER2's `WhitespaceTokenSplitter._PATTERN` (processor.py:231-238).
# Reimplementing this as `text.lower().split()` produces false positives: it leaves
# punctuation glued to the token ("inc." vs "inc"), so resolvable mentions are reported
# as missing and the whole preflight degrades into noise nobody reads. ~keep
_SPLIT = re.compile(
    r"""(?:https?://[^\s]+|www\.[^\s]+)
    |[a-z0-9._%+-]+@[a-z0-9.-]+\.[a-z]{2,}
    |@[a-z0-9_]+
    |\w+(?:[-_]\w+)*
    |\S""",
    re.VERBOSE | re.IGNORECASE,
)


class Finding(NamedTuple):
    """One (record, label, mention) triple that a check flagged."""

    record_index: int
    label: str
    mention: str


def gliner2_tokens(text: str) -> list[str]:
    """Tokenize exactly as GLiNER2 does before matching: its splitter, then lowercase."""
    return [m.group().lower() for m in _SPLIT.finditer(text)]


def _iter_mentions(records: list[dict]):
    for index, rec in enumerate(records):
        for label, mentions in rec["output"]["entities"].items():
            for mention in mentions:
                yield index, label, mention


def width_violations(records: list[dict], max_width: int = MAX_WIDTH) -> list[Finding]:
    """Mentions over `max_width` tokens, which are silently unlearnable.

    GLiNER2 computes `width = end_word_idx - start_word_idx`, so an N-token mention has
    width `N - 1`. The model's bound is `0 <= width < max_width`, so a mention of
    exactly `max_width` tokens (width `max_width - 1`) is still learnable — only
    mentions with *more* than `max_width` tokens are lost.
    """
    return [
        Finding(index, label, mention)
        for index, label, mention in _iter_mentions(records)
        if len(gliner2_tokens(mention)) > max_width
    ]


def unresolvable_mentions(records: list[dict]) -> list[Finding]:
    """Mentions GLiNER2's own matching cannot find in the record's text.

    Each one costs the whole entity type's supervision for that record, not just the
    one mention.
    """
    findings = []
    for index, rec in enumerate(records):
        haystack = gliner2_tokens(rec["input"])
        for label, mentions in rec["output"]["entities"].items():
            for mention in mentions:
                needle = gliner2_tokens(mention)
                found = needle and any(
                    haystack[start : start + len(needle)] == needle
                    for start in range(len(haystack) - len(needle) + 1)
                )
                if not found:
                    findings.append(Finding(index, label, mention))
    return findings


def assert_split_integrity(splits: dict[str, list[dict]]) -> None:
    """Raise if any `doc_id` appears in more than one split.

    Chunk-level leakage inflates held-out metrics without any visible symptom, and it
    is never a legitimate corpus property — unlike the width and resolvability checks,
    this one raises.
    """
    seen: dict[str, str] = {}
    leaked: list[str] = []
    for split_name, records in splits.items():
        doc_ids = set()
        for index, r in enumerate(records):
            if "doc_id" not in r:
                raise ValueError(
                    f"record {index} in split {split_name!r} has no doc_id; a record "
                    f"cannot be assigned to a split without one — check the emitter "
                    f"that produced this corpus"
                )
            doc_ids.add(r["doc_id"])
        for doc_id in doc_ids:
            if seen.setdefault(doc_id, split_name) != split_name:
                leaked.append(f"{doc_id!r} in both {seen[doc_id]!r} and {split_name!r}")

    if leaked:
        raise ValueError(
            f"document leakage across splits: {'; '.join(sorted(leaked))}. "
            f"Split by doc_id, never by chunk — held-out metrics are meaningless otherwise."
        )
