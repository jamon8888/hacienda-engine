---
priority: critical
---

- Pipeline order is fixed: regex detection + optional NER detection → merge into non-overlapping spans → redaction → audit chain. Never redact before merge — overlapping spans corrupt offsets.
- Pseudonymize is the only reversible redaction mode (AES-256-SIV). Mask, hash, and remove are one-way — never claim they can be undone.
- Pseudonymization keys come from `HACIENDA_PSEUDONYM_ACTIVE_KEY` plus retired keys for rotation — never hardcode or log key material.
- Normalize values (NFKC, case, whitespace collapse; digits-only for phone numbers) before pseudonymizing so equivalent mentions resolve to the same token — do not normalize so aggressively that distinct entities collide.
- Pseudonym tokens follow `[CATEGORY:key_id:base32_ciphertext]` — preserve this format for downstream re-identification tooling.
- Test both directions: redaction must leak nothing (mask/hash paths), and pseudonymize-without-a-key must fail loudly, never silently degrade to a weaker mode.
- The NER path is optional and additive to regex — a missing or unloadable NER model must not break the regex-only pipeline.
- Audit chain entries are append-only — never mutate or truncate prior entries when adding new redaction events.
