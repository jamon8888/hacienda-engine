"""Assemble accepted character-offset spans into GLiNER2's training format.

`to_gliner2_record` is the emitter the trainer consumes. GLiNER2 takes **verbatim
mention strings**, not spans, and resolves them to word spans internally via
`SchemaTransformer._find_sublist` over lowercased tokenized text:

    {"input": "...", "output": {"entities": {"party": ["Acme"], ...}}}

`ExtractorDataset._load_dict_list` raises `ValueError("Unknown dict format...")` on
anything else, which is what the older `to_word_span_record` emits — that is GLiNER
**v1**'s `{"tokenized_text", "ner"}` shape (spec 2026-08-15 §1.1). It is retained
only for its round-trip discipline, now re-pointed as a preflight in
`training/dataset_preflight.py`; nothing feeds the trainer from it.

Mentions are sliced straight out of `chunk_text` rather than rebuilt from tokens.
Rebuilding collapses runs of whitespace and drops punctuation, and GLiNER2's matcher
is verbatim — a mention it cannot find causes `sanitize()` to drop that entity type
for the whole record, with no error.
"""

import random
import re

_WORD_RE = re.compile(r"\S+")


def _words_with_offsets(text: str) -> list[tuple[str, int, int]]:
    return [(m.group(), m.start(), m.end()) for m in _WORD_RE.finditer(text)]


def to_gliner2_record(
    chunk_text: str,
    char_spans: list[tuple[int, int, str]],
    *,
    doc_id: str = "",
    source: str = "auto_labeled",
) -> dict:
    """Group accepted `(start, end, label)` spans into GLiNER2's `input`/`output` record.

    `doc_id` and `source` ride along unchanged so `train_val_test_split` keeps working;
    GLiNER2 ignores keys it does not recognise.
    """
    entities: dict[str, list[str]] = {}
    for start, end, label in char_spans:
        if not 0 <= start < end <= len(chunk_text):
            raise ValueError(
                f"span ({start}, {end}) is out of range for a {len(chunk_text)}-char chunk; "
                f"Python slicing would clamp it into a silently shorter mention"
            )
        mention = chunk_text[start:end]
        # Duplicates add no supervision: GLiNER2 resolves a mention to every
        # occurrence in the text, not to the one span it came from.
        mentions = entities.setdefault(label, [])
        if mention not in mentions:
            mentions.append(mention)

    return {
        "input": chunk_text,
        "output": {"entities": entities},
        "doc_id": doc_id,
        "source": source,
    }


def to_word_span_record(
    chunk_text: str,
    char_spans: list[tuple[int, int, str]],
    *,
    doc_id: str = "",
    source: str = "auto_labeled",
) -> dict:
    """Convert `(start_char, end_char, label)` spans into GLiNER2's word-span format."""
    words = _words_with_offsets(chunk_text)
    tokenized_text = [w for w, _, _ in words]

    ner = []
    for start_char, end_char, label in char_spans:
        overlapping = [
            i
            for i, (_, w_start, w_end) in enumerate(words)
            if w_start < end_char and w_end > start_char
        ]
        if not overlapping:
            raise ValueError(
                f"span ({start_char}, {end_char}) does not align to any word in: {chunk_text!r}"
            )
        start_word, end_word = overlapping[0], overlapping[-1]

        # Mandatory round-trip assertion (spec §8): a silent off-by-one here
        # corrupts training data with no other signal, so it is checked at
        # assembly time rather than assumed correct by construction.
        rendered = " ".join(tokenized_text[start_word : end_word + 1])
        original = chunk_text[start_char:end_char]
        if rendered.strip() != original.strip():
            raise ValueError(
                f"word-span assembly drift: rendered {rendered!r} != original {original!r}"
            )

        ner.append([start_word, end_word, label])

    return {
        "tokenized_text": tokenized_text,
        "ner": ner,
        "doc_id": doc_id,
        "source": source,
    }


def train_val_test_split(
    records: list[dict],
    seed: int,
    train_frac: float = 0.8,
    val_frac: float = 0.1,
) -> dict[str, list[dict]]:
    """Split records by `doc_id`, never by chunk, so no document leaks across splits.

    The test split is always the fully-human-reviewed slice (spec §8: "Test set is
    the human-reviewed slice only ... since that's the only slice with a trustworthy
    label"), not a random `train_frac`/`val_frac`/remainder split like train/val — a
    document only qualifies if every record from it is human-reviewed, so a
    partially-reviewed document's auto-labeled chunks can't leak into training while
    its reviewed chunks sit in test.
    """
    by_doc: dict[str, list[dict]] = {}
    for record in records:
        by_doc.setdefault(record["doc_id"], []).append(record)

    doc_ids = sorted(by_doc.keys())
    rng = random.Random(seed)
    rng.shuffle(doc_ids)

    human_doc_ids = [
        d for d in doc_ids if all(r["source"] == "human_reviewed" for r in by_doc[d])
    ]
    remaining_doc_ids = [d for d in doc_ids if d not in human_doc_ids]

    n_train = int(len(remaining_doc_ids) * train_frac)
    n_val = int(len(remaining_doc_ids) * val_frac)

    train_docs = set(remaining_doc_ids[:n_train])
    val_docs = set(remaining_doc_ids[n_train : n_train + n_val])
    # Rounding leftovers join train rather than being silently dropped.
    train_docs |= set(remaining_doc_ids[n_train + n_val :])

    return {
        "train": [r for d in train_docs for r in by_doc[d]],
        "val": [r for d in val_docs for r in by_doc[d]],
        "test": [r for d in human_doc_ids for r in by_doc[d]],
    }
