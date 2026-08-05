"""Self-consistency voting across repeated LLM label samples.

Spec §6.3: sample each chunk 3x and keep spans with >=2/3 (offset, label) agreement
for training; 1/3-agreement spans go to a human review queue instead, not straight
into the training set. Full agreement across samples reduces hallucination, it does
not eliminate labels the LLM is confidently and consistently wrong about (spec §7) —
that's what the review queue and the human spot-check are for.
"""

from collections import Counter
from dataclasses import dataclass

Span = tuple[int, int, str]


@dataclass(frozen=True)
class VoteResult:
    accepted: list[Span]
    review_queue: list[Span]


def vote(samples: list[list[Span]], min_agreement: int = 2) -> VoteResult:
    """`samples` is one list of (start, end, label) spans per LLM sample."""
    if not samples:
        raise ValueError("samples must be non-empty")
    if min_agreement < 1:
        raise ValueError(f"min_agreement must be >= 1, got {min_agreement}")
    if min_agreement > len(samples):
        raise ValueError(
            f"min_agreement ({min_agreement}) cannot exceed number of samples ({len(samples)})"
        )

    counts: Counter[Span] = Counter()
    for sample in samples:
        # De-duped within a sample first: a labeler repeating the same span twice
        # in one completion must not count as two votes for it.
        for span in set(sample):
            counts[span] += 1

    accepted = [span for span, count in counts.items() if count >= min_agreement]
    review_queue = [span for span, count in counts.items() if 0 < count < min_agreement]
    return VoteResult(accepted=accepted, review_queue=review_queue)
