---
priority: high
---

- Vertical NER specialization is GLiNER2 base model + a per-vertical LoRA adapter (e.g. `business_law`), merged at model-load time — never bake a vertical into the base weights.
- The vertical taxonomy YAML (e.g. `apps/hacienda-studio/lib/verticals/business_law.yaml`) is the single source of truth for entity types, shared between the frontend and the labeling pipeline — never let label sets drift between them.
- `labeling/taxonomy_gate.py` rejects any LLM-proposed label outside the taxonomy — do not bypass the gate to "just get more training data."
- `labeling/offset_resolver.py` resolves verbatim mentions to character offsets deterministically — unresolvable mentions are dropped, never guessed.
- `labeling/consistency.py` requires ≥2/3 agreement across 3x self-consistency sampling before a label is accepted for training; 1/3 agreement routes to human review, it is not discarded.
- `dataset/assemble.py` converts character spans to GLiNER2 word-token spans with a mandatory round-trip assertion — a failing assertion means the assembly step is broken, not the input document.
- Train/val/test splits preserve document boundaries (no chunk-level leakage); the test set is fully-human-reviewed documents only.
- No model weights are committed to the repo — `scripts/convert_gliner2_f16.py` handles F32→F16 conversion for distribution outside git.
- `hacienda-core/tests/lora_adapter_contract.rs` is the contract test for adapter loading — it must fail loudly (never panic) on a missing or malformed adapter/model directory.
