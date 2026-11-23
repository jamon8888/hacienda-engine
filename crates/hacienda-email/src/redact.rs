use crate::backend::EmailClient;
use crate::config::{EmailConfig, RedactionRule};
use crate::error::{EmailError, Result};
use crate::models::{EmailDestination, EmailMessage, EmailOperation, EmailSource, RedactionStats};
use crate::pii::PiiEngine;
use hacienda_core::auth::Caller;
use std::collections::HashMap;

/// Redact PII from emails using the host's tenant-scoped PII authority.
pub async fn redact(
    client: &dyn EmailClient,
    pii: &PiiEngine,
    caller: Caller<'_>,
    op: EmailOperation,
    config: &EmailConfig,
) -> Result<crate::models::EmailResponse> {
    let EmailOperation::Redact {
        source,
        destination,
        rules,
    } = op
    else {
        return Err(EmailError::InvalidOperation(
            "Expected Redact operation".to_string(),
        ));
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
                // P0.3: build a content-free projection of every metadata field
                // from the ORIGINAL message before any output is written. The
                // PII engine cannot be trusted to recognize arbitrary display
                // names, so names, addresses, subject and attachment names are
                // tokenized rather than merely regex-redacted.
                let redacted_envelope =
                    crate::redacted::RedactedEmailEnvelope::from_message(pii, caller, &message)
                        .await?;
                // Fail closed: a body whose redaction errored still holds clear
                // content, so it must never reach a file named "redacted".
                let mut bodies_safe = true;

                // Redact the text body (real PII engine).
                if let Some(text) = &message.text_body {
                    match redact_body(
                        text,
                        &redaction_rules,
                        replacement,
                        preserve_format,
                        pii,
                        caller,
                    )
                    .await
                    {
                        Ok((redacted, stats)) => {
                            message.text_body = Some(redacted);
                            merge_stats(&mut all_stats, stats);
                        }
                        Err(e) => {
                            errors
                                .push(format!("Text redaction failed for {}: {}", envelope.id, e));
                            bodies_safe = false;
                        }
                    }
                }

                // Redact the HTML body. The original markup is NEVER reused:
                // a "redacted" file that still contains the source HTML would
                // also contain the PII hidden in attributes/comments/markup.
                if let Some(html) = &message.html_body {
                    match redact_html_body(
                        html,
                        &redaction_rules,
                        replacement,
                        preserve_format,
                        pii,
                        caller,
                    )
                    .await
                    {
                        Ok((redacted, stats)) => {
                            message.html_body = Some(redacted);
                            merge_stats(&mut all_stats, stats);
                        }
                        Err(e) => {
                            errors
                                .push(format!("HTML redaction failed for {}: {}", envelope.id, e));
                            bodies_safe = false;
                        }
                    }
                }

                if !bodies_safe {
                    // Skip this message entirely; no partial/clear output file.
                    continue;
                }

                // Replace every PII-bearing header with its token. This covers
                // From/To/Cc/Bcc display names AND addresses, Subject, and
                // attachment filenames — none can survive in clear text.
                redacted_envelope
                    .apply_to_message(pii, caller, &mut message)
                    .await?;

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
            EmailDestination::Maildir { path } | EmailDestination::Files { path } => {
                Some(path.clone())
            }
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
        EmailSource::Files { paths: _ } => Err(EmailError::InvalidOperation(
            "File source not yet implemented".to_string(),
        )),
        EmailSource::Stdin => Err(EmailError::InvalidOperation(
            "Stdin source not yet implemented".to_string(),
        )),
    }
}

/// Redact a text body through the real PII engine.
async fn redact_body(
    text: &str,
    rules: &[RedactionRule],
    replacement: &str,
    preserve_format: bool,
    pii: &PiiEngine,
    caller: Caller<'_>,
) -> Result<(String, HashMap<String, usize>)> {
    pii.redact(caller, text, rules, replacement, preserve_format)
        .await
}

/// Redact an HTML body by rebuilding a new, safe HTML fragment from the
/// redacted visible projection.
///
/// P0.3: the original HTML is discarded after extraction — it is never
/// concatenated into the result. Tags, attributes and comments could all carry
/// clear PII, so the output contains only escaped redacted text.
async fn redact_html_body(
    html: &str,
    rules: &[RedactionRule],
    replacement: &str,
    preserve_format: bool,
    pii: &PiiEngine,
    caller: Caller<'_>,
) -> Result<(String, HashMap<String, usize>)> {
    let visible = crate::ner::html_to_text(html);
    let (redacted, stats) = pii
        .redact(caller, &visible, rules, replacement, preserve_format)
        .await?;
    Ok((safe_html_from_redacted_text(&redacted), stats))
}

