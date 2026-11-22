//! Dedicated, safe MIME parsing and token-only viewer projection.
//!
//! The generic `EmailMessage` kept by the plugin is a *processing* model: it
//! carries decoded body text and attachment bytes so redaction, NER, xberg
//! attachment extraction and RAG can do their work. It is deliberately **not** a
//! shape that may cross the renderer/native boundary. This module adds the two
//! things the rest of the crate did not have:
//!
//! 1. A **MIME-tree-aware parser** ([`parse_message_parts`]) that walks the RFC
//!    2045 part tree (including `multipart/*` and nested `message/rfc822`)
//!    instead of taking `body_text(0)` / `body_html(0)` from the parser's flat
//!    "first body part" list. Body parts are collected recursively, and parts
//!    that are attachments by disposition, filename or non-text media type are
//!    kept out of the body.
//! 2. A **content-free viewer projection** ([`MimeViewerProjection`], built by
//!    [`project_viewer`]) whose serialized form contains no raw message text,
//!    no attachment filename and no attachment bytes. The body is passed
//!    through the host's [`PiiEngine`] before it is placed in the projection,
//!    so the projection keeps the crate's existing redaction/token-only
//!    guarantee.
//!
//! # Security contract
//!
//! * The native parsed parts ([`MimeParts`]) are `pub(crate)` and never
//!   `Serialize`: filenames and decoded bytes stay inside the trusted process
//!   and are only used to build the existing internal `EmailMessage`.
//! * [`MimeViewerProjection`] is `Serialize` and has no field that can hold a
//!   filename, a path or raw content. Attachment references are deterministic,
//!   non-reversible handles derived from a caller-supplied opaque seed and the
//!   part ordinal. They are **display handles, not reveal capabilities**;
//!   revealing original content still goes through the existing message-bound
//!   viewer capability and never through this module.
//! * Parse failures are reported with constant, content-free error strings:
//!   no raw byte, header or filename is ever echoed into an error.
//! * Nothing here mints a token, opens a file or changes tenant/reveal policy.
//!   Redaction uses the injected host engine with the caller it is given.

use crate::error::{EmailError, Result};
use crate::models::EmailAttachment;
use crate::pii::PiiEngine;
use hacienda_core::auth::Caller;
use mail_parser::{Message, MessageParser, MessagePart, MimeHeaders, PartType};
use serde::Serialize;

/// Constant, content-free parse errors (no bytes/filenames are ever echoed).
const EMPTY_MESSAGE_ERROR: &str = "email message is empty";
pub(crate) const MIME_PARSE_ERROR: &str = "failed to parse MIME message";

/// Guard against pathological/degenerate part nesting. A well-formed message
/// never approaches this; it only bounds work on hostile input.
const MAX_MIME_DEPTH: usize = 32;

/// Which decoded MIME part supplied the viewer body.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MimeBodyKind {
    /// A `text/plain` body part was used.
    Plain,
    /// Only a `text/html` body part was available; its visible text was used.
    Html,
    /// No usable body part was present.
    Empty,
}

/// Content-free projection of a MIME message, safe to return across IPC.
///
/// Every field is either a count, an opaque handle, a content-free label, or
/// text that has already passed through the host PII engine. There is no field
/// that can hold an attachment filename, a filesystem path or unrevealed
/// message content.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MimeViewerProjection {
    /// Body text after tenant-scoped redaction. May contain opaque tokens, but
    /// never clear PII and never original HTML markup.
    pub body_token_only: String,
    /// Which body part supplied `body_token_only`.
    pub body_kind: MimeBodyKind,
    /// Number of MIME sections visited by the tree walk (content-free).
    pub section_count: usize,
    /// Number of attachment parts. No names, sizes or bytes are projected.
    pub attachment_count: usize,
    /// Deterministic, opaque display handles, one per attachment part. These
    /// are NOT viewer capabilities and grant no reveal authority.
    pub attachment_refs: Vec<String>,
}

