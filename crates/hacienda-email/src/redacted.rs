//! Content-free projections of email metadata.
//!
//! P0.1 and P0.3 require that no clear subject, display name, address or
//! attachment name survives into a RAG row or a "redacted" RFC822 file. The PII
//! engine removes *detectable* structured PII from body text, but arbitrary
//! names in `From:`/`Subject:` are not reliably detected. This module replaces
//! those metadata fields with opaque tokens **before** any record or output is
//! built.
//!
//! P0.5: tokens are no longer produced by a local, non-keyed FNV-1a hash. They
//! are minted by the host's tenant-keyed pseudonymiser
//! (`HaciendaFacade::mint_pseudonym_token_with_auth`) through [`PiiEngine`],
//! using the request's [`Caller`]. The same facade and key resolver can reveal
//! them under the owning tenant, but a different tenant's key cannot — and no
//! new secret or ad-hoc hash is introduced here.

use crate::error::Result;
use crate::models::{EmailAddress, EmailMessage, Envelope};
use crate::pii::PiiEngine;
use chrono::{DateTime, Utc};
use hacienda_core::auth::Caller;

/// Opaque, tenant-keyed token for one metadata value.
///
/// The `domain` is folded into the pseudonym category label, so a subject token
/// can never collide with an address token for the same string. The value is
/// never echoed: the token's cleartext is recoverable only by an authorised
/// reveal through the same host facade and tenant key.
pub async fn metadata_token(
    pii: &PiiEngine,
    caller: Caller<'_>,
    domain: &str,
    value: &str,
) -> Result<String> {
    pii.project_metadata(caller, domain, value).await
}

/// Opaque address pair. Display name and mailbox are tokenized separately so
/// neither a raw name nor a raw address can appear in a serialized header.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RedactedEmailAddress {
    /// Tokenized display name, or `None` when the original had no name.
    pub name: Option<String>,
    /// Tokenized mailbox address.
    pub address: String,
}

impl RedactedEmailAddress {
    async fn from_address(
        pii: &PiiEngine,
        caller: Caller<'_>,
        address: &EmailAddress,
    ) -> Result<Self> {
        let name = match address.name.as_deref().filter(|n| !n.is_empty()) {
            Some(name) => Some(metadata_token(pii, caller, "name", name).await?),
            None => None,
        };
        Ok(Self {
            name,
            address: metadata_token(pii, caller, "address", &address.address).await?,
        })
    }

    fn to_model(&self) -> EmailAddress {
        EmailAddress {
            name: self.name.clone(),
            address: self.address.clone(),
        }
    }

    /// Render tokenized addresses for RAG metadata text and RFC822 headers.
    pub fn format(addresses: &[Self]) -> String {
        addresses
            .iter()
            .map(|a| match &a.name {
                Some(name) => format!("{name} <{}>", a.address),
                None => a.address.clone(),
            })
            .collect::<Vec<_>>()
            .join(", ")
    }
}

/// An envelope projection whose every PII-bearing field is an opaque token.
///
/// Build this from the *original* envelope/message before mutating anything,
/// then use it for both `build_email_records` (RAG) and header rewriting in
/// the redact path (RFC822).
#[derive(Debug, Clone)]
pub struct RedactedEmailEnvelope {
    pub id: String,
    pub mailbox: String,
    /// Opaque subject token (never the clear subject).
    pub subject: String,
    pub from: Vec<RedactedEmailAddress>,
    pub to: Vec<RedactedEmailAddress>,
    pub cc: Vec<RedactedEmailAddress>,
    pub bcc: Vec<RedactedEmailAddress>,
    pub date: DateTime<Utc>,
    pub has_attachments: bool,
}

