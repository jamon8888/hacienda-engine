# Product

<!-- impeccable:product-schema 1 -->

## Platform

web

## Users

Three confirmed audiences, all underserved by cloud redaction tools:

- **Individual professionals** — lawyers, consultants, freelancers — handling sensitive client documents solo, without an IT department to vet a SaaS vendor for them.
- **Compliance/legal teams at companies** operating under regulatory obligation (GDPR, HIPAA, and similar), where an auditable record of what was redacted and when matters as much as the redaction itself.
- **Developers/technical teams** embedding the extraction/detection/redaction pipeline into a larger system rather than using the UI directly.

## Product Purpose

Hacienda Studio strips PII out of documents entirely on-device, so they can be safely handed to a party that shouldn't see the raw content — most concretely, **pasted or uploaded into AI chatbots like Claude and ChatGPT** without leaking sensitive information to that provider. Success is a document a user can confidently share, submit, or feed into a third-party tool, plus a verifiable record proving what was redacted.

## Positioning

**Documents never leave the device.** Extraction, PII detection, and redaction all run in-browser via WebAssembly and a Web Worker — there is no server round-trip for a competing cloud-based redaction tool (or a raw paste into Claude/ChatGPT) to intercept. This is architectural, not a policy promise: the product has no backend to send the document to.

## Operating Context

The core workflow: a user has a document containing PII and needs to use it somewhere that shouldn't see that PII unredacted — most often pasting/uploading it into an AI chatbot (Claude, ChatGPT) for analysis or drafting help, but also internal sharing, discovery, or compliance review. They drop the file in, review what the pipeline found, correct any misses, choose how each span gets redacted, and export.

## Capabilities and Constraints

- Extracts PDF, DOCX, XLSX, PPTX, email, images, and plain text to clean markdown with a layout map.
- Detects PII via a Rust/WASM regex engine plus an optional local NER model, with per-finding confidence scores.
- Four redaction modes per finding: Mask, Hash, Pseudonymize (reversible with a passphrase), Remove.
- Every redaction is appended to a local, tamper-evident blake3 hash-chain audit log, independently verifiable without a server.
- Runs in a Web Worker; no accounts, no server storage. Works offline once on-device assets (WASM runtime, NER model, OCR data) are cached in IndexedDB.
- Two-step upload: files land in a review queue before processing starts, not processed on drop.
- Native viewers (docx/xlsx/pptx/pdf) for previewing the original alongside the redacted output.
- Export as a redacted document or a zip of the processed batch.

## Brand Commitments

Name: **Hacienda Studio**. Core promise repeated across all copy: zero uploads, 100% on-device processing — "Redact sensitive documents without letting them leave your laptop."

## Evidence on Hand

No customer testimonials, case studies, or usage metrics exist yet. Do not fabricate any — the current copy makes no such claims and future work should not add them without real evidence.

## Product Principles

1. Privacy is architectural, not a policy — nothing leaves the device, so there is nothing to promise not to misuse.
2. Every redaction must be provable, not just asserted — the audit chain lets anyone verify after the fact, without trusting the tool's word for it.
3. The product's job is to make it safe to hand sensitive material to AI tools, not to compete with them.
4. The same pipeline serves a solo professional clicking through the UI and a developer embedding it — neither audience is a second-class citizen of the design.