/// Native, in-process parts produced by the MIME tree walk.
///
/// Never serialized. `attachments` intentionally carries filenames and bytes:
/// the existing internal pipeline (xberg extraction, redaction, RAG) needs
/// them. They are tokenized/replaced before any output or persistence step.
#[derive(Debug, Default)]
pub(crate) struct MimeParts {
    pub(crate) text_body: Option<String>,
    pub(crate) html_body: Option<String>,
    pub(crate) attachments: Vec<EmailAttachment>,
    /// Count of attachment parts, independent of whether their bytes were
    /// captured. The viewer path uses this without retaining any payload.
    pub(crate) attachment_count: usize,
    pub(crate) section_count: usize,
}

/// Whether the tree walk materializes attachment payloads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AttachmentCapture {
    /// Keep decoded bytes for the trusted processing pipeline (backend, xberg,
    /// redaction, RAG).
    Bytes,
    /// Count parts only. The viewer projection needs handles/metadata, never
    /// attachment bytes, so it must not clone or retain any payload.
    CountOnly,
}

#[derive(Debug, Default)]
struct Collector {
    text_fragments: Vec<String>,
    html_fragments: Vec<String>,
    attachments: Vec<EmailAttachment>,
    attachment_count: usize,
    section_count: usize,
}

impl Collector {
    /// Record an attachment part that was identified from the MIME tree.
    ///
    /// `content` is `None` when the attachment is a whole subtree (e.g. a
    /// `multipart/*` part marked as an attachment): in that case we keep its
    /// metadata but never serialize or copy any descendant body bytes.
    fn push_attachment(
        &mut self,
        part: &MessagePart<'_>,
        content: Option<&[u8]>,
        capture: AttachmentCapture,
    ) {
        self.attachment_count += 1;
        if capture == AttachmentCapture::CountOnly {
            // No filename, no bytes, no `EmailAttachment` allocation: the
            // viewer only needs the count to derive opaque handles.
            return;
        }
        let bytes = content.unwrap_or(&[]);
        let index = self.attachments.len();
        let filename = part
            .attachment_name()
            .map(str::to_owned)
            .unwrap_or_else(|| format!("attachment-{index}"));
        let content_type = part
            .content_type()
            .map(|ct| format!("{}/{}", ct.ctype(), ct.subtype().unwrap_or("octet-stream")))
            .unwrap_or_else(|| "application/octet-stream".to_owned());
        self.attachments.push(EmailAttachment {
            filename,
            content_type,
            size: bytes.len(),
            content_id: part.content_id().map(str::to_owned),
            content: bytes.to_vec(),
        });
    }

    fn finish(self) -> MimeParts {
        MimeParts {
            text_body: (!self.text_fragments.is_empty()).then(|| self.text_fragments.join("\n\n")),
            html_body: (!self.html_fragments.is_empty()).then(|| self.html_fragments.join("\n\n")),
            attachments: self.attachments,
            attachment_count: self.attachment_count,
            section_count: self.section_count,
        }
    }
}

/// True when a part is an attachment rather than a body candidate.
///
/// A part is an attachment when it is explicitly marked as one by
/// `Content-Disposition`, carries a filename (in disposition or Content-Type),
/// or has a non-text/non-container media type. This is what keeps an attached
/// `.txt` file out of the body where a flat `body_text(0)` heuristic would
/// happily include it.
fn is_attachment_part(part: &MessagePart<'_>) -> bool {
    if part
        .content_disposition()
        .is_some_and(|disposition| disposition.is_attachment())
    {
        return true;
    }
    if part.attachment_name().is_some() {
        return true;
    }
    part.content_type().is_some_and(|content_type| {
        !matches!(
            content_type.ctype().to_ascii_lowercase().as_str(),
            "text" | "multipart" | "message"
        )
    })
}

