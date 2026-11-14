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