/// Build a brand-new HTML fragment from already-redacted text. Every HTML
/// metacharacter in the source text is escaped, so no original markup or
/// attribute can reappear through the projection.
fn safe_html_from_redacted_text(redacted: &str) -> String {
    let escaped = redacted
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;");
    let body = escaped.split('\n').collect::<Vec<_>>().join("<br/>");
    format!("<p>{body}</p>")
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
        .map(|c| {
            if c.is_alphanumeric() || c == ' ' || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .take(100)
        .collect();
    let filename = format!("{}_{}_redacted.eml", message.id, safe_subject);
    fs::write(base.join(&filename), message_to_rfc822(message)).map_err(EmailError::Io)
}

/// Convert message to RFC822 (also used by the backend's `add_message`).
pub(crate) fn message_to_rfc822(message: &EmailMessage) -> String {
    let mut lines = Vec::new();

    lines.push(format!("Message-ID: <{}>", message.id));
    lines.push(format!(
        "Date: {}",
        message.date.format("%a, %d %b %Y %H:%M:%S %z")
    ));
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{EmailAddress, EmailAttachment};
    use crate::redacted::RedactedEmailEnvelope;
    use hacienda_core::auth::{AuthContext, Capability, CapabilitySet};

    fn test_ctx() -> AuthContext {
        AuthContext::with_tenant(
            "test",
            hacienda_core::tenancy::TenantId::default_tenant(),
            CapabilitySet::new([Capability::DocumentsProcess]),
        )
    }

    const CLEAR_FIXTURES: &[&str] = &[
        "Jane Doe",
        "Richard Roe",
        "Bcc Person",
        "jane.doe@acme.example",
        "richard.roe@acme.example",
        "cc.person@acme.example",
        "bcc.person@acme.example",
        "123-45-6789",
        "jane-doe-contract.pdf",
    ];

    fn fixture_message() -> EmailMessage {
        EmailMessage {
            id: "uid-1".to_string(),
            mailbox: "INBOX".to_string(),
            subject: "Jane Doe SSN 123-45-6789 review".to_string(),
            from: vec![EmailAddress {
                name: Some("Jane Doe".to_string()),
                address: "jane.doe@acme.example".to_string(),
            }],
            to: vec![EmailAddress {
                name: Some("Richard Roe".to_string()),
                address: "richard.roe@acme.example".to_string(),
            }],
            cc: vec![EmailAddress {
                name: None,
                address: "cc.person@acme.example".to_string(),
            }],
            bcc: vec![EmailAddress {
                name: Some("Bcc Person".to_string()),
                address: "bcc.person@acme.example".to_string(),
            }],
            date: chrono::Utc::now(),
            text_body: Some("Plain body for jane.doe@acme.example".to_string()),
            html_body: Some(
                "<html><body><p>Contact <b>jane.doe@acme.example</b></p>\
                 <div data-secret=\"Jane Doe\">SSN 123-45-6789</div></body></html>"
                    .to_string(),
            ),
            attachments: vec![EmailAttachment {
                filename: "jane-doe-contract.pdf".to_string(),
                content_type: "application/pdf".to_string(),
                size: 12,
                content_id: None,
                content: vec![0; 12],
            }],
            flags: vec!["\\Seen".to_string()],
        }
    }

    fn assert_no_clear_fixtures(text: &str) {
        for &fixture in CLEAR_FIXTURES {
            assert!(
                !text.contains(fixture),
                "clear fixture leaked into redacted output: {fixture}\noutput:\n{text}"
            );
        }
    }

    /// P0.3: an HTML body marked "redacted" must not reuse a single byte of
    /// the original markup — tags, attributes and comments can all hide PII.
    #[tokio::test]
    async fn html_redaction_discards_original_markup_and_pii() {
        let pii = PiiEngine::for_local_testing();
        let ctx = test_ctx();
        let config = EmailConfig::default();
        let message = fixture_message();
        let html = message.html_body.as_ref().unwrap();

        let (safe, _) = redact_html_body(
            html,
            &config.redaction.rules,
            &config.redaction.replacement,
            config.redaction.preserve_format,
            &pii,
            Caller::Principal(&ctx),
        )
        .await
        .unwrap();

        assert!(safe.starts_with("<p>"), "expected a fresh fragment: {safe}");
        for markup in ["<html", "<body", "<b>", "<div", "data-secret", "<!--"] {
            assert!(!safe.contains(markup), "original markup survived: {markup}");
        }
        assert_no_clear_fixtures(&safe);
    }

    /// P0.3: RFC822 output built from the token projection must expose neither
    /// display names/addresses nor the subject or attachment filename in clear.
    #[tokio::test]
    async fn tokenized_message_never_leaks_clear_metadata_in_rfc822() {
        let pii = PiiEngine::for_local_testing();
        let ctx = test_ctx();
        let caller = Caller::Principal(&ctx);
        let mut message = fixture_message();
        let projection = RedactedEmailEnvelope::from_message(&pii, caller, &message)
            .await
            .unwrap();

        // Body redaction belongs to the PII engine; emulate its safe result so
        // this test isolates the metadata/RFC822 contract.
        message.text_body = Some("[REDACTED] body".to_string());
        message.html_body = Some(safe_html_from_redacted_text("[REDACTED] body"));
        projection
            .apply_to_message(&pii, caller, &mut message)
            .await
            .unwrap();

        let rfc822 = message_to_rfc822(&message);
        assert_no_clear_fixtures(&rfc822);
        assert!(rfc822.contains("Subject: [EMAIL.SUBJECT:"));
        assert!(rfc822.contains("From: \"[EMAIL.NAME:"));
        assert!(rfc822.contains("To: \"[EMAIL.NAME:"));
        // Addresses are syntactically valid synthetic addr-specs under the
        // reserved `.invalid` domain; the plain correlation token stays in the
        // RAG projection, not in a mail header. Cc has no display name, so its
        // header value is the bare addr-spec.
        let cc_line = rfc822
            .lines()
            .find(|line| line.starts_with("Cc: "))
            .expect("Cc header must be present");
        let cc_address = cc_line.strip_prefix("Cc: ").expect("Cc header value");
        let (cc_local, cc_domain) = cc_address.split_once('@').expect("one @ separator");
        assert_eq!(cc_domain, "hacienda.invalid", "{cc_address}");
        assert!(cc_local.contains("EMAIL_ADDRESS"), "{cc_address}");
        assert!(
            cc_local
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_'),
            "synthetic local part must use only RFC 5322 atext: {cc_address}"
        );
        assert!(!rfc822.contains("Cc: [EMAIL.ADDRESS:"));
        assert!(!rfc822.contains("<html"));
        assert!(message.attachments[0]
            .filename
            .starts_with("[EMAIL.ATTACHMENT:"));
    }
}
