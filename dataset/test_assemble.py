import itertools

from assemble import to_word_span_record, train_val_test_split

_doc_id_counter = itertools.count()


def make_record(doc_id: str | None = None, source: str = "auto_labeled") -> dict:
    if doc_id is None:
        doc_id = f"doc-{next(_doc_id_counter)}"
    return {"doc_id": doc_id, "source": source, "tokenized_text": ["placeholder"], "ner": []}


def test_word_span_round_trips_to_the_original_text():
    chunk = "The Licensee shall not assign this Agreement without consent."
    entity_text = "assign this Agreement"
    char_start = chunk.index(entity_text)
    char_end = char_start + len(entity_text)

    record = to_word_span_record(chunk, [(char_start, char_end, "assignment_clause")])

    start_w, end_w, label = record["ner"][0]
    rendered = " ".join(record["tokenized_text"][start_w : end_w + 1])
    assert rendered == chunk[char_start:char_end]
    assert label == "assignment_clause"


def test_a_single_word_span_resolves_to_the_same_start_and_end_index():
    chunk = "Force majeure excuses non-performance."
    char_start = chunk.index("Force")
    char_end = char_start + len("Force")

    record = to_word_span_record(chunk, [(char_start, char_end, "force_majeure")])

    start_w, end_w, _ = record["ner"][0]
    assert start_w == end_w == 0


def test_span_not_aligned_to_a_word_boundary_raises_rather_than_silently_corrupting():
    chunk = "The Licensee shall not assign this Agreement without consent."
    # Deliberately mid-word: "ssign" instead of "assign" — a genuine offset bug, not
    # a realistic input, since resolve_offset never returns a mid-word span.
    char_start = chunk.index("assign") + 1
    char_end = char_start + len("ssign")

    try:
        to_word_span_record(chunk, [(char_start, char_end, "assignment_clause")])
        raised = False
    except ValueError:
        raised = True

    assert raised, "a mid-word span must be rejected, not silently corrupted"


def test_split_is_by_document_not_by_chunk():
    doc_id = "doc-shared"
    records = [make_record(doc_id=doc_id), make_record(doc_id=doc_id)]

    split = train_val_test_split(records, seed=1)

    doc_ids_per_split = {k: {r["doc_id"] for r in v} for k, v in split.items()}
    assert doc_ids_per_split["train"].isdisjoint(doc_ids_per_split["val"])
    assert doc_ids_per_split["train"].isdisjoint(doc_ids_per_split["test"])
    # Both chunks from the shared document must land in the same split, not spread
    # across train/val/test independently.
    train_ids = {id(r) for r in split["train"]}
    for r in records:
        assert (id(r) in train_ids) == (
            all(id(other) in train_ids for other in records)
        )


def test_only_human_reviewed_records_land_in_test_split():
    records = [
        make_record(source="auto_labeled"),
        make_record(source="human_reviewed"),
    ]

    split = train_val_test_split(records, seed=1)

    assert len(split["test"]) > 0
    assert all(r["source"] == "human_reviewed" for r in split["test"])


def test_a_document_with_mixed_sources_does_not_qualify_for_the_test_split():
    doc_id = "doc-mixed"
    records = [
        make_record(doc_id=doc_id, source="auto_labeled"),
        make_record(doc_id=doc_id, source="human_reviewed"),
    ]

    split = train_val_test_split(records, seed=1)

    # A partially-reviewed document must not leak its reviewed chunk into test while
    # its auto-labeled chunk trains — the whole document goes to train/val instead.
    assert split["test"] == []
