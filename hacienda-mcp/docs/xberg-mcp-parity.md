# Hacienda MCP and official xberg (v1.4.0)

This crate implements a Hacienda-owned MCP server. It does not enable xberg's native `mcp`
feature: native xberg extraction returns raw document content and does not pass through
`HaciendaFacade`'s PII detection, redaction, audit, or authorization boundary.

The matrix below is the deliberately safe subset of the official xberg v1.4.0 MCP contract
(pinned at rev `507fca3d3cc5b8e71f0e416af4a8aec15e18d2cd`, tracked by issue #139). The shared
build/runtime capability report is returned to MCP clients by `get_capabilities`.

## Capability matrix

| Official xberg capability | Hacienda MCP status | Decision |
| --- | --- | --- |
| `extract`, `extract_batch` | `documents_process` | Enabled through `HaciendaFacade`; outputs are redacted and audited. URI shorthand remains compatible; structured `bytes`, `filename`, and `mime_type` inputs are supported. |
| `get_version` | `get_version` | Enabled; reports Hacienda's version. |
| `detect_mime_type` | `detect_mime_type` | Enabled behind `hacienda_core::xberg_surface`; inline base64 bytes only, never a wire-supplied path. |
| `list_formats` | `list_formats` | Enabled; static catalogue from `hacienda_core::xberg_surface::supported_formats()`. |
| `cache_stats` | `cache_stats` | Enabled; sanitized statistics without filesystem paths. |
| `cache_clear` | `cache_clear` | Enabled; requires `confirm=true`, mirroring the REST route. |
| `cache_warm` | `cache_warm` | Enabled; requires `confirm=true`, mirroring the REST route. |
| `model_manifest` (1.4.0) | Disabled | Model file lists could reveal host/runtime state; not a Hacienda contract. |
| xberg prompts | Disabled | No Hacienda-owned prompt contract exists yet; no raw xberg workflow prompt is imported. |
| xberg resources | Disabled | No static Hacienda resource contract exists yet. |
| completions | Disabled | There is no completion provider behind `HaciendaFacade`. |
| tasks | Disabled | Hacienda's job store is in-memory and MCP task lifecycle semantics are not implemented. |
| Streamable HTTP transport | Disabled | `rmcp` support alone does not authenticate calls or propagate an API `Caller`; stdio remains the only transport. |
| native xberg MCP server | Disabled | Enabling `xberg/mcp` would create an unredacted bypass around `HaciendaFacade`. |

## Input and policy boundary

`documents_process` accepts the existing string form (interpreted as a local path or URI) and
the xberg-shaped structured form:

- `{"kind":"bytes","bytes":[...],"mime_type":"...","filename":"..."}`;
- `{"kind":"uri","uri":"...","mime_type":"...","filename":"..."}`.

Per-call xberg extraction configuration, PDF passwords, and response-format switches remain
absent. They are not silently accepted or ignored: a structured input containing an unknown
policy field is rejected by its schema (`deny_unknown_fields`). The server's configured
redaction mode remains the only redaction policy for the call.

## rmcp version decision

The official xberg v1.4.0 manifest declares `rmcp = 3.10.0` with `server`, `macros`, `base64`,
and `transport-io`. Hacienda uses `rmcp = 3.10.0` with only `server`, `macros`, and
`transport-io`: base64 is not needed because MCP byte inputs use the JSON array form already
defined by xberg's input schema. HTTP transport features are intentionally not enabled.

This keeps the protocol model on the same SDK generation as official xberg without importing
the native xberg server or expanding Hacienda's trust boundary.

References:

- [official xberg v1.4.0 MCP server](https://github.com/xberg-io/xberg/blob/v1.4.0/crates/xberg/src/mcp/server.rs)
- [official xberg v1.4.0 MCP parameters](https://github.com/xberg-io/xberg/blob/v1.4.0/crates/xberg/src/mcp/params.rs)
- [official xberg v1.4.0 manifest](https://github.com/xberg-io/xberg/blob/v1.4.0/crates/xberg/Cargo.toml)
- [rmcp Streamable HTTP service](https://docs.rs/rmcp/latest/rmcp/transport/streamable_http_server/tower/struct.StreamableHttpService.html)
