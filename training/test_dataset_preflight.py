import pytest
from dataset_preflight import (
    assert_split_integrity,
    gliner2_tokens,
    unresolvable_mentions,
    width_violations,
)


def record(text: str, entities: dict[str, list[str]], doc_id: str = "d0") -> dict:
    return {"input": text, "output": {"entities": entities}, "doc_id": doc_id}


def test_a_span_at_the_width_cap_is_reported_because_the_bound_is_exclusive():
    # model.py:594-604 sets a label only when `0 <= width < scores.shape[3]`, so a
    # span of exactly max_width tokens is already unlearnable, not borderline.
    mention = " ".join(f"w{i}" for i in range(8))
    findings = width_violations([record(mention, {"clause": [mention]})], max_width=8)

    assert [f.mention for f in findings] == [mention]


def test_a_span_one_token_under_the_cap_is_not_reported():
    mention = " ".join(f"w{i}" for i in range(7))
    assert width_violations([record(mention, {"clause": [mention]})], max_width=8) == []


def test_width_is_counted_in_gliner2_tokens_not_whitespace_words():
    # Punctuation is its own token, so this is 8 tokens to GLiNER2 but 5 to str.split().
    mention = "Acme, Globex, Initech, and Umbrella"
    assert len(mention.split()) < 8
    findings = width_violations([record(mention, {"party": [mention]})], max_width=8)

    assert len(findings) == 1, "counting whitespace words understates the real width"


def test_width_findings_carry_the_label_so_a_corpus_level_decision_is_possible():
    long_mention = " ".join(f"w{i}" for i in range(12))
    findings = width_violations(
        [record(long_mention, {"governing_law_clause": [long_mention]})], max_width=8
    )

    assert {f.label for f in findings} == {"governing_law_clause"}


def test_width_findings_identify_the_offending_record():
    long_mention = " ".join(f"w{i}" for i in range(12))
    records = [record("Acme.", {"party": ["Acme"]}), record(long_mention, {"c": [long_mention]})]

    findings = width_violations(records, max_width=8)

    assert [f.record_index for f in findings] == [1]


def test_a_mention_present_verbatim_resolves():
    assert unresolvable_mentions([record("Acme pays Globex.", {"party": ["Globex"]})]) == []


def test_matching_is_case_insensitive_like_gliner2s_lowercased_search():
    assert unresolvable_mentions([record("ACME pays Globex.", {"party": ["Acme"]})]) == []


def test_punctuation_attached_to_a_token_does_not_produce_a_false_positive():
    # A naive text.lower().split() leaves 'inc.' glued together and reports this
    # perfectly resolvable mention as missing, degrading the preflight into noise.
    assert unresolvable_mentions([record("Notice to Globex Inc. here.", {"p": ["Globex Inc"]})]) == []


def test_extra_whitespace_between_words_does_not_produce_a_false_positive():
    assert unresolvable_mentions([record("Notice to  Globex  Inc.", {"p": ["Globex Inc"]})]) == []


def test_a_mention_absent_from_the_text_is_reported():
    findings = unresolvable_mentions([record("Acme pays Globex.", {"party": ["Initech"]})])

    assert [(f.record_index, f.label, f.mention) for f in findings] == [(0, "party", "Initech")]


def test_a_mention_whose_words_appear_out_of_order_is_reported():
    # _find_sublist is a contiguous sublist search, not a bag-of-words check.
    findings = unresolvable_mentions([record("Globex pays Acme.", {"party": ["Acme Globex"]})])

    assert len(findings) == 1


def test_a_reconstructed_mention_that_dropped_punctuation_is_reported():
    # The exact failure mode that makes rebuilding mentions from tokens unsafe.
    findings = unresolvable_mentions([record("Sued Globex, Inc. today.", {"p": ["Globex Inc."]})])

    assert len(findings) == 1


def test_gliner2_tokens_splits_punctuation_off_and_lowercases():
    assert gliner2_tokens("Globex, Inc.") == ["globex", ",", "inc", "."]


def test_gliner2_tokens_keeps_a_url_as_one_token():
    assert gliner2_tokens("see https://x.test/a?b=1 now") == ["see", "https://x.test/a?b=1", "now"]


def test_gliner2_tokens_keeps_an_email_as_one_token():
    assert gliner2_tokens("mail a@b.test now") == ["mail", "a@b.test", "now"]


def test_a_doc_id_in_two_splits_raises_because_it_is_always_a_bug():
    splits = {
        "train": [record("a", {}, doc_id="d1")],
        "val": [],
        "test": [record("b", {}, doc_id="d1")],
    }

    with pytest.raises(ValueError, match="d1"):
        assert_split_integrity(splits)


def test_disjoint_splits_pass():
    splits = {
        "train": [record("a", {}, doc_id="d1")],
        "val": [record("b", {}, doc_id="d2")],
        "test": [record("c", {}, doc_id="d3")],
    }

    assert_split_integrity(splits)


def test_the_same_doc_id_appearing_twice_within_one_split_is_fine():
    # Multiple chunks of one document belong together in one split by design.
    splits = {"train": [record("a", {}, doc_id="d1"), record("b", {}, doc_id="d1")], "val": []}

    assert_split_integrity(splits)


def test_width_and_resolvability_report_rather_than_raise():
    # Both are corpus facts needing a human decision, unlike split leakage which is
    # always a bug. Raising here would push callers toward silently filtering.
    bad = record("short", {"c": ["nowhere to be found"]})

    assert width_violations([bad]) == []
    assert len(unresolvable_mentions([bad])) == 1
