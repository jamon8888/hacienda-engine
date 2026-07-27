---
description: Plugin trait system and Python FFI integration
---

- Core traits: Extractor, PostProcessor, MetadataExtractor — each with async extract/process methods returning Result
- Discovery: static registration (Rust plugins compiled in) + dynamic discovery (Python plugins via PyO3 FFI)
- Priority selection: plugins declare priority per MIME type, registry selects highest-priority match, fallback to next
- Registry: PluginRegistry holds all discovered plugins, provides lookup by MIME type, supports hot-reload for Python plugins
- Python FFI: Python plugins implement a Python class matching the trait interface, called via PyO3 with GIL management
- GIL management: acquire GIL only for Python calls, release immediately after, use py.allow_threads() for Rust-side work
- Plugin lifecycle: init → register → validate → ready. Plugins validate their dependencies (e.g., Tesseract binary, Python packages) at startup
- Error handling: plugin errors are wrapped in PluginError with source plugin name, converted to ExtractionError at boundary
- Testing: test plugins with real files (not mocks), test fallback chains, test Python plugin loading/unloading
