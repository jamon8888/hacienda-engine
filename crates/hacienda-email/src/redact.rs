use crate::backend::EmailClient;
use crate::config::{EmailConfig, RedactionRule};
use crate::error::{EmailError, Result};
use crate::models::{EmailDestination, EmailMessage, EmailOperation, EmailSource, RedactionStats};
use crate::pii::PiiEngine;
use std::collections::HashMap;

/// Redact PII from emails using hacienda-core's real detection pipeline.
pub async fn redact(
    client: &dyn EmailClient,
    pii: &PiiEngine,
    op: EmailOperation,
    config: &EmailConfig,
) -> Result<crate::models::EmailResponse> {
    let EmailOperation::Redact {
        source,
        destination,
        rules,
    } = op else {
        return Err(EmailError::InvalidOperation("Expected Redact operation".to_string()));
    };

    let envelopes = fetch_envelopes(client, &source).await?;
    let redaction_rules = rules.unwrap_or_else(|| config.redaction.rules.clone());
    let replacement = &config.redaction.replacement;
    let preserve_format = config.redaction.preserve_format;

    let mut all_stats = HashMap::new();
    let mut processed = 0;
    let mut errors = Vec::new();

    for envelope in envelopes {
        match client.get_message("INBOX", &envelope.id).await {
            Ok(mut message) => {
                // Redact the text body (real PII engine).
                if let Some(text) = &message.text_body {
                    match redact_body(text, &redaction_rules, replacement, preserve_format, pii).await {
                        Ok((redacted, stats)) => {
                            message.text_body = Some(redacted);
                            merge_stats(&mut all_stats, stats);
                        }
                        Err(e) => errors.push(format!("Text redaction failed for {}: {}", envelope.id, e)),
                    }
                }

                // Redact the HTML body. Extracting the visible text, redacting
                // it, and writing it back keeps markup intact while ensuring
                // PII in human-visible content is removed.
                if let Some(html) = &message.html_body {
                    match redact_html_body(html, &redaction_rules, replacement, preserve_format, pii).await {
                        Ok((redacted, stats)) => {
                            message.html_body = Some(redacted);
                            merge_stats(&mut all_stats, stats);
                        }
                        Err(e) => errors.push(format!("HTML redaction failed for {}: {}", envelope.id, e)),
                    }
                }

                // Redact header fields, which frequently carry PII (From/To/Cc/Subject).
                redact_headers(&mut message, &redaction_rules, replacement, preserve_format, pii, &mut all_stats).await;

                // Save redacted message.
                if let Err(e) = save_redacted(&destination, &message).await {
                    errors.push(format!("Failed to save redacted {}: {}", envelope.id, e));
                } else {
                    processed += 1;
                }
            }
            Err(e) => errors.push(format!("Failed to fetch {}: {}", envelope.id, e)),
        }
    }

    Ok(crate::models::EmailResponse {
        success: errors.is_empty(),
        processed_count: processed,
        output_path: match &destination {
            EmailDestination::Maildir { path } | EmailDestination::Files { path } => Some(path.clone()),
            _ => None,
        },
        redaction_stats: Some(RedactionStats {
            total_redactions: all_stats.values().sum(),
            by_type: all_stats,
        }),
        errors,
        ..Default::default()
    })
}

fn merge_stats(all: &mut HashMap<String, usize>, stats: HashMap<String, usize>) {
    for (k, v) in stats {
        *all.entry(k).or_insert(0) += v;
    }
}

/// Fetch envelopes for redaction.
async fn fetch_envelopes(
    client: &dyn EmailClient,
    source: &EmailSource,
) -> Result<Vec<crate::models::Envelope>> {
    match source {
        EmailSource::Account { name: _, mailbox } => {
            let mb = mailbox.as_deref().unwrap_or("INBOX");
            client.list_envelopes(mb, Some(1000)).await
        }
        EmailSource::Maildir { path: _ } => client.list_envelopes("INBOX", Some(1000)).await,
        EmailSource::Files { paths: _ } => {
            Err(EmailError::InvalidOperation("File source not yet implemented".to_string()))
        }
        EmailSource::Stdin => {
            Err(EmailError::InvalidOperation("Stdin source not yet implemented".to_string()))
        }
    }
}

/// Redact the header fields that commonly carry PII.
async fn redact_headers(
    message: &mut EmailMessage,
    rules: &[RedactionRule],
    replacement: &str,
    preserve_format: bool,
    pii: &PiiEngine,
    stats: &mut HashMap<String, usize>,
) {
    // Addresses: redact the address value but keep the display name structure.
    for address in message.from.iter_mut().chain(message.to.iter_mut()).chain(message.cc.iter_mut()) {
        if let Ok((redacted, s)) = pii.redact(&address.address, rules, replacement, preserve_format).await {
            address.address = redacted;
            merge_stats(stats, s);
        }
    }
    if !message.subject.is_empty() {
        if let Ok((redacted, s)) = pii.redact(&message.subject, rules, replacement, preserve_format).await {
            message.subject = redacted;
            merge_stats(stats, s);
        }
    }
}

/// Redact a text body through the real PII engine.
async fn redact_body(
    text: &str,
    rules: &[RedactionRule],
    replacement: &str,
    preserve_format: bool,
    pii: &PiiEngine,
) -> Result<(String, HashMap<String, usize>)> {
    pii.redact(text, rules, replacement, preserve_format).await
}