impl RedactedEmailEnvelope {
    /// Tokenize an envelope before it feeds RAG record construction.
    pub async fn from_envelope(
        pii: &PiiEngine,
        caller: Caller<'_>,
        envelope: &Envelope,
    ) -> Result<Self> {
        let mut from = Vec::with_capacity(envelope.from.len());
        for address in &envelope.from {
            from.push(RedactedEmailAddress::from_address(pii, caller, address).await?);
        }
        let mut to = Vec::with_capacity(envelope.to.len());
        for address in &envelope.to {
            to.push(RedactedEmailAddress::from_address(pii, caller, address).await?);
        }
        Ok(Self {
            id: envelope.id.clone(),
            mailbox: envelope.mailbox.clone(),
            subject: metadata_token(pii, caller, "subject", &envelope.subject).await?,
            from,
            to,
            cc: Vec::new(),
            bcc: Vec::new(),
            date: envelope.date.clone(),
            has_attachments: envelope.has_attachments,
        })
    }

    /// Tokenize a full message before writing a redacted RFC822 file.
    pub async fn from_message(
        pii: &PiiEngine,
        caller: Caller<'_>,
        message: &EmailMessage,
    ) -> Result<Self> {
        async fn addresses(
            pii: &PiiEngine,
            caller: Caller<'_>,
            input: &[EmailAddress],
        ) -> Result<Vec<RedactedEmailAddress>> {
            let mut out = Vec::with_capacity(input.len());
            for address in input {
                out.push(RedactedEmailAddress::from_address(pii, caller, address).await?);
            }
            Ok(out)
        }
        Ok(Self {
            id: message.id.clone(),
            mailbox: message.mailbox.clone(),
            subject: metadata_token(pii, caller, "subject", &message.subject).await?,
            from: addresses(pii, caller, &message.from).await?,
            to: addresses(pii, caller, &message.to).await?,
            cc: addresses(pii, caller, &message.cc).await?,
            bcc: addresses(pii, caller, &message.bcc).await?,
            date: message.date.clone(),
            has_attachments: !message.attachments.is_empty(),
        })
    }

    /// Overwrite every PII-bearing header (including attachment filenames)
    /// with the tokenized projection. Body text is handled separately by the
    /// PII engine; this intentionally touches only metadata.
    pub async fn apply_to_message(
        &self,
        pii: &PiiEngine,
        caller: Caller<'_>,
        message: &mut EmailMessage,
    ) -> Result<()> {
        message.subject = self.subject.clone();
        message.from = self
            .from
            .iter()
            .map(RedactedEmailAddress::to_model)
            .collect();
        message.to = self.to.iter().map(RedactedEmailAddress::to_model).collect();
        message.cc = self.cc.iter().map(RedactedEmailAddress::to_model).collect();
        message.bcc = self
            .bcc
            .iter()
            .map(RedactedEmailAddress::to_model)
            .collect();
        for attachment in &mut message.attachments {
            attachment.filename =
                metadata_token(pii, caller, "attachment", &attachment.filename).await?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn metadata_tokens_are_domain_separated_and_content_free() {
        let pii = test_pii();
        let ctx = test_caller();
        let caller = Caller::Principal(&ctx);

        let subject = metadata_token(&pii, caller, "subject", "jane.doe@acme.example")
            .await
            .unwrap();
        let same = metadata_token(&pii, caller, "subject", "jane.doe@acme.example")
            .await
            .unwrap();
        let other_domain = metadata_token(&pii, caller, "address", "jane.doe@acme.example")
            .await
            .unwrap();
        let other_value = metadata_token(&pii, caller, "subject", "john.doe@acme.example")
            .await
            .unwrap();

        assert_eq!(subject, same, "tokens must be stable for re-ingestion");
        assert_ne!(subject, other_domain, "domains must not collide");
        assert_ne!(subject, other_value, "values must not collide");
        assert!(!subject.contains("jane.doe@acme.example"));
        assert!(!subject.contains("acme"));
    }

    /// The test-only helper wires a deterministic default-tenant key resolver.
    /// See `tests/tenant_pii.rs` for the full two-tenant matrix.
    fn test_pii() -> PiiEngine {
        PiiEngine::for_local_testing()
    }

    fn test_caller() -> hacienda_core::auth::AuthContext {
        use hacienda_core::auth::{Capability, CapabilitySet};
        hacienda_core::auth::AuthContext::with_tenant(
            "test",
            hacienda_core::tenancy::TenantId::default_tenant(),
            CapabilitySet::new([Capability::DocumentsProcess]),
        )
    }
}
