//! Attachment processing using the pinned xberg extractor.
//!
//! Attachment bytes are decoded by mail-parser when the message is parsed and
//! stored in [`EmailAttachment::content`]. Each attachment is fed to xberg's
//! real `extract()` API with its MIME type and filename, so the same 96-format
//! extractor used for folder documents handles PDFs, Office files, images,
//! archives, etc. attached to emails.

use crate::error::{EmailError, Result};
use crate::models::{EmailAttachment, ExtractedAttachment};
use std::path::Path;

/// Content-free diagnostic label for a sender-controlled attachment.
///
/// Filenames arrive from the wire and can embed PII (a person's name, an SSN,
/// ...). They must never be echoed into stderr or an `EmailResponse.errors`
/// entry served by HTTP sync/job polling. This label is deliberately constant:
/// the old local FNV hash was an unkeyed pseudo-token, and the correlatable
/// opaque token is now minted by the tenant-keyed authority in `redacted.rs`
/// for persisted/output metadata. Diagnostics carry no correlation value at
/// all, so they cannot leak or be cross-tenant linked.
pub(crate) fn attachment_diagnostic_token(_filename: &str) -> &'static str {
    "attachment"
}

/// Allowlisted, content-free cause categories for an attachment failure.
///
/// The category is chosen from the error *variant*, never from extractor or PII
/// text, so it can be emitted on a public surface without re-opening the leak.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AttachmentFailureCategory {
    Extraction,
    Redaction,
    Processing,
}

impl AttachmentFailureCategory {
    /// Fixed label safe for stderr and `EmailResponse.errors`.
    pub(crate) const fn label(self) -> &'static str {
        match self {
            Self::Extraction => "attachment extraction error",
            Self::Redaction => "redaction error",
            Self::Processing => "processing error",
        }
    }

    /// The operation that failed, for operator guidance.
    pub(crate) const fn operation(self) -> &'static str {
        match self {
            Self::Extraction => "attachment text extraction",
            Self::Redaction => "attachment text redaction",
            Self::Processing => "attachment processing",
        }
    }

    /// Approved, fixed recovery suggestion — never derived from untrusted text.
    pub(crate) const fn suggestion(self) -> &'static str {
        match self {
            Self::Extraction => "retry with a supported attachment format",
            Self::Redaction => {
                "the attachment is not persisted; retry after the PII service is available"
            }
            Self::Processing => "retry the message or contact the administrator",
        }
    }
}

impl From<&EmailError> for AttachmentFailureCategory {
    fn from(err: &EmailError) -> Self {
        match err {
            EmailError::Attachment(_) => Self::Extraction,
            EmailError::Redaction(_) => Self::Redaction,
            _ => Self::Processing,
        }
    }
}

/// Structured attachment failure retained inside the crate.
///
/// The extractor's own error text is kept in `cause` for in-process diagnosis,
/// but it is deliberately excluded from both `Display` and `Debug`. Every
/// public rendering goes through [`Self::safe_message`], which only contains an
/// approved category, the failed operation and a recovery suggestion. The raw
/// cause and the sender-controlled filename must never reach stderr or HTTP.
#[derive(Clone)]
pub(crate) struct AttachmentFailure {
    category: AttachmentFailureCategory,
    cause: String,
}

impl AttachmentFailure {
    /// Map an extractor error into an approved category, retaining its text
    /// internally while keeping every public surface content-free.
    pub(crate) fn extraction(err: impl std::fmt::Display) -> Self {
        Self {
            category: AttachmentFailureCategory::Extraction,
            cause: err.to_string(),
        }
    }

    /// Test-only inspection of the approved category. Production surfaces use
    /// `safe_message`/`Display`, so this accessor would be dead code in a normal
    /// build and is restricted to tests.
    #[cfg(test)]
    pub(crate) fn category(&self) -> AttachmentFailureCategory {
        self.category
    }

    /// Test-only inspection of the internal cause. Must never be logged or
    /// returned to a client: it is untrusted extractor text that can echo the
    /// submitted filename. Gated to tests to keep non-test builds warning-free.
    #[cfg(test)]
    pub(crate) fn internal_cause(&self) -> &str {
        &self.cause
    }

