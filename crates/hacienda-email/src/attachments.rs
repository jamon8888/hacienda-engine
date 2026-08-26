use crate::error::{EmailError, Result};
use crate::models::{EmailAttachment, ExtractedAttachment};
use std::path::Path;

/// Extract and process email attachments using xberg
pub async fn extract_attachments(
    message: &crate::models::EmailMessage,
) -> Result<Vec<ExtractedAttachment>> {
    let mut results = Vec::new();

    for attachment in &message.attachments {
        match process_attachment(attachment).await {
            Ok(extracted) => results.push(extracted),
            Err(e) => {
                // Log error but continue with other attachments
                eprintln!("Failed to process attachment {}: {}", attachment.filename, e);
            }
        }
    }

    Ok(results)
}

/// Process a single attachment
async fn process_attachment(attachment: &EmailAttachment) -> Result<ExtractedAttachment> {
    // In a real implementation, this would:
    // 1. Decode the attachment content (base64, quoted-printable, etc.)
    // 2. Determine MIME type
    // 3. Use xberg to extract text and metadata
    // 4. Return structured result

    // For now, return placeholder
    Ok(ExtractedAttachment {
        filename: attachment.filename.clone(),
        mime_type: attachment.content_type.clone(),
        content: format!("[Attachment: {} - {} bytes]", attachment.filename, attachment.size),
        metadata: serde_json::json!({
            "size": attachment.size,
            "content_id": attachment.content_id,
        }),
    })
}

/// Save attachments to filesystem
pub async fn save_attachments(
    attachments: &[ExtractedAttachment],
    base_path: &str,
) -> Result<()> {
    use std::fs;
    use std::path::PathBuf;

    let attach_dir = PathBuf::from(base_path).join("attachments");
    fs::create_dir_all(&attach_dir)
        .map_err(|e| EmailError::Io(e))?;

    for attachment in attachments {
        let safe_name = sanitize_filename(&attachment.filename);
        let file_path = attach_dir.join(&safe_name);

        // Save extracted content
        fs::write(&file_path, &attachment.content)
            .map_err(|e| EmailError::Io(e))?;

        // Save metadata
        let meta_path = attach_dir.join(format!("{}.meta.json", safe_name));
        let meta_json = serde_json::to_string_pretty(&attachment.metadata)
            .map_err(|e| EmailError::Serde(e))?;
        fs::write(&meta_path, meta_json)
            .map_err(|e| EmailError::Io(e))?;
    }

    Ok(())
}

/// Sanitize filename for filesystem
fn sanitize_filename(filename: &str) -> String {
    filename.chars()
        .map(|c| if c.is_alphanumeric() || c == '.' || c == '-' || c == '_' { c } else { '_' })
        .collect::<String>()
        .chars()
        .take(200)
        .collect::<String>()
}

/// Detect MIME type from content
pub fn detect_mime_type(content: &[u8], filename: &str) -> String {
    // Use mime_guess or file extension
    mime_guess::from_path(filename)
        .first()
        .map(|m| m.to_string())
        .unwrap_or_else(|| "application/octet-stream".to_string())
}

/// Extract text from attachment using xberg
#[cfg(feature = "rag")]
pub async fn extract_text_with_xberg(
    content: &[u8],
    mime_type: &str,
) -> Result<String> {
    use xberg::{Extractor, ExtractInput, MimeType};

    let mime = MimeType::parse(mime_type)
        .map_err(|e| EmailError::Attachment(format!("Invalid MIME type: {}", e)))?;

    let input = ExtractInput::from_bytes(content, mime);
    let extractor = Extractor::new()
        .map_err(|e| EmailError::Attachment(format!("Failed to create extractor: {}", e)))?;

    let extracted = extractor.extract(input).await
        .map_err(|e| EmailError::Attachment(format!("Extraction failed: {}", e)))?;

    Ok(extracted.text)
}
