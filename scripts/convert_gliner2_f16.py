#!/usr/bin/env python3
"""
Convert GLiNER2 F32 checkpoint to F16 (half precision).

Usage:
    python convert_gliner2_f16.py --input-dir ./model_f32 --output-dir ./model_f16

Input: model.safetensors (F32), tokenizer.json, encoder_config/config.json
Output: model.safetensors (F16), tokenizer.json (copied), encoder_config/config.json (copied)

Streaming conversion — processes one tensor at a time via mmap, needs only numpy.
"""

import argparse
import hashlib
import json
import shutil
from pathlib import Path

import numpy as np
from safetensors import safe_open
from safetensors.numpy import save_file


def convert_safetensors_f32_to_f16(input_path: Path, output_path: Path) -> str:
    """Convert safetensors from F32 to F16, streaming one tensor at a time."""
    tensors = {}
    with safe_open(input_path, framework="numpy", device="cpu") as f:
        for key in f.keys():
            tensor = f.get_tensor(key)
            if tensor.dtype == np.float32:
                tensors[key] = tensor.astype(np.float16)
            else:
                tensors[key] = tensor  # keep non-float tensors as-is (int64, etc.)
    save_file(tensors, output_path)

    # Return SHA256 of output
    h = hashlib.sha256()
    with open(output_path, "rb") as f:
        for chunk in iter(lambda: f.read(8192), b""):
            h.update(chunk)
    return h.hexdigest()


def copy_other_files(input_dir: Path, output_dir: Path):
    """Copy tokenizer.json and encoder_config/ as-is."""
    for rel in ["tokenizer.json", "encoder_config/config.json"]:
        src = input_dir / rel
        dst = output_dir / rel
        if src.exists():
            dst.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(src, dst)
        else:
            print(f"Warning: {rel} not found in {input_dir}")


def main():
    parser = argparse.ArgumentParser(description="Convert GLiNER2 F32 to F16")
    parser.add_argument("--input-dir", type=Path, required=True,
                        help="Directory containing model.safetensors (F32), tokenizer.json, encoder_config/")
    parser.add_argument("--output-dir", type=Path, required=True,
                        help="Output directory for F16 model")
    parser.add_argument("--source-sha256", type=str,
                        help="Expected SHA256 of input model.safetensors (for verification)")
    args = parser.parse_args()

    input_model = args.input_dir / "model.safetensors"
    if not input_model.exists():
        raise FileNotFoundError(f"Input model not found: {input_model}")

    # Verify source hash if provided
    if args.source_sha256:
        h = hashlib.sha256()
        with open(input_model, "rb") as f:
            for chunk in iter(lambda: f.read(8192), b""):
                h.update(chunk)
        actual = h.hexdigest()
        if actual != args.source_sha256:
            raise ValueError(f"Source hash mismatch: expected {args.source_sha256}, got {actual}")
        print(f"Verified source model SHA256: {actual}")

    args.output_dir.mkdir(parents=True, exist_ok=True)

    output_model = args.output_dir / "model.safetensors"
    print(f"Converting {input_model} -> {output_model} (F32 -> F16)...")
    output_sha256 = convert_safetensors_f32_to_f16(input_model, output_model)
    print(f"Output model SHA256: {output_sha256}")

    copy_other_files(args.input_dir, args.output_dir)

    # Write hash file
    hash_file = args.output_dir / "model.safetensors.sha256"
    hash_file.write_text(f"{output_sha256}  model.safetensors\n")
    print(f"Wrote {hash_file}")

    # Verify output size
    input_size = input_model.stat().st_size
    output_size = output_model.stat().st_size
    print(f"Input size:  {input_size:,} bytes ({input_size / 1e9:.2f} GB)")
    print(f"Output size: {output_size:,} bytes ({output_size / 1e9:.2f} GB)")
    print(f"Reduction:   {100 * (1 - output_size / input_size):.1f}%")

    print("\n--- Publish instructions ---")
    print(f"1. Upload {args.output_dir}/ contents to your model host")
    print(f"2. Set VITE_MODEL_BASE_URL to the resolved URL of model.safetensors")
    print(f"   Example: https://huggingface.co/your-org/your-repo/resolve/main")
    print(f"3. Verify: curl -I <VITE_MODEL_BASE_URL>/model.safetensors")
    print(f"   Should return 200 with content-type application/octet-stream")


if __name__ == "__main__":
    main()