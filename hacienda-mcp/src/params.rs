//! MCP tool parameter types.

use rmcp::schemars;
use serde::de::Error as _;

/// A document input accepted by the guarded MCP extraction tool.
///
/// The string form remains supported for existing stdio clients and is interpreted as a
/// URI. The structured form mirrors xberg's bytes/URI input shape (v1.4.0 MCP contract), while deliberately
/// omitting xberg's per-input extraction config so a caller cannot alter Hacienda's redaction
/// policy for one request.
///
/// `Deserialize` is implemented by hand instead of derived: serde's untagged-enum fallback
/// masks the inner schema error, so a hand-written implementation lets an unknown policy
/// field (xberg's per-input `config`, a PDF password, a response-format switch) surface in
/// the error as `unknown field ...` rather than a generic `did not match any variant`.
#[derive(Debug, serde::Serialize, schemars::JsonSchema)]
#[serde(untagged)]
pub enum DocumentInput {
    /// Existing shorthand for a local path or URI.
    Uri(String),
    /// A structured bytes or URI input.
    Structured(StructuredDocumentInput),
}

impl<'de> serde::Deserialize<'de> for DocumentInput {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = serde_json::Value::deserialize(deserializer)?;
        match value {
            serde_json::Value::String(uri) => Ok(Self::Uri(uri)),
            object => serde_json::from_value::<StructuredDocumentInput>(object)
                .map(Self::Structured)
                .map_err(|err| D::Error::custom(format!("invalid document input: {err}"))),
        }
    }
}

impl DocumentInput {
    /// Convert a validated MCP input into xberg's unified extraction input.
    pub fn into_extract_input(self) -> Result<xberg::ExtractInput, &'static str> {
        match self {
            Self::Uri(uri) => Ok(xberg::ExtractInput::from_uri(uri)),
            Self::Structured(input) => input.into_extract_input(),
        }
    }
}

/// Structured input fields exposed by `documents_process`.
#[derive(Debug, serde::Deserialize, serde::Serialize, schemars::JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum StructuredDocumentInput {
    /// In-memory document bytes. A MIME type is required to avoid guessing a parser from
    /// untrusted content when a caller has already supplied the bytes.
    Bytes {
        bytes: Vec<u8>,
        mime_type: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        filename: Option<String>,
    },
    /// A local path, `file://` URI, or HTTP(S) URL with optional detection hints.
    Uri {
        uri: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        mime_type: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        filename: Option<String>,
    },
}

impl StructuredDocumentInput {
    fn into_extract_input(self) -> Result<xberg::ExtractInput, &'static str> {
        match self {
            Self::Bytes {
                bytes,
                mime_type,
                filename,
            } => {
                if mime_type.trim().is_empty() {
                    return Err("`mime_type` must not be empty for byte inputs");
                }
                Ok(xberg::ExtractInput::from_bytes(bytes, mime_type, filename))
            }
            Self::Uri {
                uri,
                mime_type,
                filename,
            } => {
                if uri.trim().is_empty() {
                    return Err("`uri` must not be empty");
                }
                let mut input = xberg::ExtractInput::from_uri(uri);
                input.mime_type = mime_type;
                input.filename = filename;
                Ok(input)
            }
        }
    }
}

/// Parameters for `documents_process`.
#[derive(Debug, serde::Deserialize, serde::Serialize, schemars::JsonSchema)]
pub struct DocumentsProcessParams {
    /// Documents to extract and redact. Strings remain local paths or `file://` URIs, same as
    /// `hacienda extract`'s positional arguments. Structured inputs additionally support
    /// in-memory bytes with filename and MIME hints, matching xberg's MCP input contract.
    /// This tool runs in-process
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
    pub inputs: Vec<DocumentInput>,
}

/// Parameters for `pii_scan` and `pii_redact`.
#[derive(Debug, serde::Deserialize, serde::Serialize, schemars::JsonSchema)]
pub struct TextParams {
    /// Raw text to scan or redact.
    pub text: String,
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

#[derive(Debug, serde::Deserialize, serde::Serialize, schemars::JsonSchema)]
pub struct MimeDetectParams {
    pub content_base64: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub filename: Option<String>,
}

#[derive(Debug, serde::Deserialize, serde::Serialize, schemars::JsonSchema)]
pub struct CacheConfirmParams {
    pub confirm: bool,
}

#[derive(Debug, serde::Deserialize, serde::Serialize, schemars::JsonSchema)]
pub struct CacheWarmParams {
    pub preset: String,
    pub confirm: bool,
}
