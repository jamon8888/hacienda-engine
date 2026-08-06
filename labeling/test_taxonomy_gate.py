import pathlib

import pytest

from taxonomy_gate import _load_entity_types, is_valid_label


def test_accepts_a_taxonomy_label():
    assert is_valid_label("contracting_party") is True


def test_accepts_every_label_actually_in_the_yaml_file():
    # Single-source-of-truth check: if business_law.yaml gains/loses a label and
    # this module isn't re-derived from it, this is what would catch the drift.
    for label in _load_entity_types(
        pathlib.Path(__file__).resolve().parent.parent
        / "apps"
        / "hacienda-studio"
        / "lib"
        / "verticals"
        / "business_law.yaml"
    ):
        assert is_valid_label(label) is True


def test_rejects_a_near_miss_label():
    # LLMs invent labels like this — must be gated, not silently accepted (spec §6.2).
    assert is_valid_label("contracting_party_name") is False


def test_rejects_case_variants_to_force_exact_taxonomy_match():
    assert is_valid_label("Contracting_Party") is False


def test_rejects_an_empty_label():
    assert is_valid_label("") is False


def test_missing_taxonomy_file_fails_loudly(tmp_path):
    with pytest.raises(FileNotFoundError):
        _load_entity_types(tmp_path / "does_not_exist.yaml")
