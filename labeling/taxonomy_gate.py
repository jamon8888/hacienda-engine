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
    """Load and validate the set of entity types from a taxonomy YAML file.

    The YAML is expected to be a mapping with an "entityTypes" key whose value is
    a list of strings. Any deviation from this schema raises a ValueError with a
    clear message so schema drift or corruption in the single source of truth is
    caught explicitly at startup rather than surfacing as a confusing KeyError or
    silently gating every label.
    """
    with open(path, encoding="utf-8") as f:
        data = yaml.safe_load(f)

    if not isinstance(data, dict):
        raise ValueError(
            f"Taxonomy file {path} must contain a mapping at the top level; "
            f"got {type(data).__name__!r}"
        )

    if "entityTypes" not in data:
        raise ValueError(f"Taxonomy file {path} is missing required 'entityTypes' field")

    entity_types = data["entityTypes"]

    if not isinstance(entity_types, list):
        raise ValueError(
            f"'entityTypes' in {path} must be a list; got {type(entity_types).__name__!r}"
        )

    if not all(isinstance(t, str) for t in entity_types):
        raise ValueError(
            f"All entries in 'entityTypes' in {path} must be strings; got {entity_types!r}"
        )

    return frozenset(entity_types)


_ENTITY_TYPES = _load_entity_types(_TAXONOMY_PATH)


def is_valid_label(label: str) -> bool:
    """True iff `label` is exactly one of business_law.yaml's entityTypes."""
    return label in _ENTITY_TYPES
