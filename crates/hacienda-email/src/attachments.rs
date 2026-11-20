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

/// Extract and process all attachments of a message via xberg.
pub async fn extract_attachments(
    message: &crate::models::EmailMessage,
) -> Result<Vec<ExtractedAttachment>> {
    let mut results = Vec::new();
    for attachment in &message.attachments {
        match process_attachment(attachment).await {
            Ok(extracted) => results.push(extracted),
            Err(e) => {
                // One bad attachment must not fail the whole message.
                eprintln!(
                    "Failed to process attachment {}: {}",
                    attachment.filename, e
                );
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
    use xberg::{extract, ExtractInput, ExtractionConfig};

    let input = ExtractInput::from_bytes(
        content.to_vec(),
        mime_type.to_string(),
        Some(filename.to_string()),
    );
    let config = ExtractionConfig::default();
    let output = extract(input, &config)
        .await
        .map_err(|e| EmailError::Attachment(format!("xberg extraction failed: {e}")))?;

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
