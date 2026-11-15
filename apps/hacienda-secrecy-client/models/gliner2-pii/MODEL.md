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
remaining runtime smoke check is to inspect the packaged desktop UI and confirm
it displays `local_model` for the installed checkpoint.

## Local runtime verification

On 2026-11-15, the ignored Tauri test
`pinned_gliner2_model_runs_french_detection_and_redaction` loaded the installed
checkpoint on an Intel Core i7-3520M (2 cores / 4 logical CPUs, 16 GiB RAM).
It passed in both test profiles: 269.13 s in the debug profile and 25.53 s in
the optimized release profile. The test sends French text containing a person
and a city through the same `ProviderRequest` redaction adapter used before
egress, and verifies the serialized request contains only reversible
`[PERSON:local:…]` and `[ADDRESS:local:…]` tokens, with neither source value.
The same loaded redactor also produces `provider_send_enabled: true` and
`detector: "local_model"` through the status object used by the Tauri command.

This proves model loading, French inference, and the provider DTO redaction
boundary in the Rust test binary. It does not verify GUI startup, OS keychain
integration, or the status shown by a packaged desktop app; those remain the
packaged-runtime smoke check.

The Tauri release build completed for the macOS `.app` target, and that app's
native process started and remained alive. The Rust status contract is verified
with the real model above, but the rendered GUI status could not be observed in
this run because the computer-use native pipe failed to start. The default DMG
target also failed in Tauri's `bundle_dmg.sh`; the `.app` target itself
completed successfully.