    /// Fixed, content-free summary used on every public surface.
    pub(crate) fn safe_message(&self) -> String {
        format!(
            "{} failed ({}); {}",
            self.category.operation(),
            self.category.label(),
            self.category.suggestion()
        )
    }
}

impl std::fmt::Debug for AttachmentFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AttachmentFailure")
            .field("category", &self.category)
            .field("operation", &self.category.operation())
            .field("suggestion", &self.category.suggestion())
            // A byte count is content-free, so it may appear on this internal
            // diagnostic surface. Reading it here also keeps the retained cause
            // intentional in production builds (zero-warnings policy) without
            // ever emitting the extractor text itself.
            .field("cause_bytes", &self.cause.len())
            .finish_non_exhaustive()
    }
}

impl std::fmt::Display for AttachmentFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.safe_message())
    }
}

impl From<AttachmentFailure> for EmailError {
    fn from(failure: AttachmentFailure) -> Self {
        // Only the approved summary crosses the crate boundary; the retained
        // extractor cause is intentionally dropped here.
        EmailError::Attachment(failure.to_string())
    }
}

/// Fixed, content-free category for an attachment failure.
///
/// Extractor/PII error strings are deliberately not forwarded: they can quote
/// the submitted filename or redacted content, which would re-open the leak on
/// a different surface. Only the variant category is safe to emit.
pub(crate) fn attachment_error_category(err: &EmailError) -> &'static str {
    AttachmentFailureCategory::from(err).label()
}

/// The exact stderr line emitted for a failed attachment. Kept as a pure
/// function so tests can assert that the *produced log* is content-free.
pub(crate) fn attachment_failure_log(filename: &str, err: &EmailError) -> String {
    let category = AttachmentFailureCategory::from(err);
    format!(
        "Failed to process attachment {}: {} failed ({}); {}",
        attachment_diagnostic_token(filename),
        category.operation(),
        category.label(),
        category.suggestion()
    )
}

/// The exact `EmailResponse.errors` entry emitted when attachment text
/// redaction fails. Pure for the same testability reason.
pub(crate) fn attachment_redaction_error(filename: &str, err: &EmailError) -> String {
    let category = AttachmentFailureCategory::from(err);
    format!(
        "Redaction failed for attachment {} — refusing to persist ({}); {}",
        attachment_diagnostic_token(filename),
        category.label(),
        category.suggestion()
    )
}

/// Extract and process all attachments of a message via xberg.
pub async fn extract_attachments(
    message: &crate::models::EmailMessage,
) -> Result<Vec<ExtractedAttachment>> {
    let mut results = Vec::new();
    for attachment in &message.attachments {
        match process_attachment(attachment).await {
            Ok(extracted) => results.push(extracted),
            Err(e) => {
                // One bad attachment must not fail the whole message. Never log
                // the raw filename: it is sender-controlled and may carry PII.
                eprintln!("{}", attachment_failure_log(&attachment.filename, &e));
            }
        }
    }
    Ok(results)
}

/// Process a single attachment through xberg.
pub async fn process_attachment(attachment: &EmailAttachment) -> Result<ExtractedAttachment> {
    // Never trust the sender-declared Content-Type: sniff the real bytes so a
    // mislabeled payload is extracted with the correct xberg extractor.
    let mime = detect_mime_type(&attachment.content, &attachment.filename);
    let content = extract_text_with_xberg(&attachment.content, &mime, &attachment.filename).await?;

    Ok(ExtractedAttachment {
        filename: attachment.filename.clone(),
        mime_type: mime,
        content,
        metadata: serde_json::json!({
            "size": attachment.size,
            "content_id": attachment.content_id,
            "declared_mime": attachment.content_type,
            "extractor": "xberg",
        }),
    })
}

/// Extract text from attachment bytes with the pinned xberg API.
///
/// `#[cfg(feature = "attachments")]`-free: xberg is an optional dependency
/// shared with the `rag` feature, so this function is compiled whenever xberg
/// is available (i.e. when `rag` or an explicit `attachments` feature is on).
#[cfg(any(feature = "rag", feature = "attachments"))]
pub async fn extract_text_with_xberg(
    content: &[u8],
    mime_type: &str,
    filename: &str,
) -> Result<String> {
    // The inner error keeps the extractor cause internally; the conversion to
    // `EmailError` emits only its safe, content-free summary.
    extract_text_with_xberg_inner(content, mime_type, filename)
        .await
        .map_err(EmailError::from)
}

