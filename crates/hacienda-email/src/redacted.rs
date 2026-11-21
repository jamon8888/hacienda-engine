//! Content-free projections of email metadata.
//!
//! P0.1 and P0.3 both require that no clear subject, display name, address or
//! attachment name survives into a RAG row or a "redacted" RFC822 file. The PII
//! engine removes *detectable* structured PII from body text, but arbitrary
//! names in `From:`/`Subject:` are not reliably detected. This module is the
//! single place that replaces those metadata fields with deterministic,
//! domain-separated opaque tokens **before** any record or output is built.
//!
//! Tokens are correlation handles, not encryption: the same input always
//! yields the same token (so re-ingestion stays idempotent), but the clear
//! value cannot be read back from a persisted row or file. Revealing the
//! original remains a separate, audited viewer capability; cross-component
//! key/reveal interoperability is out of scope for this crate (brief P0.5).

use crate::models::{EmailAddress, EmailMessage, Envelope};
use chrono::{DateTime, Utc};

/// Stable, domain-separated, content-free token for one metadata value.
///
/// FNV-1a 64-bit: no dependency, no key material on disk, stable across runs.
/// The domain separator prevents a subject token from colliding with an
/// address token for the same string.
pub fn metadata_token(domain: &str, value: &str) -> String {
    const OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0000_0100_0000_01b3;

    let mut hash = OFFSET_BASIS;
    // Feed the domain, a NUL separator, then the value. The trailing NUL keeps
    // ("ab", "c") and ("a", "bc") from hashing identically.
    for byte in domain
        .as_bytes()
        .iter()
        .copied()
        .chain(std::iter::once(0))
        .chain(value.as_bytes().iter().copied())
    {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(PRIME);
    }
    format!("{domain}-{hash:016x}")
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
    fn from_address(address: &EmailAddress) -> Self {
        Self {
            name: address
                .name
                .as_deref()
                .filter(|name| !name.is_empty())
                .map(|name| metadata_token("name", name)),
            address: metadata_token("address", &address.address),
        }
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
    pub fn from_envelope(envelope: &Envelope) -> Self {
        Self {
            id: envelope.id.clone(),
            mailbox: envelope.mailbox.clone(),
            subject: metadata_token("subject", &envelope.subject),
            from: envelope
                .from
                .iter()
                .map(RedactedEmailAddress::from_address)
                .collect(),
            to: envelope
                .to
                .iter()
                .map(RedactedEmailAddress::from_address)
                .collect(),
            cc: Vec::new(),
            bcc: Vec::new(),
            date: envelope.date.clone(),
            has_attachments: envelope.has_attachments,
        }
    }

    /// Tokenize a full message before writing a redacted RFC822 file.
    pub fn from_message(message: &EmailMessage) -> Self {
        Self {
            id: message.id.clone(),
            mailbox: message.mailbox.clone(),
            subject: metadata_token("subject", &message.subject),
            from: message
                .from
                .iter()
                .map(RedactedEmailAddress::from_address)
                .collect(),
            to: message
                .to
                .iter()
                .map(RedactedEmailAddress::from_address)
                .collect(),
            cc: message
                .cc
                .iter()
                .map(RedactedEmailAddress::from_address)
                .collect(),
            bcc: message
                .bcc
                .iter()
                .map(RedactedEmailAddress::from_address)
                .collect(),
            date: message.date.clone(),
            has_attachments: !message.attachments.is_empty(),
        }
    }

    /// Overwrite every PII-bearing header (including attachment filenames)
    /// with the tokenized projection. Body text is handled separately by the
    /// PII engine; this intentionally touches only metadata.
    pub fn apply_to_message(&self, message: &mut EmailMessage) {
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
            attachment.filename = metadata_token("attachment", &attachment.filename);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_are_stable_domain_separated_and_content_free() {
        let subject = metadata_token("subject", "jane.doe@acme.example");
        assert_eq!(subject, metadata_token("subject", "jane.doe@acme.example"));
        assert_ne!(subject, metadata_token("address", "jane.doe@acme.example"));
        assert_ne!(subject, metadata_token("subject", "john.doe@acme.example"));
        assert!(!subject.contains("jane.doe@acme.example"));
        assert!(!subject.contains("acme"));
    }

    #[test]
    fn distinct_domains_do_not_collide_on_the_same_value() {
        assert_ne!(metadata_token("a", "bc"), metadata_token("ab", "c"));
    }
}
