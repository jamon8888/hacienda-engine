//! MCP tool parameter types.

use std::path::Path;

use rmcp::schemars;
use serde::de::Error as _;

/// Content read from a local file to validate a URI input's MIME hint, bounded by the same
/// limit hacienda-core's own `detect_mime_type` applies — validating a hint costs one
/// bounded read of a local document, never a whole-document load in the tool handler.
/// A larger file is left to xberg's own detection, exactly as before.
const URI_CONTENT_SNIFF_BYTES: u64 = hacienda_core::xberg_surface::MAX_MIME_INPUT_BYTES as u64;

/// One `documents_process` input failing conversion into an extraction input.
///
/// Carries the offending field, the root cause, and a repair suggestion — never the
/// document content itself. `documents_process` prefixes the failing input's index, so a
/// batch caller can tell which document was refused.
#[derive(Debug, thiserror::Error)]
pub enum DocumentInputError {
    /// A byte input arrived without a usable MIME type.
    #[error(
        "`mime_type` must not be empty for byte inputs; suggestion: pass the document's MIME type, e.g. `application/pdf`"
    )]
    EmptyMimeType,

    /// A URI input arrived without a usable URI.
    #[error(
        "`uri` must not be empty; suggestion: pass a local path, `file://` URI, or HTTP(S) URL"
    )]
    EmptyUri,

    /// The declared MIME contradicts the signature detected from the content.
    #[error(
        "`mime_type` `{declared}` conflicts with the content signature (detected `{detected}`); suggestion: pass the detected MIME type, or omit `mime_type` to let content detection decide"
    )]
    MimeTypeConflict { declared: String, detected: String },

    /// Content detection rejected the bytes, so no declared MIME can be verified.
    #[error(
        "the bytes are not a recognizable document format, so `mime_type` `{declared}` cannot be verified; suggestion: pass bytes of a supported document whose MIME type matches its content"
    )]
    MimeTypeUnverifiable { declared: String },
}

/// Why a declared MIME hint could not be reconciled with the content it describes.
enum MimeHintConflict {
    /// The content carries a concrete signature that differs from the hint.
    Conflict { declared: String, detected: String },
    /// The content carries no recognizable signature at all.
    Unverifiable { declared: String },
}

/// A declared MIME hint reduced to its comparable essence: lowercased, whitespace-free,
/// parameters (`; charset=utf-8`) stripped — the same normalization xberg's own
/// `validate_mime_type` applies before its registry lookup.
fn normalized_mime_hint(mime_type: &str) -> String {
    mime_type
        .split(';')
        .next()
        .unwrap_or(mime_type)
        .trim()
        .to_ascii_lowercase()
}

/// Validate a caller-declared MIME hint against the content it claims to describe.
///
/// xberg's byte extractor treats an explicit, non-generic MIME as authoritative and skips
/// content detection entirely (`resolve_owned_bytes_mime` validates it against the
/// registry and returns it, even under `ContentOnly`), so a lying hint silently routes
/// bytes to the wrong extractor — a PDF sent as `text/plain` never gets its signature
/// checked. The hint must therefore agree with a signature detected from the content
/// itself: the same content-over-hint rule hacienda-core's `detect_mime_type` applies to
/// filename hints.
fn validate_mime_hint_against_content(
    bytes: &[u8],
    declared: &str,
) -> Result<(), MimeHintConflict> {
    // An explicitly generic hint claims nothing; xberg already ignores it and falls
    // back to content detection, so it is not this function's to police.
    if declared == "application/octet-stream" {
        return Ok(());
    }
    match xberg::detect_mime_type_from_bytes(bytes) {
        Ok(detected) if detected == declared => Ok(()),
        // A plain-text payload carries no magic-byte signature: `text/plain` is the
        // content baseline for markdown, CSV, and every other textual format, exactly
        // as xberg's own `prefer_content_mime` lets a textual hint win over generic
        // text content. Any non-textual hint must match detection byte for byte —
        // `text/plain` on real PDF bytes is precisely the lie that must fail here.
        Ok(detected) if detected == "text/plain" && declared.starts_with("text/") => Ok(()),
        Ok(detected) => Err(MimeHintConflict::Conflict {
            declared: declared.to_string(),
            detected,
        }),
        Err(_) => Err(MimeHintConflict::Unverifiable {
            declared: declared.to_string(),
        }),
    }
}