#[cfg(any(feature = "rag", feature = "attachments"))]
async fn extract_text_with_xberg_inner(
    content: &[u8],
    mime_type: &str,
    filename: &str,
) -> std::result::Result<String, AttachmentFailure> {
    use xberg::{extract, ExtractInput, ExtractionConfig};

    let input = ExtractInput::from_bytes(
        content.to_vec(),
        mime_type.to_string(),
        Some(filename.to_string()),
    );
    let config = ExtractionConfig::default();
    // xberg's Display text can echo the submitted filename. Keep it as the
    // internal cause, but let only the approved summary escape this crate.
    let output = extract(input, &config)
        .await
        .map_err(AttachmentFailure::extraction)?;

    Ok(output
        .results
        .first()
        .map(|d| d.content.clone())
        .unwrap_or_default())
}

/// Fallback when xberg is not enabled: return the decoded text/binary as UTF-8
/// lossily. This keeps the default build functional without the heavy xberg
/// dependency while the attachment feature is off.
#[cfg(not(any(feature = "rag", feature = "attachments")))]
pub async fn extract_text_with_xberg(
    content: &[u8],
    _mime_type: &str,
    _filename: &str,
) -> Result<String> {
    Ok(String::from_utf8_lossy(content).into_owned())
}

/// Save attachments to filesystem.
pub async fn save_attachments(attachments: &[ExtractedAttachment], base_path: &str) -> Result<()> {
    use std::fs;
    use std::path::PathBuf;

    let attach_dir = PathBuf::from(base_path).join("attachments");
    fs::create_dir_all(&attach_dir).map_err(EmailError::Io)?;

    for attachment in attachments {
        let safe_name = sanitize_filename(&attachment.filename);
        let file_path = attach_dir.join(&safe_name);
        fs::write(&file_path, &attachment.content).map_err(EmailError::Io)?;

        let meta_path = attach_dir.join(format!("{safe_name}.meta.json"));
        let meta_json =
            serde_json::to_string_pretty(&attachment.metadata).map_err(EmailError::Serde)?;
        fs::write(&meta_path, meta_json).map_err(EmailError::Io)?;
    }

    Ok(())
}

/// Sanitize filename for filesystem.
fn sanitize_filename(filename: &str) -> String {
    filename
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '.' || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .take(200)
        .collect()
}

/// Detect MIME type from content/extension.
pub fn detect_mime_type(content: &[u8], filename: &str) -> String {
    // Content sniffing wins over the sender-declared Content-Type (untrusted)
    // and over the filename extension. A crafted "invoice.pdf" that is really
    // an executable is treated as what it actually is.
    #[cfg(any(feature = "rag", feature = "attachments"))]
    if let Ok(detected) = xberg::detect_mime_type_from_bytes(content) {
        if !detected.is_empty() && detected != "application/octet-stream" {
            return detected;
        }
    }
    // Fallback: extension-based guess (still better than trusting the header).
    mime_guess::from_path(filename)
        .first()
        .map(|m| m.to_string())
        .unwrap_or_else(|| "application/octet-stream".to_string())
}