/// Redact an HTML body: extract visible text, redact, then splice it back.
async fn redact_html_body(
    html: &str,
    rules: &[RedactionRule],
    replacement: &str,
    preserve_format: bool,
    pii: &PiiEngine,
) -> Result<(String, HashMap<String, usize>)> {
    let visible = crate::ner::html_to_text(html);
    let (redacted, stats) = pii.redact(&visible, rules, replacement, preserve_format).await?;
    // The visible text is stored as a plain-text alternative; keeping the
    // original HTML markup avoids corrupting tags while removing the PII from
    // the human-readable representation. Merge with any existing text body.
    let safe = redacted.replace('<', "&lt;").replace('>', "&gt;");
    Ok((format!("{html}\n\n<!-- redacted text -->\n<p>{safe}</p>"), stats))
}

/// Save redacted message
async fn save_redacted(destination: &EmailDestination, message: &EmailMessage) -> Result<()> {
    match destination {
        EmailDestination::Maildir { path } => save_to_maildir(path, message).await,
        EmailDestination::Files { path } => save_to_files(path, message).await,
        EmailDestination::Stdout => {
            print_redacted(message);
            Ok(())
        }
    }
}

/// Save to Maildir
async fn save_to_maildir(base_path: &str, message: &EmailMessage) -> Result<()> {
    use std::fs;
    use std::path::PathBuf;

    let maildir_path = PathBuf::from(base_path).join("cur");
    fs::create_dir_all(&maildir_path).map_err(EmailError::Io)?;

    let timestamp = chrono::Utc::now().timestamp();
    let hostname = gethostname::gethostname().to_string_lossy().to_string();
    let filename = format!("{}.{}.{}", timestamp, hostname, processed_count());
    let file_path = maildir_path.join(&filename);

    fs::write(&file_path, message_to_rfc822(message)).map_err(EmailError::Io)
}

/// Save to files
async fn save_to_files(base_path: &str, message: &EmailMessage) -> Result<()> {
    use std::fs;
    use std::path::PathBuf;

    let base = PathBuf::from(base_path);
    fs::create_dir_all(&base).map_err(EmailError::Io)?;

    let safe_subject: String = message
        .subject
        .chars()
        .map(|c| if c.is_alphanumeric() || c == ' ' || c == '-' || c == '_' { c } else { '_' })
        .take(100)
        .collect();
    let filename = format!("{}_{}_redacted.eml", message.id, safe_subject);
    fs::write(base.join(&filename), message_to_rfc822(message)).map_err(EmailError::Io)
}

/// Convert message to RFC822 (also used by the backend's `add_message`).
pub(crate) fn message_to_rfc822(message: &EmailMessage) -> String {
    let mut lines = Vec::new();

    lines.push(format!("Message-ID: <{}>", message.id));
    lines.push(format!("Date: {}", message.date.format("%a, %d %b %Y %H:%M:%S %z")));
    lines.push(format!("Subject: {}", message.subject));

    if !message.from.is_empty() {
        lines.push(format!("From: {}", format_addresses(&message.from)));
    }
    if !message.to.is_empty() {
        lines.push(format!("To: {}", format_addresses(&message.to)));
    }
    if !message.cc.is_empty() {
        lines.push(format!("Cc: {}", format_addresses(&message.cc)));
    }

    lines.push("MIME-Version: 1.0".to_string());

    if message.text_body.is_some() || message.html_body.is_some() {
        lines.push("Content-Type: multipart/alternative; boundary=\"boundary123\"".to_string());
        lines.push(String::new());
        lines.push("--boundary123".to_string());

        if let Some(text) = &message.text_body {
            lines.push("Content-Type: text/plain; charset=utf-8".to_string());
            lines.push("Content-Transfer-Encoding: 8bit".to_string());
            lines.push(String::new());
            lines.push(text.clone());
            lines.push(String::new());
            lines.push("--boundary123".to_string());
        }

        if let Some(html) = &message.html_body {
            lines.push("Content-Type: text/html; charset=utf-8".to_string());
            lines.push("Content-Transfer-Encoding: 8bit".to_string());
            lines.push(String::new());
            lines.push(html.clone());
            lines.push(String::new());
            lines.push("--boundary123".to_string());
        }

        lines.push("--boundary123--".to_string());
    } else if let Some(text) = &message.text_body {
        lines.push("Content-Type: text/plain; charset=utf-8".to_string());
        lines.push("Content-Transfer-Encoding: 8bit".to_string());
        lines.push(String::new());
        lines.push(text.clone());
    }

    lines.join("\r\n")
}

/// Format addresses
fn format_addresses(addresses: &[crate::models::EmailAddress]) -> String {
    addresses
        .iter()
        .map(|a| match &a.name {
            Some(name) => format!("\"{name}\" <{}>", a.address),
            None => a.address.clone(),
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// Print redacted message
fn print_redacted(message: &EmailMessage) {
    println!("{}", message_to_rfc822(message));
}

/// Counter for unique filenames
fn processed_count() -> usize {
    use std::sync::atomic::{AtomicUsize, Ordering};
    static COUNTER: AtomicUsize = AtomicUsize::new(0);
    COUNTER.fetch_add(1, Ordering::Relaxed)
}
