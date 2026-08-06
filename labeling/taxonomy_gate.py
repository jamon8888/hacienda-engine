"""Reject any LLM-proposed label outside the business_law taxonomy.

LLMs invent near-miss labels (`contracting_party_name` for `contracting_party`) that
fragment the class distribution if not gated (spec §6.2). The taxonomy YAML is the
single source of truth — this module parses the same file
`apps/hacienda-studio/lib/verticals/business_law.yaml` uses, rather than hand-copying
the label list into Python, so the two can't drift apart.
"""

import pathlib

import yaml

_TAXONOMY_PATH = (
    pathlib.Path(__file__).resolve().parent.parent
    / "apps"
    / "hacienda-studio"
    / "lib"
    / "verticals"
    / "business_law.yaml"
)


def _load_entity_types(path: pathlib.Path) -> frozenset[str]:
    with open(path, encoding="utf-8") as f:
        data = yaml.safe_load(f)
    return frozenset(data["entityTypes"])


_ENTITY_TYPES = _load_entity_types(_TAXONOMY_PATH)


def is_valid_label(label: str) -> bool:
    """True iff `label` is exactly one of business_law.yaml's entityTypes."""
    return label in _ENTITY_TYPES