/// Recursively walk one MIME part, collecting body text and attachments.
fn walk(
    message: &Message<'_>,
    part: &MessagePart<'_>,
    depth: usize,
    out: &mut Collector,
    capture: AttachmentCapture,
) {
    if depth > MAX_MIME_DEPTH {
        return;
    }
    out.section_count += 1;

    match &part.body {
        PartType::Multipart(children) => {
            if is_attachment_part(part) {
                // A multipart/* part explicitly marked as an attachment (or
                // carrying a filename) is an opaque container. Its descendant
                // text is NOT body content: record metadata/handle only and
                // never recurse, so no child bytes reach the viewer body.
                // We pass no payload bytes at all — not even for the native
                // processing path.
                out.push_attachment(part, None, capture);
            } else {
                for id in children {
                    if let Some(child) = message.parts.get(*id as usize) {
                        walk(message, child, depth + 1, out, capture);
                    }
                }
            }
        }
        PartType::Message(nested) => {
            if is_attachment_part(part) {
                // An attached `.eml` (Content-Disposition: attachment or a
                // filename/name on message/rfc822) is an opaque attachment,
                // NOT body content. Its encapsulated text must never be
                // folded into the viewer body — that would surface sensitive
                // content from an attachment in a token-only projection.
                out.push_attachment(part, Some(nested.raw_message()), capture);
            } else {
                // An inline forwarded message has no attachment marker: walk
                // the encapsulated message's own tree. Its headers are not
                // body text; only its body parts are collected.
                walk(nested, nested.root_part(), depth + 1, out, capture);
            }
        }
        PartType::Text(text) => {
            if is_attachment_part(part) {
                out.push_attachment(part, Some(text.as_bytes()), capture);
            } else {
                out.text_fragments.push(text.to_string());
            }
        }
        PartType::Html(html) => {
            if is_attachment_part(part) {
                out.push_attachment(part, Some(html.as_bytes()), capture);
            } else {
                out.html_fragments.push(html.to_string());
            }
        }
        PartType::Binary(bytes) | PartType::InlineBinary(bytes) => {
            out.push_attachment(part, Some(bytes), capture);
        }
    }
}

/// Walk an already-parsed message's MIME tree into owned processing parts.
///
/// Used by the backend adapter so a message is parsed once: headers come from
/// the `mail_parser::Message` and body/attachments come from this tree walk.
pub(crate) fn parse_message_parts(message: &Message<'_>) -> MimeParts {
    collect(message, AttachmentCapture::Bytes)
}

fn collect(message: &Message<'_>, capture: AttachmentCapture) -> MimeParts {
    let mut collector = Collector::default();
    walk(message, message.root_part(), 0, &mut collector, capture);
    collector.finish()
}

/// Parse raw RFC 5322 bytes into native processing parts (bytes captured).
pub(crate) fn parse(raw: &[u8]) -> Result<MimeParts> {
    let message = parse_message(raw)?;
    Ok(parse_message_parts(&message))
}

/// Parse only what the viewer needs: body text, counts and section metadata.
/// Attachment payload bytes are never decoded into an owned copy or retained.
fn parse_viewer_parts(raw: &[u8]) -> Result<MimeParts> {
    let message = parse_message(raw)?;
    Ok(collect(&message, AttachmentCapture::CountOnly))
}

fn parse_message(raw: &[u8]) -> Result<Message<'_>> {
    if raw.is_empty() {
        return Err(EmailError::Backend(EMPTY_MESSAGE_ERROR.to_owned()));
    }
    let message = MessageParser::default()
        .parse(raw)
        .ok_or_else(|| EmailError::Backend(MIME_PARSE_ERROR.to_owned()))?;
    if message.parts.is_empty() {
        return Err(EmailError::Backend(MIME_PARSE_ERROR.to_owned()));
    }
    Ok(message)
}

/// Derive an opaque, deterministic display handle for one attachment part.
///
/// The seed must be an opaque message reference supplied by the trusted host.
/// The digest is one-way; neither the filename nor the bytes participate.
fn attachment_ref(seed: &str, ordinal: usize) -> String {
    let digest = blake3::hash(format!("mime-attachment:{seed}:{ordinal}").as_bytes()).to_hex();
    format!("mimeatt_{}", &digest[..24])
}

