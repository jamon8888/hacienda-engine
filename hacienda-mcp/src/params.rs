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
    pub inputs: Vec<String>,
    /// Redaction mode: `mask`, `hash`, or `pseudonymize`. Omit to use the server's
    /// configured default. `pseudonymize` requires the server to have been started with
    /// a pseudonymisation key configured.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mode: Option<String>,
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
