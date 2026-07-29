"""Assemble accepted character-offset spans into GLiNER2's word-token training format.

GLiNER2 trains on word-token spans, not character offsets (spec §8):

    {"tokenized_text": [...], "ner": [[start_word_idx, end_word_idx, label], ...]}

A char-to-word remapping that drifts by one token has no error signal by default —
it just silently corrupts a slice of the training set. `to_word_span_record` asserts
the round trip internally (re-rendering the chosen word range must reproduce the
original character span) rather than trusting the index arithmetic, since that's
exactly the kind of drift a future refactor could reintroduce without failing loudly.
"""

import random
import re

_WORD_RE = re.compile(r"\S+")


def _words_with_offsets(text: str) -> list[tuple[str, int, int]]:
    return [(m.group(), m.start(), m.end()) for m in _WORD_RE.finditer(text)]


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
