from consistency import vote


def test_majority_agreement_is_accepted():
    samples = [
        [(0, 4, "contracting_party")],
        [(0, 4, "contracting_party")],
        [(0, 4, "counterparty_role")],  # disagrees on label, not span
    ]

    result = vote(samples)

    assert (0, 4, "contracting_party") in result.accepted


def test_single_sample_agreement_goes_to_review_not_training():
    samples = [
        [(10, 20, "force_majeure")],
        [],
        [],
    ]

    result = vote(samples)

    assert (10, 20, "force_majeure") in result.review_queue
    assert (10, 20, "force_majeure") not in result.accepted


def test_full_agreement_is_accepted_and_not_in_review_queue():
    samples = [
        [(0, 4, "contracting_party")],
        [(0, 4, "contracting_party")],
        [(0, 4, "contracting_party")],
    ]

    result = vote(samples)

    assert (0, 4, "contracting_party") in result.accepted
    assert (0, 4, "contracting_party") not in result.review_queue


def test_a_span_repeated_twice_in_one_sample_counts_once_not_twice():
    samples = [
        [(0, 4, "contracting_party"), (0, 4, "contracting_party")],
        [],
        [],
    ]

    result = vote(samples)

    # A single sample repeating a span must not be enough to hit min_agreement=2 on
    # its own — that would let one hallucinating completion outvote consensus.
    assert (0, 4, "contracting_party") not in result.accepted
    assert (0, 4, "contracting_party") in result.review_queue


def test_no_agreement_at_all_produces_empty_result():
    result = vote([[], [], []])

    assert result.accepted == []
    assert result.review_queue == []
