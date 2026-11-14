# Hacienda Secrecy Client

This is the focused Tauri shell for the local secrecy client. The provider seam lives
in [`crates/hacienda-client-core`](../../crates/hacienda-client-core/src/provider.rs):
Claude Desktop and Codex requests cross a required redaction adapter before transport,
and streamed chunks retain opaque token references.

Rehydration is intentionally not a provider or MCP operation. A renderer may request
one display-only chunk through an explicit viewer capability; without both the
capability and a rehydration adapter the operation fails closed. The shell does not
persist transcripts or log prompt/response content.

The Tauri runtime dependencies are declared in `src-tauri/Cargo.toml` but are not added
to the repository workspace yet. This keeps the foundation's targeted Rust tests
dependency-light until platform packaging and signing are selected.
