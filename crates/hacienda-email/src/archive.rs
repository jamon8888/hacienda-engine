use crate::backend::EmailClient;
use crate::config::EmailConfig;
use crate::error::{EmailError, Result};
use crate::models::{EmailDestination, EmailMessage, EmailOperation, EmailSource, Envelope};
use chrono::{DateTime, Duration, Utc};
use std::path::Path;

pub async fn archive(
    client: &dyn EmailClient,
    op: EmailOperation,
    _config: &EmailConfig,
) -> Result<crate::models::EmailResponse> {
    let EmailOperation::Archive {
        source,
        destination,
        since,
        before,
        mailbox,
        include_attachments: _,
    } = op
    else {
        return Err(EmailError::InvalidOperation(
            "Expected Archive operation".to_string(),
        ));
    };
    let mailbox = mailbox.unwrap_or_else(|| "INBOX".to_string());
    let envelopes = fetch_envelopes(client, &source, &mailbox, since, before).await?;
    let mut response = crate::models::EmailResponse::default();
    for envelope in envelopes {
        match client.get_message(&mailbox, &envelope.id).await {
            Ok(message) => match save_message(&destination, &message).await {
                Ok(()) => response.processed_count += 1,
                Err(error) => response
                    .errors
                    .push(format!("Failed to save message {}: {error}", envelope.id)),
            },
            Err(error) => response
                .errors
                .push(format!("Failed to fetch message {}: {error}", envelope.id)),
        }
    }
    response.success = response.errors.is_empty();
    Ok(response)
}

async fn fetch_envelopes(
    client: &dyn EmailClient,
    source: &EmailSource,
    mailbox: &str,
    since: Option<String>,
    before: Option<String>,
) -> Result<Vec<Envelope>> {
    let mut envelopes = match source {
        EmailSource::Account { .. } | EmailSource::Maildir { .. } => {
            client.list_envelopes(mailbox, None).await?
        }
        EmailSource::Files { .. } | EmailSource::Stdin => Vec::new(),
    };
    let after = since.as_deref().and_then(parse_human_date);
    let until = before.as_deref().and_then(parse_human_date);
    envelopes.retain(|item| {
        after.is_none_or(|date| item.date >= date) && until.is_none_or(|date| item.date <= date)
    });
    Ok(envelopes)
}

async fn save_message(destination: &EmailDestination, message: &EmailMessage) -> Result<()> {
    let content = message.text_body.clone().unwrap_or_default();
    match destination {
        EmailDestination::Files { path } | EmailDestination::Maildir { path } => {
            tokio::fs::create_dir_all(path)
                .await
                .map_err(EmailError::Io)?;
            tokio::fs::write(Path::new(path).join(format!("{}.eml", message.id)), content)
                .await
                .map_err(EmailError::Io)
        }
        EmailDestination::Stdout => {
            println!("{content}");
            Ok(())
        }
    }
}

pub fn parse_human_date(value: &str) -> Option<DateTime<Utc>> {
    if let Ok(date) = DateTime::parse_from_rfc3339(value) {
        return Some(date.with_timezone(&Utc));
    }
    if let Ok(date) = chrono::NaiveDate::parse_from_str(value, "%Y-%m-%d") {
        return Some(date.and_hms_opt(0, 0, 0)?.and_utc());
    }
    let days = value.strip_suffix(" days ago")?.parse::<i64>().ok()?;
    Some(Utc::now() - Duration::days(days))
}
