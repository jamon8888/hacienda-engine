"""Emit a PEFT LoRA adapter directory that xberg-gliner's loader will actually accept.

Nothing downstream validates a trained adapter except a load-time failure, and the
loader's checks fail in ways that are cheap to produce by accident and expensive to
discover — after a full training run (spec 2026-07-29 §9, verified against `xberg` tag
`v1.0.2`, commit `9dcc864`):

- `adapter_model.safetensors` must be **F32**. PEFT saves in whatever dtype the model
  trained in, so a bf16/f16 run silently yields an adapter the loader rejects outright
  (`candle/lora.rs:127`).
- `base_model_name_or_path` is matched by *bidirectional case-sensitive substring*
  against the deployed model **directory basename** (`candle/model.rs:137`), not an
  HF-style slug: `"fastino/GLiNER2-Guardrails-PII-Multi"` neither contains nor is
  contained by `"gliner2-guardrails-pii-multi"`, so writing the slug is a false
  mismatch. The guard is also skipped entirely when the field is absent, so omitting
  it by oversight disables the only cross-model sanity check that exists — this module
  therefore always writes it.
- `r = 0` is rejected at load (`candle/lora.rs:94`).

`verify_target_modules` runs the loader's one *unconditional* check (`merge_into_base`,
`candle/lora.rs:246`) before training rather than after, so a wrong module name costs
seconds instead of a training run.
"""

import json
import pathlib

import numpy as np
from safetensors.numpy import save_file

#: DeBERTa-v2 attention projections, as PEFT `target_modules` suffixes. DeBERTa-v2
#: names these `*_proj`, unlike BERT's bare `query`/`key`/`value`. These are consumed
#: by PEFT at *training* time; the Rust loader parses `target_modules` but never
#: matches on it (`candle/lora.rs:56`, `#[allow(dead_code)]`) — see
#: `verify_module_paths` for the check that actually governs merge success.
DEFAULT_TARGET_MODULES = ["query_proj", "key_proj", "value_proj"]

_LORA_SUFFIXES = (".lora_A.weight", ".lora_B.weight")
_PEFT_PREFIX = "base_model.model."


def base_model_name_for(model_dir: str | pathlib.Path) -> str:
    """The `model_id` xberg derives for `model_dir` — its basename (`candle/model.rs:199`).

    This is the value to pin as `base_model_name_or_path` at training time.
    """
    return pathlib.Path(model_dir).resolve().name


def passes_base_model_guard(model_id: str, base_model_name_or_path: str | None) -> bool:
    """Mirror of the load-time guard in `Gliner2Candle::load_adapter` (`model.rs:137`).

    Absent field means the guard is skipped, so `None` passes — that is the loader's
    documented escape hatch, not an endorsement of omitting it.
    """
    if base_model_name_or_path is None:
        return True
    return (
        base_model_name_or_path in model_id or model_id in base_model_name_or_path
    )


def verify_module_paths(base_tensor_keys: list[str], module_paths: list[str]) -> None:
    """Fail if any adapter module path has no exactly-matching base key.

    This is the loader's only *unconditional* check (`merge_into_base`,
    `candle/lora.rs:246`), and it is an exact `HashMap` lookup of
    `key.strip_suffix(".weight")` (`lora.rs:224-238`) — **not** a substring or
    suffix match. `query_proj` alone therefore never matches; the fully-qualified
    `encoder.encoder.layer.0.attention.self.query_proj` does.

    Note this is a different question from `adapter_config.json`'s `target_modules`,
    which the Rust loader parses but never uses for matching (`lora.rs:56`, marked
    `#[allow(dead_code)]`). Only the *tensor keys* determine what gets merged, so
    only they are worth verifying. Running this before training turns a wasted
    training run into an immediate error.
    """
    base = set(base_tensor_keys)
    missing = [p for p in module_paths if f"{p}.weight" not in base]
    if missing:
        raise ValueError(
            f"adapter module paths {missing[:5]} have no exactly-matching "
            f"'<path>.weight' key in the base safetensors; the adapter would be "
            f"rejected at merge time. Paths must be fully qualified against the "
            f"deployed checkpoint (note GLiNER2's doubled 'encoder.encoder.' prefix)."
        )


def module_paths_from_tensor_keys(lora_tensors: dict[str, np.ndarray]) -> list[str]:
    """The module paths `merge_into_base` will look up, derived from PEFT tensor keys.

    Mirrors `parse_lora_key` (`candle/lora.rs:172-185`): strip the
    `base_model.model.` prefix, then the `.lora_{A,B}.weight` suffix. Keys missing the
    prefix are skipped — `removeprefix` is a no-op on them, and treating the bare
    suffix-stripped key as a module path can spuriously match an unrelated base key.
    """
    paths = set()
    for key in lora_tensors:
        if not key.startswith(_PEFT_PREFIX):
            continue
        stripped = key[len(_PEFT_PREFIX) :]
        for suffix in _LORA_SUFFIXES:
            if stripped.endswith(suffix):
                paths.add(stripped[: -len(suffix)])
    return sorted(paths)


def write_adapter(
    out_dir: str | pathlib.Path,
    lora_tensors: dict[str, np.ndarray],
    *,
    base_model_dir: str | pathlib.Path,
    r: int,
    lora_alpha: float,
    target_modules: list[str] | None = None,
    fan_in_fan_out: bool = False,
    base_tensor_keys: list[str] | None = None,
) -> pathlib.Path:
    """Write `adapter_config.json` + `adapter_model.safetensors` in PEFT's shape.

    `lora_tensors` keys must follow `base_model.model.<module_path>.lora_{A,B}.weight`.
    Tensors are cast to F32 here rather than trusted to already be F32, since the
    training dtype is a property of the run, not of this call.
    """
    if r <= 0:
        raise ValueError(f"r={r} is rejected at load (candle/lora.rs:94)")

    target_modules = target_modules or DEFAULT_TARGET_MODULES

    model_id = base_model_name_for(base_model_dir)
    if not model_id:
        raise ValueError(
            f"base model directory {base_model_dir!r} resolves to an empty basename; "
            f"an empty base_model_name_or_path passes the loader's substring guard "
            f"vacuously against any deployed model, silently disabling it"
        )

    bad_keys = [
        k
        for k in lora_tensors
        if not (k.startswith(_PEFT_PREFIX) and k.endswith(_LORA_SUFFIXES))
    ]
    if bad_keys:
        raise ValueError(
            f"non-PEFT tensor keys {bad_keys}; expected names starting with "
            f"{_PEFT_PREFIX!r} and ending in {' or '.join(_LORA_SUFFIXES)}"
        )
    if not lora_tensors:
        raise ValueError("refusing to write an adapter with no LoRA tensors")

    if base_tensor_keys is not None:
        verify_module_paths(base_tensor_keys, module_paths_from_tensor_keys(lora_tensors))

    out_path = pathlib.Path(out_dir)
    out_path.mkdir(parents=True, exist_ok=True)

    f32_tensors = {k: np.ascontiguousarray(v, dtype=np.float32) for k, v in lora_tensors.items()}
    save_file(f32_tensors, str(out_path / "adapter_model.safetensors"))

    config = {
        "peft_type": "LORA",
        "r": r,
        "lora_alpha": lora_alpha,
        "target_modules": target_modules,
        # Always written: absence silently disables the guard (spec §9).
        "base_model_name_or_path": model_id,
        "fan_in_fan_out": fan_in_fan_out,
    }
    (out_path / "adapter_config.json").write_text(
        json.dumps(config, indent=2) + "\n", encoding="utf-8"
    )

    return out_path