/// Parse and redact a message into a content-free viewer projection.
///
/// The body is taken from the first plain-text body part when present,
/// otherwise from the visible text of an HTML part (decoded with mail-parser's
/// HTML decoder, not a regex heuristic), then redacted through the host PII
/// engine as `caller`. `seed` should be an opaque message reference; it is used
/// only to derive stable attachment display handles.
///
/// The viewer path uses a count-only tree walk: attachment payloads are never
/// cloned into an owned buffer or retained in the returned projection.
pub async fn project_viewer(
    pii: &PiiEngine,
    caller: Caller<'_>,
    seed: &str,
    raw: &[u8],
) -> Result<MimeViewerProjection> {
    let parts = parse_viewer_parts(raw)?;
    let (body_kind, visible) = match (parts.text_body.as_deref(), parts.html_body.as_deref()) {
        (Some(text), _) => (MimeBodyKind::Plain, text.to_owned()),
        (None, Some(html)) => (
            MimeBodyKind::Html,
            mail_parser::decoders::html::html_to_text(html),
        ),
        (None, None) => (MimeBodyKind::Empty, String::new()),
    };
    let body_token_only = if visible.is_empty() {
        String::new()
    } else {
        pii.redact(caller, &visible, &[], "", false).await?.0
    };
    let attachment_refs = (0..parts.attachment_count)
        .map(|ordinal| attachment_ref(seed, ordinal))
        .collect();
    Ok(MimeViewerProjection {
        body_token_only,
        body_kind,
        section_count: parts.section_count,
        attachment_count: parts.attachment_count,
        attachment_refs,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use hacienda_core::auth::{AuthContext, Capability, CapabilitySet};

    /// Synthetic PII fixtures. These are invented values, not real data.
    const CLEAR_EMAIL: &str = "jane.doe@acme.example";
    const CLEAR_SSN: &str = "123-45-6789";
    const CLEAR_FILENAME: &str = "jane-doe-contract.pdf";

    fn caller() -> AuthContext {
        AuthContext::with_tenant(
            "test",
            hacienda_core::tenancy::TenantId::default_tenant(),
            CapabilitySet::new([Capability::DocumentsProcess]),
        )
    }

    fn pii() -> PiiEngine {
        PiiEngine::for_local_testing()
    }

    fn assert_no_clear_fixtures(value: &str) {
        for fixture in [CLEAR_EMAIL, CLEAR_SSN, CLEAR_FILENAME] {
            assert!(
                !value.contains(fixture),
                "clear fixture leaked into viewer projection: {fixture}\nvalue:\n{value}"
            );
        }
    }

    fn projection_json(projection: &MimeViewerProjection) -> String {
        serde_json::to_string(projection).expect("projection serializes")
    }

    const ALTERNATIVE: &[u8] = concat!(
        "From: Jane Doe <jane.doe@acme.example>\r\n",
        "To: Richard Roe <richard.roe@acme.example>\r\n",
        "Subject: Contract 123-45-6789\r\n",
        "MIME-Version: 1.0\r\n",
        "Content-Type: multipart/alternative; boundary=\"alt\"\r\n",
        "\r\n",
        "--alt\r\n",
        "Content-Type: text/plain; charset=utf-8\r\n",
        "\r\n",
        "Plain body for jane.doe@acme.example 123-45-6789\r\n",
        "--alt\r\n",
        "Content-Type: text/html; charset=utf-8\r\n",
        "\r\n",
        "<html><body><p>HTML body <b>SSN 123-45-6789</b></p></body></html>\r\n",
        "--alt--\r\n"
    )
    .as_bytes();

    const MIXED: &[u8] = concat!(
        "MIME-Version: 1.0\r\n",
        "Content-Type: multipart/mixed; boundary=\"mix\"\r\n",
        "\r\n",
        "--mix\r\n",
        "Content-Type: text/plain; charset=utf-8\r\n",
        "\r\n",
        "Body text jane.doe@acme.example\r\n",
        "--mix\r\n",
        "Content-Type: application/pdf; name=\"jane-doe-contract.pdf\"\r\n",
        "Content-Disposition: attachment; filename=\"jane-doe-contract.pdf\"\r\n",
        "Content-Transfer-Encoding: base64\r\n",
        "\r\n",
        "JVBERi0xLjQK\r\n",
        "--mix--\r\n"
    )
    .as_bytes();

    const NESTED: &[u8] = concat!(
        "MIME-Version: 1.0\r\n",
        "Content-Type: multipart/mixed; boundary=\"outer\"\r\n",
        "\r\n",
        "--outer\r\n",
        "Content-Type: multipart/alternative; boundary=\"inner\"\r\n",
        "\r\n",
        "--inner\r\n",
        "Content-Type: text/plain\r\n",
        "\r\n",
        "Nested plain jane.doe@acme.example\r\n",
        "--inner\r\n",
        "Content-Type: text/html\r\n",
        "\r\n",
        "<p>Nested html 123-45-6789</p>\r\n",
        "--inner--\r\n",
        "--outer\r\n",
        "Content-Type: message/rfc822\r\n",
        "\r\n",
        "From: someone@example.org\r\n",
        "Content-Type: text/plain\r\n",
        "\r\n",
        "Forwarded body jane.doe@acme.example\r\n",
        "--outer--\r\n"
    )
    .as_bytes();

    /// Distinctive non-PII marker that would survive redaction if an attached
    /// `.eml` were ever folded into the viewer body by the tree walk.
    const EML_MARKER: &str = "ATTACHED_EML_SECRET_MARKER";

    const EML_ATTACHMENT: &[u8] = concat!(
        "MIME-Version: 1.0\r\n",
        "Content-Type: multipart/mixed; boundary=\"mail\"\r\n",
        "\r\n",
        "--mail\r\n",
        "Content-Type: text/plain\r\n",
        "\r\n",
        "Outer body is safe to display.\r\n",
        "--mail\r\n",
        "Content-Type: message/rfc822; name=\"sensitive.eml\"\r\n",
        "Content-Disposition: attachment; filename=\"sensitive.eml\"\r\n",
        "\r\n",
        "From: Attacker <attacker@example.org>\r\n",
        "Subject: Attached secret\r\n",
        "Content-Type: text/plain\r\n",
        "\r\n",
        "ATTACHED_EML_SECRET_MARKER jane.doe@acme.example 123-45-6789\r\n",
        "--mail--\r\n"
    )
    .as_bytes();

    /// Distinctive marker carried by the TEXT CHILD of an attachment-marked
    /// multipart container. If the walk descends into it, this leaks.
    const MULTIPART_MARKER: &str = "MULTIPART_ATTACHMENT_SECRET_MARKER";

    const MULTIPART_ATTACHMENT: &[u8] = concat!(
        "MIME-Version: 1.0\r\n",
        "Content-Type: multipart/mixed; boundary=\"top\"\r\n",
        "\r\n",
        "--top\r\n",
        "Content-Type: text/plain\r\n",
        "\r\n",
        "Outer body is safe to display.\r\n",
        "--top\r\n",
        "Content-Type: multipart/mixed; boundary=\"inner\"\r\n",
        "Content-Disposition: attachment; filename=\"nested-archive\"\r\n",
        "\r\n",
        "--inner\r\n",
        "Content-Type: text/plain\r\n",
        "\r\n",
        "MULTIPART_ATTACHMENT_SECRET_MARKER jane.doe@acme.example 123-45-6789\r\n",
        "--inner--\r\n",
        "--top--\r\n"
    )
    .as_bytes();

    #[test]
    fn multipart_alternative_collects_both_body_parts() {
        let parts = parse(ALTERNATIVE).expect("alternative message parses");
        let text = parts.text_body.expect("plain part collected");
        let html = parts.html_body.expect("html part collected");
        assert!(text.contains("Plain body"), "unexpected text: {text}");
        assert!(html.contains("<html"), "unexpected html: {html}");
        assert!(parts.attachments.is_empty());
        assert!(parts.section_count >= 3);
    }

    #[tokio::test]
    async fn multipart_alternative_projection_prefers_plain_and_redacts() {
        let projection = project_viewer(&pii(), Caller::Principal(&caller()), "msg-1", ALTERNATIVE)
            .await
            .expect("projection succeeds");
        assert_eq!(projection.body_kind, MimeBodyKind::Plain);
        assert!(projection.body_token_only.contains("Plain body"));
        assert!(!projection.body_token_only.contains("<html"));
        assert_eq!(projection.attachment_count, 0);
        assert!(projection.attachment_refs.is_empty());
        assert_no_clear_fixtures(&projection_json(&projection));
    }

    #[test]
    fn multipart_mixed_keeps_attachments_out_of_the_body() {
        let parts = parse(MIXED).expect("mixed message parses");
        assert!(parts.text_body.unwrap().contains("Body text"));
        assert_eq!(parts.attachments.len(), 1, "exactly one attachment");
        assert_eq!(parts.attachments[0].filename, CLEAR_FILENAME);
        assert!(parts.attachments[0]
            .content_type
            .starts_with("application/pdf"));
        assert!(!parts.attachments[0].content.is_empty());
    }

    #[tokio::test]
    async fn multipart_mixed_projection_has_no_filename_or_content() {
        let projection = project_viewer(&pii(), Caller::Principal(&caller()), "msg-2", MIXED)
            .await
            .expect("projection succeeds");
        assert_eq!(projection.attachment_count, 1);
        assert_eq!(projection.attachment_refs.len(), 1);
        assert!(
            projection.attachment_refs[0].starts_with("mimeatt_"),
            "attachment handle must be opaque"
        );
        let json = projection_json(&projection);
        assert_no_clear_fixtures(&json);
        assert!(
            !json.contains("JVBERi0"),
            "no attachment bytes in projection"
        );
        // The handle is a one-way digest of the seed/ordinal, not the filename.
        let other_seed = project_viewer(&pii(), Caller::Principal(&caller()), "msg-3", MIXED)
            .await
            .expect("projection succeeds");
        assert_ne!(projection.attachment_refs, other_seed.attachment_refs);
    }

    #[test]
    fn viewer_path_counts_attachments_without_retaining_payload_bytes() {
        let viewer = parse_viewer_parts(MIXED).expect("viewer summary parses");
        assert_eq!(viewer.attachment_count, 1);
        assert!(
            viewer.attachments.is_empty(),
            "viewer tree walk must not clone or retain attachment bytes"
        );
        assert!(viewer.text_body.unwrap().contains("Body text"));

        // The trusted processing path still receives the real bytes.
        let native = parse(MIXED).expect("native parse");
        assert_eq!(native.attachments.len(), 1);
        assert!(!native.attachments[0].content.is_empty());
    }

    #[test]
    fn attached_eml_is_an_attachment_not_body_text() {
        let parts = parse(EML_ATTACHMENT).expect("message with .eml attachment parses");
        let text = parts.text_body.as_deref().expect("outer body present");
        assert!(text.contains("Outer body is safe to display."));
        assert!(
            !text.contains(EML_MARKER),
            "attached .eml was folded into text_body: {text}"
        );
        assert_eq!(parts.attachments.len(), 1);
        assert_eq!(parts.attachments[0].filename, "sensitive.eml");
        assert!(parts.attachments[0]
            .content_type
            .starts_with("message/rfc822"));
        assert!(
            String::from_utf8_lossy(&parts.attachments[0].content).contains(EML_MARKER),
            "native attachment content must still be available to the trusted pipeline"
        );
    }

    #[tokio::test]
    async fn attached_eml_secret_never_appears_in_viewer_projection() {
        let projection = project_viewer(
            &pii(),
            Caller::Principal(&caller()),
            "msg-eml",
            EML_ATTACHMENT,
        )
        .await
        .expect("projection succeeds");
        assert_eq!(projection.body_kind, MimeBodyKind::Plain);
        assert_eq!(projection.attachment_count, 1);
        assert_eq!(projection.attachment_refs.len(), 1);
        assert!(
            !projection.body_token_only.contains(EML_MARKER),
            "attached .eml body leaked into viewer text: {}",
            projection.body_token_only
        );
        let json = projection_json(&projection);
        assert!(
            !json.contains(EML_MARKER),
            "secret in projection JSON: {json}"
        );
        assert!(
            !json.contains("sensitive.eml"),
            "filename in projection JSON: {json}"
        );
        assert_no_clear_fixtures(&json);
    }

    #[test]
    fn attachment_marked_multipart_is_not_descended_into() {
        let parts = parse(MULTIPART_ATTACHMENT).expect("multipart attachment message parses");
        let text = parts.text_body.as_deref().expect("outer body present");
        assert!(text.contains("Outer body is safe to display."));
        assert!(
            !text.contains(MULTIPART_MARKER),
            "attachment-marked multipart child leaked into text_body: {text}"
        );
        assert_eq!(parts.attachment_count, 1);
        assert_eq!(parts.attachments.len(), 1);
        assert_eq!(parts.attachments[0].filename, "nested-archive");
        assert!(
            parts.attachments[0].content.is_empty(),
            "subtree attachment must never copy child body bytes"
        );
    }

    #[tokio::test]
    async fn multipart_attachment_child_and_filename_never_reach_projection() {
        let projection = project_viewer(
            &pii(),
            Caller::Principal(&caller()),
            "msg-multipart",
            MULTIPART_ATTACHMENT,
        )
        .await
        .expect("projection succeeds");
        assert_eq!(projection.body_kind, MimeBodyKind::Plain);
        assert_eq!(projection.attachment_count, 1);
        assert_eq!(projection.attachment_refs.len(), 1);
        assert!(
            !projection.body_token_only.contains(MULTIPART_MARKER),
            "multipart attachment child body leaked: {}",
            projection.body_token_only
        );
        let json = projection_json(&projection);
        assert!(
            !json.contains(MULTIPART_MARKER),
            "child secret in projection JSON: {json}"
        );
        assert!(
            !json.contains("nested-archive"),
            "attachment filename in projection JSON: {json}"
        );
        assert_no_clear_fixtures(&json);

        // The viewer path counts the part but retains no attachment payload.
        let viewer = parse_viewer_parts(MULTIPART_ATTACHMENT).expect("viewer summary parses");
        assert_eq!(viewer.attachment_count, 1);
        assert!(viewer.attachments.is_empty());
    }

    #[test]
    fn nested_mime_is_walked_recursively() {
        let parts = parse(NESTED).expect("nested message parses");
        let text = parts.text_body.expect("nested text collected");
        assert!(text.contains("Nested plain"), "unexpected text: {text}");
        assert!(
            text.contains("Forwarded body"),
            "nested rfc822 body was skipped: {text}"
        );
        assert!(
            parts.section_count >= 6,
            "section count too small: {}",
            parts.section_count
        );
        assert!(parts.attachments.is_empty());
    }

    #[tokio::test]
    async fn nested_mime_projection_never_leaks_clear_pii() {
        let projection = project_viewer(&pii(), Caller::Principal(&caller()), "msg-4", NESTED)
            .await
            .expect("projection succeeds");
        let json = projection_json(&projection);
        assert_no_clear_fixtures(&json);
        assert!(projection.section_count >= 6);
    }

    #[test]
    fn malformed_input_is_content_free_and_never_panics() {
        assert!(parse(&[]).is_err(), "empty input must fail closed");
        let truncated = b"Content-Type: multipart/mixed; boundary=\"gone\"\r\n\r\n--gone\r\nContent-Type: text/plain\r\n\r\ncontact jane.doe@acme.example 123-45-6789";
        let parts = parse(truncated).expect("lenient parser still yields a tree");
        // A truncated boundary may be treated as a body or an attachment; what
        // matters is that no PII escapes the projection and nothing panics.
        let projection = futures::executor::block_on(project_viewer(
            &pii(),
            Caller::Principal(&caller()),
            "msg-5",
            truncated,
        ))
        .expect("projection succeeds");
        let json = projection_json(&projection);
        assert_no_clear_fixtures(&json);
        assert_eq!(projection.attachment_count, parts.attachments.len());
    }

    #[tokio::test]
    async fn redaction_replaces_pii_in_the_body_only() {
        let projection = project_viewer(&pii(), Caller::Principal(&caller()), "msg-6", ALTERNATIVE)
            .await
            .expect("projection succeeds");
        assert_ne!(
            projection.body_token_only,
            "Plain body for jane.doe@acme.example 123-45-6789"
        );
        assert_no_clear_fixtures(&projection.body_token_only);
        // Serialized projection is the IPC boundary: it must carry no fixture.
        assert_no_clear_fixtures(&projection_json(&projection));
    }
}
