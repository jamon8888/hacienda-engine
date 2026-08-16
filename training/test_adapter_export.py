import json

import numpy as np
import pytest
from adapter_export import (
    base_model_name_for,
    module_paths_from_tensor_keys,
    passes_base_model_guard,
    verify_module_paths,
    write_adapter,
)

# The real path in fastino/GLiNER2-Guardrails-PII-Multi, verified against the
# checkpoint header: GLiNER2 nests HF's DebertaV2Model under its own `encoder`
# attribute, so the prefix is doubled.
QUERY_PROJ = "encoder.encoder.layer.0.attention.self.query_proj"


def make_tensors() -> dict[str, np.ndarray]:
    return {
        f"base_model.model.{QUERY_PROJ}.lora_A.weight": np.zeros((8, 768), dtype=np.float16),
        f"base_model.model.{QUERY_PROJ}.lora_B.weight": np.zeros((768, 8), dtype=np.float16),
    }


def test_tensors_are_written_as_f32_even_when_trained_in_half_precision(tmp_path):
    from safetensors import safe_open

    out = write_adapter(
        tmp_path / "adapter",
        make_tensors(),
        base_model_dir=tmp_path / "gliner2-guardrails-pii-multi",
        r=8,
        lora_alpha=16,
    )

    with safe_open(str(out / "adapter_model.safetensors"), framework="numpy") as f:
        for key in f.keys():
            assert f.get_tensor(key).dtype == np.float32, (
                "the loader rejects any non-F32 adapter dtype (candle/lora.rs:127)"
            )


def test_base_model_name_is_the_directory_basename_not_an_hf_slug(tmp_path):
    out = write_adapter(
        tmp_path / "adapter",
        make_tensors(),
        base_model_dir=tmp_path / "gliner2-guardrails-pii-multi",
        r=8,
        lora_alpha=16,
    )

    config = json.loads((out / "adapter_config.json").read_text())
    assert config["base_model_name_or_path"] == "gliner2-guardrails-pii-multi"


def test_written_config_always_sets_the_base_model_guard_field(tmp_path):
    out = write_adapter(
        tmp_path / "adapter",
        make_tensors(),
        base_model_dir=tmp_path / "gliner2-guardrails-pii-multi",
        r=8,
        lora_alpha=16,
    )

    config = json.loads((out / "adapter_config.json").read_text())
    # Omitting this field is a documented bypass of the only cross-model check.
    assert "base_model_name_or_path" in config


def test_the_written_name_passes_the_loaders_substring_guard(tmp_path):
    model_dir = tmp_path / "gliner2-guardrails-pii-multi"
    out = write_adapter(
        tmp_path / "adapter",
        make_tensors(),
        base_model_dir=model_dir,
        r=8,
        lora_alpha=16,
    )

    config = json.loads((out / "adapter_config.json").read_text())
    assert passes_base_model_guard(
        base_model_name_for(model_dir), config["base_model_name_or_path"]
    )


def test_an_hf_style_slug_would_fail_the_guard_against_a_lowercase_directory():
    # Documents the trap in spec §9: the mixed-case slug neither contains nor is
    # contained by the deployed lowercase basename, so pinning it silently produces
    # an adapter that is refused at merge time.
    assert not passes_base_model_guard(
        "gliner2-guardrails-pii-multi", "fastino/GLiNER2-Guardrails-PII-Multi"
    )


def test_absent_base_model_name_skips_the_guard():
    assert passes_base_model_guard("gliner2-guardrails-pii-multi", None)


def test_module_path_missing_from_the_base_checkpoint_is_rejected_before_training():
    base_keys = [f"{QUERY_PROJ}.weight"]

    with pytest.raises(ValueError):
        verify_module_paths(base_keys, [QUERY_PROJ, "encoder.encoder.layer.0.nonexistent"])


def test_module_paths_present_in_the_base_checkpoint_pass():
    verify_module_paths([f"{QUERY_PROJ}.weight"], [QUERY_PROJ])


