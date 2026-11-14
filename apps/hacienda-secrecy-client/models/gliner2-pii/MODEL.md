# Fastino GLiNER2 multilingual PII checkpoint

LocalFull uses the official `fastino/gliner2-privacy-filter-PII-multi` GLiNER2
span checkpoint at immutable revision
`1cb4166094dc58fa8d836429f060d6c95f62b495`. The model card declares
Apache-2.0 and seven languages: English, French, Spanish, German, Italian,
Portuguese, and Dutch.

Do not substitute a `gliner2.5-*` boundary checkpoint: the pinned Xberg
Candle runtime implements span heads and expects this GLiNER2 span checkpoint.

This is a PII detection aid, not a guarantee of privacy or legal compliance.
The official card reports modest precision, over-prediction of name-like spans,
evaluation on only the SPY benchmark's two domains, fully synthetic training
data without human validation, and no measured performance on non-European
locales or scripts. Calibrate thresholds and review redaction results for the
intended domain; never treat a missed span as evidence that text is safe.

The installer fetches files only when explicitly invoked with `--download`.
Runtime code must use an already-installed directory and must not download a
checkpoint implicitly.

The desktop Tauri runtime looks first at the explicit
`HACIENDA_NER_MODEL_DIR` environment variable. When it is unset, install the
allowlisted files under the app-local data directory at
`models/gliner2-pii` (for example, `<app-local-data>/models/gliner2-pii`).
The runtime verifies every file size and SHA-256 digest pinned by
`manifest.json`, and also checks the GLiNER2 span/extractor and DeBERTa-v2 architecture before
loading Candle. A missing, altered, or GLiNER2.5 boundary checkpoint leaves
provider egress disabled; it never triggers a download or reports the model
path in status.

The desktop Tauri package enables its `ner-candle` feature by default. Loading
the model is intentionally eager at startup and may consume roughly 1.2 GiB
of disk-read bandwidth plus the Candle model's resident memory; the remaining
runtime smoke test is to launch the packaged desktop app with the installed
checkpoint and confirm the status reports `local_model` and provider redaction
still emits reversible tokens.
