use crate::backend::EmailClient;
use crate::config::EmailConfig;
use crate::error::{EmailError, Result};
use crate::models::{EmailDestination, EmailMessage, EmailOperation, EmailSource};
use std::path::Path;

/// Archive emails from source to destination
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
        include_attachments,
    } = op else {
        return Err(EmailError::InvalidOperation("Expected Archive operation".to_string()));
    };

    let mailbox_name = mailbox.unwrap_or_else(|| "INBOX".to_string());
    let envelopes = fetch_envelopes(client, &source, &mailbox_name, since, before).await?;

    let mut processed = 0;
    let mut errors = Vec::new();

    for envelope in envelopes {
        match client.get_message(&mailbox_name, &envelope.id).await {
            Ok(message) => {
                if let Err(e) = save_message(&destination, &message, include_attachments).await {
                    errors.push(format!("Failed to save message {}: {}", envelope.id, e));
                } else {
                    processed += 1;
                }
            }
            Err(e) => {
                errors.push(format!("Failed to fetch message {}: {}", envelope.id, e));
            }
        }
    }

    Ok(crate::models::EmailResponse {
        succesbase64: entrée incorrecte
