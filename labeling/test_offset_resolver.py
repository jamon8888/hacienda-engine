from offset_resolver import resolve_offset


def test_unique_match_resolves_directly():
    chunk = "The Licensee shall not assign this Agreement without consent."
    entity_text = "assign this Agreement"
    expected_start = chunk.index(entity_text)

    result = resolve_offset(chunk, entity_text, None, None)

    assert result == (expected_start, expected_start + len(entity_text))


def test_ambiguous_match_resolved_by_context():
    chunk = "Acme shall notify Beta. Beta shall notify Acme in return."

    result = resolve_offset(
        chunk, "Acme", context_before=None, context_after="shall notify Beta"
    )

    assert result == (0, 4)


def test_unresolvable_match_returns_none_rather_than_guessing():
    chunk = "Acme shall notify Beta. Beta shall notify Acme in return."

    assert resolve_offset(chunk, "Acme", context_before=None, context_after=None) is None


def test_text_not_present_returns_none():
    chunk = "The parties agree to arbitration in Delaware."

    assert resolve_offset(chunk, "mediation", None, None) is None


def test_empty_entity_text_returns_none():
    assert resolve_offset("some text", "", None, None) is None


def test_context_disambiguates_the_second_of_two_occurrences():
    chunk = "Acme shall notify Beta. Beta shall notify Acme in return."

    result = resolve_offset(
        chunk, "Acme", context_before="notify", context_after=None
    )

    # "notify Beta" precedes the FIRST Acme's neighbourhood via a different match;
    # here context_before="notify" should match whichever occurrence is
    # immediately preceded by "notify" once trailing whitespace is stripped.
    second_acme_start = chunk.rindex("Acme")
    assert result == (second_acme_start, second_acme_start + len("Acme"))