/// Resolve a path reference (kept for API compatibility).
pub fn as_path(path: &str) -> &Path {
    Path::new(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Sentinel sender-controlled filename: it embeds PII in several shapes.
    /// Any of these fragments reaching an error or log surface is a leak.
    const SENTINEL_FILENAME: &str = "Contrat-Jane-Doe-SSN-123-45-6789-2026.pdf";
    const PII_FRAGMENTS: [&str; 4] = [SENTINEL_FILENAME, "Jane-Doe", "Jane Doe", "123-45-6789"];

    fn assert_content_free(surface: &str, label: &str) {
        for fragment in PII_FRAGMENTS {
            assert!(
                !surface.contains(fragment),
                "{label} leaked raw attachment name fragment {fragment:?}: {surface}"
            );
        }
        assert!(
            surface.contains(attachment_diagnostic_token(SENTINEL_FILENAME)),
            "{label} lost its content-free diagnostic label: {surface}"
        );
    }

    /// Regression for the stderr leak: the underlying extractor error echoes
    /// the sentinel name, but the log actually produced must not.
    #[test]
    fn attachment_failure_log_never_prints_raw_filename() {
        let failing =
            EmailError::Attachment(format!("xberg extraction failed for {SENTINEL_FILENAME}"));
        let produced_log = attachment_failure_log(SENTINEL_FILENAME, &failing);
        assert_content_free(&produced_log, "stderr log");
    }

    /// Regression for the `EmailResponse.errors` leak consumed by HTTP sync/job
    /// polling: a sentinel filename failing during redaction must not appear.
    #[test]
    fn attachment_redaction_error_never_returns_raw_filename() {
        let failing = EmailError::Redaction(format!(
            "redaction failed for attachment {SENTINEL_FILENAME}"
        ));
        let error_entry = attachment_redaction_error(SENTINEL_FILENAME, &failing);
        assert_content_free(&error_entry, "EmailResponse.errors entry");
    }

    /// Diagnostics carry no derivation of the clear name: no local hash, no
    /// local secret, and no value that could link two tenants' filenames.
    #[test]
    fn diagnostic_tokens_are_constant_and_content_free() {
        let first = attachment_diagnostic_token(SENTINEL_FILENAME);
        assert_eq!(first, attachment_diagnostic_token(SENTINEL_FILENAME));
        assert_eq!(first, attachment_diagnostic_token("other.pdf"));
        assert!(!first.contains("Jane"));
        assert!(!first.contains("123-45-6789"));
        assert!(!first.contains("pdf"));
    }

    /// The extractor cause is retained in-process for diagnosis, but Display,
    /// Debug and the public `EmailError` carry only the approved safe summary.
    #[test]
    fn attachment_failure_retains_cause_but_displays_only_safe_summary() {
        let raw = format!("xberg could not decode {SENTINEL_FILENAME}");
        let failure = AttachmentFailure::extraction(raw.as_str());

        // Internal retention: the cause is still inspectable inside the crate.
        assert!(failure.internal_cause().contains(SENTINEL_FILENAME));
        assert_eq!(failure.category(), AttachmentFailureCategory::Extraction);

        let category = AttachmentFailureCategory::Extraction;
        let display = failure.to_string();
        assert!(display.contains(category.operation()));
        assert!(display.contains(category.label()));
        assert!(display.contains(category.suggestion()));
        assert!(!display.contains(SENTINEL_FILENAME));
        assert!(!display.contains("xberg"));

        // Debug must not become a back-channel for the raw extractor text.
        // Only its content-free byte count is retained on this surface.
        let debug = format!("{failure:?}");
        assert!(!debug.contains(SENTINEL_FILENAME));
        assert!(!debug.contains("xberg"));
        assert!(debug.contains("cause_bytes"));
        assert!(debug.contains(&raw.len().to_string()));

        // Crossing the crate boundary keeps only the safe summary.
        let public = EmailError::from(failure);
        let public_display = public.to_string();
        assert!(!public_display.contains(SENTINEL_FILENAME));
        assert!(public_display.contains(category.suggestion()));
    }

    /// Logs and `EmailResponse.errors` entries carry operation/recovery
    /// guidance from the allowlisted category, never from untrusted text.
    #[test]
    fn attachment_surfaces_carry_operation_and_recovery_guidance() {
        let extraction = EmailError::Attachment("ignored extractor text".to_string());
        let log = attachment_failure_log(SENTINEL_FILENAME, &extraction);
        let category = AttachmentFailureCategory::Extraction;
        assert!(log.contains(category.operation()), "{log}");
        assert!(log.contains(category.label()), "{log}");
        assert!(log.contains(category.suggestion()), "{log}");
        assert_content_free(&log, "stderr log");

        let redaction = EmailError::Redaction("ignored PII text".to_string());
        let http = attachment_redaction_error(SENTINEL_FILENAME, &redaction);
        let category = AttachmentFailureCategory::Redaction;
        assert!(http.contains(category.label()), "{http}");
        assert!(http.contains(category.suggestion()), "{http}");
        assert_content_free(&http, "EmailResponse.errors entry");
    }
}
