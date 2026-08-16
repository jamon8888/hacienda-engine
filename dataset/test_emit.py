from assemble import to_gliner2_record, train_val_test_split


def test_record_uses_gliner2s_input_output_shape():
    chunk = "The Licensee shall not assign this Agreement without consent."
    start = chunk.index("Licensee")
    record = to_gliner2_record(chunk, [(start, start + len("Licensee"), "party")])

    assert record["input"] == chunk
    assert record["output"] == {"entities": {"party": ["Licensee"]}}


def test_mentions_are_sliced_from_the_chunk_not_rebuilt_from_tokens():
    # Rebuilding from whitespace tokens would collapse the double space and drop the
    # comma, producing a mention GLiNER2's verbatim matcher can never find.
    chunk = "Notice to  Globex, Inc. is required."
    entity = "Globex, Inc."
    start = chunk.index(entity)

    record = to_gliner2_record(chunk, [(start, start + len(entity), "party")])

    mention = record["output"]["entities"]["party"][0]
    assert mention == entity
    assert mention in chunk


def test_several_mentions_of_one_type_group_into_a_single_list():
    chunk = "Acme and Globex are parties; Acme indemnifies Globex."
    spans = [
        (chunk.index("Acme"), chunk.index("Acme") + 4, "party"),
        (chunk.index("Globex"), chunk.index("Globex") + 6, "party"),
    ]

    record = to_gliner2_record(chunk, spans)

    assert record["output"]["entities"] == {"party": ["Acme", "Globex"]}


def test_distinct_types_get_their_own_lists():
    chunk = "Acme shall pay $500 on demand."
    spans = [
        (chunk.index("Acme"), chunk.index("Acme") + 4, "party"),
        (chunk.index("$500"), chunk.index("$500") + 4, "amount"),
    ]

    record = to_gliner2_record(chunk, spans)

    assert record["output"]["entities"] == {"party": ["Acme"], "amount": ["$500"]}


def test_a_repeated_mention_string_is_emitted_once_per_type():
    # GLiNER2 resolves a mention to *every* occurrence in the text, so listing the
    # same string twice adds no supervision and skews nothing but the file size.
    chunk = "Acme notified Acme of the breach."
    spans = [
        (0, 4, "party"),
        (chunk.index("Acme", 5), chunk.index("Acme", 5) + 4, "party"),
    ]

    record = to_gliner2_record(chunk, spans)

    assert record["output"]["entities"]["party"] == ["Acme"]


def test_a_chunk_with_no_accepted_spans_still_emits_a_valid_record():
    # Negative chunks are legitimate training signal, and GLiNER2 accepts an empty
    # entity map — dropping them would bias the corpus toward entity-dense text.
    record = to_gliner2_record("Nothing of interest here.", [])

    assert record["input"] == "Nothing of interest here."
    assert record["output"] == {"entities": {}}


def test_split_metadata_survives_so_the_existing_splitter_still_works():
    record = to_gliner2_record("Acme.", [(0, 4, "party")], doc_id="d1", source="human_reviewed")

    split = train_val_test_split([record], seed=1)

    assert split["test"] == [record]


def test_an_out_of_range_span_is_rejected_rather_than_silently_truncated():
    chunk = "Acme."
    try:
        to_gliner2_record(chunk, [(0, 99, "party")])
        raised = False
    except ValueError:
        raised = True

    assert raised, "Python slicing clamps out-of-range ends, yielding a shorter mention"


def test_an_empty_span_is_rejected():
    # An empty mention string matches at every position under GLiNER2's sublist search.
    try:
        to_gliner2_record("Acme.", [(2, 2, "party")])
        raised = False
    except ValueError:
        raised = True

    assert raised


def test_a_whitespace_only_span_is_rejected():
    # A span covering only whitespace tokenizes to zero GLiNER2 tokens, which would
    # otherwise pass the bounds check and silently drop the whole entity type.
    chunk = "Acme  Corp."
    start = chunk.index("  ")
    try:
        to_gliner2_record(chunk, [(start, start + 2, "party")])
        raised = False
    except ValueError:
        raised = True

    assert raised


def test_the_record_survives_a_json_round_trip():
    import json

    chunk = 'He said "Acme" — twice.'
    start = chunk.index("Acme")
    record = to_gliner2_record(chunk, [(start, start + 4, "party")])

    restored = json.loads(json.dumps(record))

    assert restored == record
    for mentions in restored["output"]["entities"].values():
        for mention in mentions:
            assert mention in restored["input"]
