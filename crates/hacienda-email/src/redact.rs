use crate::backend::EmailClient;
use crate::config::{EmailConfig, RedactionRule};
use crate::error::{EmailError, Result};
use crate::models::{EmailDestination, EmailMessage, EmailOperation, EmailSource, RedactionStats};
use std::collections::HashMap;
use std::path::Path;

/// Redact PII from emails
pub async fn redact(
    client: &dyn EmailClient,
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
                // Redact text body
                if let Some(text) = &message.text_body {
                    match redact_text(text, &redaction_rules, replacement, preserve_format) {
                        Ok((redacted, stats)) => {
                            message.text_body = Some(redacted);
                            for (k, v) in stats {
                                *all_stats.entry(k).or_insert(0) += v;
                            }
                        }
                        Err(e) => errors.push(format!("Text redaction failed for {}: {}", envelope.id, e)),
                    }
                }

                // Redact HTML body
                if let Some(html) = &message.html_body {
                    match redact_html(html, &redaction_rules, replacement, preserve_format) {
                        Ok((redacted, stats)) => {
                            message.html_body = Some(redacted);
                            for (k, v) in stats {
                                *all_stats.entry(k).or_insert(0) += v;
                            }
                        }
                        Err(e) => errors.push(format!("HTML redaction failed for {}: {}", envelope.id, e)),
                    }
                }

                // Save redacted message
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

/// Fetch envelopes for redaction
async fn fetch_envelopes(
    client: &dyn EmailClient,
    source: &EmailSource,
) -> Result<Vec<crate::models::Envelope>> {
    match source {
        EmailSource::Account { name: _, mailbox } => {
            let mb = mailbox.as_deref().unwrap_or("INBOX");
            client.list_envelopes(mb, Some(1000)).await
        }
        EmailSource::Maildir { path: _ } => {
            client.list_envelopes("INBOX", Some(1000)).await
        }
        EmailSource::Files { paths: _ } => {
            Err(EmailError::InvalidOperation("File source not yet implemented".to_string()))
        }
        EmailSource::Stdin => {
            Err(EmailError::InvalidOperation("Stdin source not yet implemented".to_string()))
        }
    }
}

/// Save redacted message
async fn save_redacted(
    destination: &EmailDestination,
    message: &EmailMessage,
) -> Result<()> {
    match destination {
        EmailDestination::Maildir { path } => {
            save_to_maildir(path, message).await
        }
        EmailDestination::Files { path } => {
            save_to_files(path, message).await
        }
        EmailDestination::Stdout => {
            print_redacted(message);
            Ok(())
        }
    }
}

/// Save to Maildir
async fn save_to_maildir(
    base_path: &str,
    message: &EmailMessage,
) -> Result<()> {
    use std::fs;
    use std::path::PathBuf;

    let maildir_path = PathBuf::from(base_path).join("cur");
    fs::create_dir_all(&maildir_path)
        .map_err(|e| EmailError::Io(e))?;

    let timestamp = chrono::Utc::now().timestamp();
    let hostname = gethostname::gethostname().to_string_lossy().to_string();
    let filename = format!("{}.{}.{}", timestamp, hostname, processed_count());
    let file_path = maildir_path.join(&filename);

    let content = message_to_rfc822(message);
    fs::write(&file_path, content)
        .map_err(|e| EmailError::Io(e))?;

    Ok(())
}

/// Save to files
async fn save_to_files(
    base_path: &str,
    message: &EmailMessage,
) -> Result<()> {
    use std::fs;
    use std::path::PathBuf;

    let base = PathBuf::from(base_path);
    fs::create_dir_all(&base)
        .map_err(|e| EmailError::Io(e))?;

    let safe_subject = message.subject.chars()
        .map(|c| if c.is_alphanumeric() || c == ' ' || c == '-' || c == '_' { c } else { '_' })
        .collect::<String>();
    let safe_subject = safe_subject.chars().take(100).collect::<String>();

    let filename = format!("{}_{}_redacted.eml", message.id, safe_subject);
    let file_path = base.join(&filename);

    let content = message_to_rfc822(message);
    fs::write(&file_path, content)
        .map_err(|e| EmailError::Io(e))?;

    Ok(())
}

/// Convert message to RFC822
fn message_to_rfc822(message: &EmailMessage) -> String {
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
        lines.push("Content-Type: multipart/alternative; boundary="boundary123"".to_string());
        lines.push("".to_string());
        lines.push("--boundary123".to_string());

        if let Some(text) = &message.text_body {
            lines.push("Content-Type: text/plain; charset=utf-8".to_string());
            lines.push("Content-Transfer-Encoding: 8bit".to_string());
            lines.push("".to_string());
            lines.push(text.clone());
            lines.push("".to_string());
            lines.push("--boundary123".to_string());
        }

        if let Some(html) = &message.html_body {
            lines.push("Content-Type: text/html; charset=utf-8".to_string());
            lines.push("Content-Transfer-Encoding: 8bit".to_string());
            lines.push("".to_string());
            lines.push(html.clone());
            lines.push("".to_string());
            lines.push("--boundary123".to_string());
        }

        lines.push("--boundary123--".to_string());
    } else if let Some(text) = &message.text_body {
        lines.push("Content-Type: text/plain; charset=utf-8".to_string());
        lines.push("Content-Transfer-Encoding: 8bit".to_string());
        lines.push("".to_string());
        lines.push(text.clone());
    }

    lines.join("\r\n")
}