impl MimeHintConflict {
    fn into_error(self) -> DocumentInputError {
        match self {
            Self::Conflict { declared, detected } => {
                DocumentInputError::MimeTypeConflict { declared, detected }
            }
            Self::Unverifiable { declared } => {
                DocumentInputError::MimeTypeUnverifiable { declared }
            }
        }
    }
}

/// The local path a URI input points at, when this process can read the content itself:
/// a bare path or a `file://` URI. A remote URL (or any other scheme) is fetched by
/// xberg downstream — past this validation point — so its hint is passed through
/// unverified.
fn local_uri_path(uri: &str) -> Option<&Path> {
    let path = uri.strip_prefix("file://").unwrap_or(uri);
    if path.contains("://") {
        return None;
    }
    Some(Path::new(path))
}

/// Read a local document's content for MIME-hint validation, bounded by
/// [`URI_CONTENT_SNIFF_BYTES`]. A file larger than that bound — or unreadable — yields
/// `None`: the hint rides along to xberg, whose extraction error will be the honest one.
fn local_uri_content(uri: &str) -> Option<Vec<u8>> {
    let path = local_uri_path(uri)?;
    let metadata = std::fs::metadata(path).ok()?;
    if !metadata.is_file() || metadata.len() > URI_CONTENT_SNIFF_BYTES {
        return None;
    }
    std::fs::read(path).ok()
}

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
    pub fn into_extract_input(self) -> Result<xberg::ExtractInput, DocumentInputError> {
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
    fn into_extract_input(self) -> Result<xberg::ExtractInput, DocumentInputError> {
        match self {
            Self::Bytes {
                bytes,
                mime_type,
                filename,
            } => {
                let declared = normalized_mime_hint(&mime_type);
                if declared.is_empty() {
                    return Err(DocumentInputError::EmptyMimeType);
                }
                validate_mime_hint_against_content(&bytes, &declared)
                    .map_err(MimeHintConflict::into_error)?;
                Ok(xberg::ExtractInput::from_bytes(bytes, mime_type, filename))
            }
            Self::Uri {
                uri,
                mime_type,
                filename,
            } => {
                if uri.trim().is_empty() {
                    return Err(DocumentInputError::EmptyUri);
                }
                // The same content-over-hint rule as for bytes, but only where the
                // content is actually here to check: a local path. A remote URL's
                // bytes are fetched by xberg itself, after this point.
                if let Some(mime) = mime_type.as_deref() {
                    let declared = normalized_mime_hint(mime);
                    if !declared.is_empty() {
                        if let Some(content) = local_uri_content(&uri) {
                            validate_mime_hint_against_content(&content, &declared)
                                .map_err(MimeHintConflict::into_error)?;
                        }
                    }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn bytes_input(mime_type: &str, bytes: &[u8]) -> DocumentInput {
        serde_json::from_value(serde_json::json!({
            "kind": "bytes",
            "bytes": bytes,
            "mime_type": mime_type,
        }))
        .expect("structured bytes input deserializes")
    }

    /// CodeRabbit follow-up (#157): PDF bytes carrying a lying `text/plain` hint. xberg
    /// treats an explicit non-generic MIME as authoritative and never detects the
    /// content, so the hint pair is only rejected if checked here.
    #[test]
    fn bytes_input_rejects_a_mime_hint_that_contradicts_the_content() {
        let error = bytes_input("text/plain", b"%PDF-1.7\n")
            .into_extract_input()
            .expect_err("a lying MIME hint must be rejected before extraction");
        assert!(
            matches!(
                error,
                DocumentInputError::MimeTypeConflict { ref declared, ref detected }
                    if declared == "text/plain" && detected == "application/pdf"
            ),
            "{error}"
        );
    }

    /// The same lie from the other direction: a hint naming a signature the bytes do
    /// not carry must not select that extractor.
    #[test]
    fn bytes_input_rejects_a_mime_hint_the_content_does_not_carry() {
        let error = bytes_input("application/pdf", b"ordinary prose, no signature at all\n")
            .into_extract_input()
            .expect_err("a MIME hint without its signature must be rejected");
        assert!(
            matches!(error, DocumentInputError::MimeTypeConflict { .. }),
            "{error}"
        );
    }

    /// Content detection cannot type the bytes at all, so no hint is provable and the
    /// input fails closed rather than trusting the caller.
    #[test]
    fn bytes_input_fails_closed_when_detection_cannot_type_the_content() {
        let error = bytes_input("application/pdf", &[0, 1, 2, 3, 252, 253, 254, 255])
            .into_extract_input()
            .expect_err("unverifiable content must fail closed");
        assert!(
            matches!(error, DocumentInputError::MimeTypeUnverifiable { .. }),
            "{error}"
        );
    }

    /// A hint that agrees with the content still rides along untouched, and a textual
    /// hint remains acceptable for a signature-less text payload.
    #[test]
    fn bytes_input_keeps_a_consistent_mime_hint() {
        let input = bytes_input("application/pdf", b"%PDF-1.7\n")
            .into_extract_input()
            .expect("a matching MIME hint passes through");
        assert_eq!(input.mime_type.as_deref(), Some("application/pdf"));

        let input = bytes_input("text/markdown", b"# Title\n\nprose\n")
            .into_extract_input()
            .expect("a textual hint on a text payload passes through");
        assert_eq!(input.mime_type.as_deref(), Some("text/markdown"));
    }

    /// A parameterized or oddly-cased hint normalizes to its essence before comparison,
    /// so `Text/Plain; charset=UTF-8` on a text payload is not a false conflict.
    #[test]
    fn bytes_input_normalizes_a_parameterized_mime_hint() {
        let input = bytes_input("Text/Plain; charset=UTF-8", b"plain prose\n")
            .into_extract_input()
            .expect("a parameterized textual hint passes through");
        assert_eq!(
            input.mime_type.as_deref(),
            Some("Text/Plain; charset=UTF-8")
        );
    }

    /// An explicitly generic hint claims nothing; xberg already ignores it in favor of
    /// content detection, so validation must not reject it either.
    #[test]
    fn bytes_input_passes_an_explicitly_generic_mime_hint_through() {
        let input = bytes_input("application/octet-stream", b"anything at all\n")
            .into_extract_input()
            .expect("a generic hint is left to content detection");
        assert_eq!(input.mime_type.as_deref(), Some("application/octet-stream"));
    }

    /// A hint on a local URI input is checked against the file's content — the same
    /// control, the same rejection, as for in-memory bytes.
    #[test]
    fn uri_input_rejects_a_mime_hint_that_contradicts_the_file_content() {
        let file = tempfile::NamedTempFile::new().expect("temp file");
        std::fs::write(file.path(), b"%PDF-1.7\n").expect("write fixture");

        let input: DocumentInput = serde_json::from_value(serde_json::json!({
            "kind": "uri",
            "uri": file.path().to_string_lossy(),
            "mime_type": "text/plain"
        }))
        .expect("structured URI input");

        let error = input
            .into_extract_input()
            .expect_err("a lying MIME hint on a local file must be rejected");
        assert!(
            matches!(
                error,
                DocumentInputError::MimeTypeConflict { ref declared, .. } if declared == "text/plain"
            ),
            "{error}"
        );
    }

    /// A hint on a remote URL cannot be checked where the URL is fetched — it stays
    /// xberg's business, and the hint rides along unchanged.
    #[test]
    fn uri_input_passes_a_remote_mime_hint_through_unverified() {
        let input: DocumentInput = serde_json::from_value(serde_json::json!({
            "kind": "uri",
            "uri": "https://example.invalid/document.bin",
            "mime_type": "text/plain"
        }))
        .expect("structured URI input");

        let input = input
            .into_extract_input()
            .expect("a remote URL's hint is xberg's to resolve");
        assert_eq!(input.mime_type.as_deref(), Some("text/plain"));
    }

    /// A hint on a file too large to bound-read is passed through: the validation is
    /// best-effort by size, and xberg's own extraction error is the honest one.
    #[test]
    fn uri_input_passes_a_mime_hint_through_for_an_unreadable_target() {
        let input: DocumentInput = serde_json::from_value(serde_json::json!({
            "kind": "uri",
            "uri": "/nonexistent/document.bin",
            "mime_type": "text/plain"
        }))
        .expect("structured URI input");

        let input = input
            .into_extract_input()
            .expect("an unreadable file's hint is left to xberg");
        assert_eq!(input.mime_type.as_deref(), Some("text/plain"));
    }
}

/// Parameters for `email_redact`.
#[derive(Debug, serde::Deserialize, serde::Serialize, schemars::JsonSchema)]
pub struct EmailRedactParams {
    /// Local Maildir path to read emails from.
    pub source: String,
    /// Directory where redacted emails are written.
    pub output: String,
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
