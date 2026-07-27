# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- Initial hacienda distribution crate
- hacienda-core with PII pipeline, redaction, compliance, audit, review, glossary
- xberg integration via PostProcessor and NerBackend traits
- Full polyglot bindings (14 languages) via alef
- CLI with pii, compliance, review, audit subcommands
- REST API with PII, compliance, review, audit endpoints
- Feature flags: xberg-full, pii, compliance, audit, review, glossary

### Changed

- Migrated from xberg-pii-ecosystem to hacienda distribution

### Security

- Added FPE encryption for pseudonymization
- Hash-chained audit log with blake3
- JWT authentication for API

## [0.1.0] - 2026-07-28

### Added

- First public release
- GDPR/DORA/AI Act compliance features
- 42 PII regex patterns + GLiNER2 ML backend
- 5 redaction modes (Mask, Hash, Pseudonymize, Remove, Custom)
- DPIA, Model Card, DORA, AI Act report generators
- Human review queue with Approve/Reject/Modify
- Hash-chained audit log (blake3) with CSV/JSON export
- Entity glossary with Markdown/HTML/Wiki link injection
- PCI-DSS, HIPAA, GDPR, Custom redaction profiles
- CLI, REST API, MCP server, 14 language bindings

[Unreleased]: https://github.com/jamon8888/hacienda-engine/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/jamon8888/hacienda-engine/releases/tag/v0.1.0