/// Format addresses
fn format_addresses(addresses: &[crate::models::EmailAddress]) -> String {
    addresses.iter()
        .map(|a| match &a.name {
            Some(name) => format!(""{} <{}>>"", name, a.address),
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

/// Redact text using hacienda-core PII engine
fn redact_text(
    text: &str,
    rules: &[RedactionRule],
    replacement: &str,
    preserve_format: bool,
) -> Result<(String, HashMap<String, usize>)> {
    #[cfg(feature = "redact")]
    {
        use hacienda_core::pii::{PiiDetector, PiiRule};

        let pii_rules: Vec<PiiRule> = rules.iter().map(convert_rule).collect();
        let detector = PiiDetector::new(pii_rules)
            .map_err(|e| EmailError::Redaction(format!("Failed to create PII detector: {}", e)))?;

        let result = detector.redact(text, replacement, preserve_format)
            .map_err(|e| EmailError::Redaction(format!("Redaction failed: {}", e)))?;

        Ok((result.redacted_text, result.stats))
    }

    #[cfg(not(feature = "redact"))]
    {
        // Fallback: regex-based redaction
        redact_text_regex(text, rules, replacement)
    }
}

/// Redact HTML
fn redact_html(
    html: &str,
    rules: &[RedactionRule],
    replacement: &str,
    preserve_format: bool,
) -> Result<(String, HashMap<String, usize>)> {
    #[cfg(feature = "redact")]
    {
        use hacienda_core::pii::{PiiDetector, PiiRule};

        let pii_rules: Vec<PiiRule> = rules.iter().map(convert_rule).collect();
        let detector = PiiDetector::new(pii_rules)
            .map_err(|e| EmailError::Redaction(format!("Failed to create PII detector: {}", e)))?;

        let result = detector.redact_html(html, replacement, preserve_format)
            .map_err(|e| EmailError::Redaction(format!("HTML redaction failed: {}", e)))?;

        Ok((result.redacted_text, result.stats))
    }

    #[cfg(not(feature = "redact"))]
    {
        // Fallback: simple text redaction on HTML
        redact_text_regex(html, rules, replacement)
    }
}

/// Convert RedactionRule to PiiRule
#[cfg(feature = "redact")]
fn convert_rule(rule: &RedactionRule) -> hacienda_core::pii::PiiRule {
    use hacienda_core::pii::PiiRule;
    match rule {
        RedactionRule::Email => PiiRule::Email,
        RedactionRule::Phone => PiiRule::Phone,
        RedactionRule::Ssn => PiiRule::Ssn,
        RedactionRule::CreditCard => PiiRule::CreditCard,
        RedactionRule::Iban => PiiRule::Iban,
        RedactionRule::Ip => PiiRule::Ip,
        RedactionRule::Custom { pattern, label } => PiiRule::Custom { pattern: pattern.clone(), label: label.clone() },
    }
}

/// Regex-based redaction (fallback)
fn redact_text_regex(
    text: &str,
    rules: &[RedactionRule],
    replacement: &str,
) -> Result<(String, HashMap<String, usize>)> {
    use regex::Regex;
    let mut result = text.to_string();
    let mut stats = HashMap::new();

    for rule in rules {
        let (pattern, label) = match rule {
            RedactionRule::Email => (r"\b[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Z|a-z]{2,}\b", "EMAIL"),
            RedactionRule::Phone => (r"\b(?:\+?1[-.\s]?)?\(?\d{3}\)?[-.\s]?\d{3}[-.\s]?\d{4}\b", "PHONE"),
            RedactionRule::Ssn => (r"\b\d{3}-\d{2}-\d{4}\b", "SSN"),
            RedactionRule::CreditCard => (r"\b\d{4}[-\s]?\d{4}[-\s]?\d{4}[-\s]?\d{4}\b", "CREDIT_CARD"),
            RedactionRule::Iban => (r"\b[A-Z]{2}\d{2}[A-Z0-9]{4}\d{7}([A-Z0-9]?){0,16}\b", "IBAN"),
            RedactionRule::Ip => (r"\b\d{1,3}\.\d{1,3}\.\d{1,3}\.\d{1,3}\b", "IP"),
            RedactionRule::Custom { pattern, label } => (pattern.as_str(), label.as_str()),
        };

        if let Ok(re) = Regex::new(pattern) {
            let count = re.find_iter(&result).count();
            if count > 0 {
                result = re.replace_all(&result, replacement).to_string();
                *stats.entry(label.to_string()).or_insert(0) += count;
            }
        }
    }

    Ok((result, stats))
}