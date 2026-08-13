//! MCP tool parameter types.

use rmcp::schemars;

/// Parameters for `documents_process`.
#[derive(Debug, serde::Deserialize, serde::Serialize, schemars::JsonSchema)]
pub struct DocumentsProcessParams {
    /// Files or URIs to extract and redact. Local paths and `file://` URIs, same as
    /// `hacienda extract`'s positional arguments — this tool runs in-process
    /// (`Caller::Trusted`), the same trust boundary as the CLI, not the network-facing
    /// HTTP API's base64-inline-bytes-only restriction (that restriction exists to stop
    /// a wire-supplied path from becoming an SSRF vector; there is no wire here).
    ///
    /// No per-call redaction-mode override — same contract as `POST /v1/documents`
    /// (`ProcessDocumentsRequest` has no mode field either): the server's configured
    /// mode always applies. A field that looked like it selected `mask`/`hash`/
    /// `pseudonymize` per call and silently fell back to the server's mode instead was
    /// exactly the kind of documented-but-unimplemented capability this project's own
    /// rule against silent redaction-mode substitution exists to prevent — see
    /// `hacienda-cli`'s `--mode` (no default, ever) and `RedactionConfig::default().mode`'s
    /// own doc comment for the same principle applied elsewhere.
    pub inputs: Vec<String>,
}

/// Parameters for `pii_scan` and `pii_redact`.
#[derive(Debug, serde::Deserialize, serde::Serialize, schemars::JsonSchema)]
pub struct TextParams {
    /// Raw text to scan or redact.
    pub text: String,
}

/// Parameters for `pii_reveal`.
#[derive(Debug, serde::Deserialize, serde::Serialize, schemars::JsonSchema)]
pub struct RevealParams {
    /// The pseudonym token to reverse, e.g. `[EMAIL:k1:base32...]`.
    pub token: String,
}

/// Parameters for `audit_entries`.
#[derive(Debug, serde::Deserialize, serde::Serialize, schemars::JsonSchema)]
pub struct AuditEntriesParams {
    /// Entries per page. Omitted defaults to the facade's own page size; capped the same
    /// way `GET /v1/audit/entries` caps it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
    /// An opaque cursor previously returned as `next_cursor`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cursor: Option<String>,
}

/// Empty parameters for tools that take no arguments.
///
/// This generates `{"type": "object", "properties": {}}`, which the MCP spec requires,
/// rather than what `()` would generate (`{"const": null}`) — same reasoning xberg's own
/// `EmptyParams` documents.
#[derive(Debug, serde::Deserialize, serde::Serialize, schemars::JsonSchema)]
pub struct EmptyParams {}