def test_a_bare_module_suffix_does_not_satisfy_the_exact_match_merge():
    # merge_into_base does an exact HashMap lookup of key.strip_suffix(".weight"),
    # not a substring match, so the short PEFT-style suffix is not a valid path.
    with pytest.raises(ValueError):
        verify_module_paths([f"{QUERY_PROJ}.weight"], ["query_proj"])


def test_module_paths_are_derived_from_peft_tensor_keys():
    assert module_paths_from_tensor_keys(make_tensors()) == [QUERY_PROJ]


def test_export_rejects_tensors_that_would_fail_the_merge_coverage_check(tmp_path):
    with pytest.raises(ValueError):
        write_adapter(
            tmp_path / "adapter",
            make_tensors(),
            base_model_dir=tmp_path / "gliner2-guardrails-pii-multi",
            r=8,
            lora_alpha=16,
            base_tensor_keys=["some.other.module.weight"],
        )


def test_export_accepts_tensors_matching_the_base_checkpoint(tmp_path):
    write_adapter(
        tmp_path / "adapter",
        make_tensors(),
        base_model_dir=tmp_path / "gliner2-guardrails-pii-multi",
        r=8,
        lora_alpha=16,
        base_tensor_keys=[f"{QUERY_PROJ}.weight"],
    )


def test_rank_zero_is_rejected(tmp_path):
    with pytest.raises(ValueError):
        write_adapter(
            tmp_path / "adapter",
            make_tensors(),
            base_model_dir=tmp_path / "gliner2-guardrails-pii-multi",
            r=0,
            lora_alpha=16,
        )


def test_negative_rank_is_rejected(tmp_path):
    with pytest.raises(ValueError):
        write_adapter(
            tmp_path / "adapter",
            make_tensors(),
            base_model_dir=tmp_path / "gliner2-guardrails-pii-multi",
            r=-1,
            lora_alpha=16,
        )


def test_a_base_model_dir_resolving_to_an_empty_basename_is_rejected(tmp_path):
    # base_model_name_for("/") returns "", which would pass the loader's substring
    # guard vacuously against any deployed model, silently disabling the one
    # cross-model check this module promises to preserve.
    with pytest.raises(ValueError):
        write_adapter(
            tmp_path / "adapter",
            make_tensors(),
            base_model_dir="/",
            r=8,
            lora_alpha=16,
        )


def test_non_peft_tensor_keys_are_rejected(tmp_path):
    with pytest.raises(ValueError):
        write_adapter(
            tmp_path / "adapter",
            {"encoder.query_proj.weight": np.zeros((8, 8), dtype=np.float32)},
            base_model_dir=tmp_path / "gliner2-guardrails-pii-multi",
            r=8,
            lora_alpha=16,
        )


def test_a_suffix_valid_key_missing_the_peft_prefix_is_rejected(tmp_path):
    # removeprefix is a no-op when the base_model.model. prefix is absent, so a key
    # with only the LoRA suffix can still resolve to a module path that spuriously
    # matches an unrelated base key — the exporter must catch this, not just the
    # loader's parse_lora_key at load time.
    with pytest.raises(ValueError):
        write_adapter(
            tmp_path / "adapter",
            {f"{QUERY_PROJ}.lora_A.weight": np.zeros((8, 768), dtype=np.float32)},
            base_model_dir=tmp_path / "gliner2-guardrails-pii-multi",
            r=8,
            lora_alpha=16,
        )


def test_module_paths_from_tensor_keys_skips_keys_missing_the_peft_prefix():
    keys = {
        f"{QUERY_PROJ}.lora_A.weight": np.zeros((8, 768), dtype=np.float32),
        f"base_model.model.{QUERY_PROJ}.lora_B.weight": np.zeros((768, 8), dtype=np.float32),
    }

    assert module_paths_from_tensor_keys(keys) == [QUERY_PROJ]


def test_an_empty_adapter_is_rejected(tmp_path):
    with pytest.raises(ValueError):
        write_adapter(
            tmp_path / "adapter",
            {},
            base_model_dir=tmp_path / "gliner2-guardrails-pii-multi",
            r=8,
            lora_alpha=16,
        )
