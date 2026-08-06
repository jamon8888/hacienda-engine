"""Resolve an LLM-labeled entity mention back to a character offset.

The labeler is asked for a verbatim substring, never a character offset — an LLM
confabulates offsets past a sentence or two. This module does the offset resolution
deterministically, downstream of generation, per spec §6.2: unique match wins,
ambiguous matches are disambiguated by surrounding context, and anything that still
can't be resolved is dropped rather than guessed.
"""

from typing import Optional


def resolve_offset(
    chunk: str,
    entity_text: str,
    context_before: Optional[str] = None,
    context_after: Optional[str] = None,
) -> Optional[tuple[int, int]]:
    """Return the (start, end) character span of `entity_text` in `chunk`, or None."""
    if not entity_text:
        return None

    positions = []
    search_from = 0
    while True:
        idx = chunk.find(entity_text, search_from)
        if idx == -1:
            break
        positions.append(idx)
        search_from = idx + 1

    if not positions:
        return None
    if len(positions) == 1:
        idx = positions[0]
        return (idx, idx + len(entity_text))

    # Multiple occurrences: only context can disambiguate. A position with no
    # context supplied is treated as unconstrained on that side, not as a match —
    # if neither side is supplied, every position stays a candidate and the
    # ambiguity is correctly preserved (falls through to "not unique" below).
    candidates = []
    for idx in positions:
        end = idx + len(entity_text)
        before_ok = context_before is None or chunk[:idx].rstrip().endswith(
            context_before.rstrip()
        )
        after_ok = context_after is None or chunk[end:].lstrip().startswith(
            context_after.lstrip()
        )
        if before_ok and after_ok:
            candidates.append((idx, end))

    if len(candidates) == 1:
        return candidates[0]
    return None
