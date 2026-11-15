//! Token-only projections for the redacted document editor.
//!
//! The projection is deliberately a separate boundary from `ExtractedDocument`: Xberg
//! carries rich extractor metadata, and the Hacienda structured-field walk must cover
//! metadata such as title, subject, and `additional` before this module runs. This module
//! copies only the already-redacted body plus opaque token references from those
//! fields. It never attempts to recover or re-run source offsets after redaction.

use crate::pii::EntitySource;
use crate::redaction::RedactionMode;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

/// Current wire version of [`EditorEnvelope`].
pub const EDITOR_PROJECTION_VERSION: u16 = 1;

const EDITOR_SCHEMA: &str = "hacienda.editor.v1";
const HACIENDA_TOKEN_MODE: &str = "pseudonymize";
const NOT_DISCLOSED: &str = "not_disclosed";

/// Opaque references supplied by the persistence/API layer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EditorProjectionOptions {
    /// Stable document reference. The projection does not interpret this value.
    pub document_ref: String,
    /// Stable version reference. The projection does not interpret this value.
    pub version_ref: String,
    /// Source label used in token provenance (normally `hacienda`).
    #[serde(default = "default_source")]
    pub source: String,
    /// Redaction mode used for this document, if known.
    #[serde(default)]
    pub redaction_mode: Option<RedactionMode>,
    /// Source-text provenance supplied by the redaction pipeline.
    ///
    /// `start` and `end` refer to the pre-redaction source only. They are never used to
    /// locate or decorate a character in the editor document.
    #[serde(default)]
    pub redactions: Vec<SourceRedactionProvenance>,
}

impl EditorProjectionOptions {
    /// Construct options with the normal Hacienda source label.
    pub fn new(document_ref: impl Into<String>, version_ref: impl Into<String>) -> Self {
        Self {
            document_ref: document_ref.into(),
            version_ref: version_ref.into(),
            source: default_source(),
            redaction_mode: None,
            redactions: Vec::new(),
        }
    }

    /// Set the redaction mode recorded in provenance.
    #[must_use]
    pub fn with_redaction_mode(mut self, mode: RedactionMode) -> Self {
        self.redaction_mode = Some(mode);
        self
    }

    /// Set a source label recorded on sensitive marks.
    #[must_use]
    pub fn with_source(mut self, source: impl Into<String>) -> Self {
        self.source = source.into();
        self
    }

    /// Attach source-only redaction provenance.
    #[must_use]
    pub fn with_redactions(mut self, redactions: Vec<SourceRedactionProvenance>) -> Self {
        self.redactions = redactions;
        self
    }
}

fn default_source() -> String {
    "hacienda".to_owned()
}

/// A source-text redaction record safe to send alongside an editor document.
///
/// This type intentionally contains no matched text, hashes, or metadata values. Offsets
/// are evidence about the source that was redacted, not offsets into the editor projection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceRedactionProvenance {
    pub token_class: String,
    pub token_id: Option<String>,
    pub start: u32,
    pub end: u32,
    pub source: EntitySource,
    pub mode: RedactionMode,
}

/// Serializable envelope consumed by a Tiptap/ProseMirror client.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EditorEnvelope {
    pub schema: String,
    pub projection_version: u16,
    pub document_ref: String,
    pub version_ref: String,
    /// ProseMirror JSON (`doc` containing paragraph and text nodes).
    pub editor: Value,
    pub search: SearchProjection,
    pub metadata: TokenOnlyMetadata,
    pub provenance: ProjectionProvenance,
}

impl EditorEnvelope {
    /// Build an envelope from a redacted Xberg document.
    pub fn from_document(
        document: &xberg::ExtractedDocument,
        options: &EditorProjectionOptions,
    ) -> Self {
        project_document(document, options)
    }
}

/// Token-only search data. Search consumers can match opaque IDs/equality without ever
/// receiving a plaintext index.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct SearchProjection {
    pub tokens: Vec<SearchToken>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchToken {
    pub token_class: String,
    pub token_id: String,
    pub mode: String,
    pub source: String,
    pub disclosure_state: String,
    pub occurrences: u32,
}

/// Metadata fields whose values are represented only by Hacienda tokens.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct TokenOnlyMetadata {
    pub title: Vec<SensitiveToken>,
    pub filename: Vec<SensitiveToken>,
    pub authors: Vec<Vec<SensitiveToken>>,
    pub subject: Vec<SensitiveToken>,
    pub keywords: Vec<Vec<SensitiveToken>>,
    pub attachments: Vec<TokenOnlyAttachment>,
    pub created_by: Vec<SensitiveToken>,
    pub modified_by: Vec<SensitiveToken>,
    pub category: Vec<SensitiveToken>,
    pub tags: Vec<Vec<SensitiveToken>>,
    pub document_version: Vec<SensitiveToken>,
    pub abstract_text: Vec<SensitiveToken>,
    pub additional: BTreeMap<String, Vec<SensitiveToken>>,
    /// Non-sensitive structural facts useful to legal-document clients.
    pub legal: LegalMetadata,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct TokenOnlyAttachment {
    pub name: Vec<SensitiveToken>,
    pub filename: Vec<SensitiveToken>,
    pub mime_type: Option<String>,
    pub size: Option<u64>,
    pub is_image: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct LegalMetadata {
    pub mime_type: String,
    pub language: Option<String>,
    pub page_count: usize,
    pub table_count: usize,
    pub image_count: usize,
    pub extraction_method: Option<String>,
    pub ocr_used: bool,
    pub format_kind: Option<String>,
}

/// Provenance for the projection and source-only redaction records.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectionProvenance {
    pub extractor: String,
    pub redactor: String,
    pub redaction_mode: Option<RedactionMode>,
    pub offsets_are_source_only: bool,
    pub redactions: Vec<SourceRedactionProvenance>,
}

/// A token reference used in marks, metadata, and search results.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SensitiveToken {
    /// The opaque token text itself, not the value it represents.
    pub token_id: String,
    pub token_class: String,
    pub mode: String,
    pub source: String,
    pub disclosure_state: String,
}

/// Convert an already Hacienda-redacted Xberg document into an editor envelope.
///
/// The body is copied as redacted text and token occurrences are marked in ProseMirror
/// JSON. Metadata is stricter: only Hacienda token occurrences are retained, so an
/// unredacted title, filename, author, subject, keyword, or attachment name cannot leak
/// through this boundary.
pub fn project_document(
    document: &xberg::ExtractedDocument,
    options: &EditorProjectionOptions,
) -> EditorEnvelope {
    let mut token_index = TokenIndex::default();
    let editor = prose_mirror_document(&document.content, options, &mut token_index);
    let metadata = token_only_metadata(document, &mut token_index);
    let search = SearchProjection {
        tokens: token_index.into_search_tokens(),
    };

    EditorEnvelope {
        schema: EDITOR_SCHEMA.to_owned(),
        projection_version: EDITOR_PROJECTION_VERSION,
        document_ref: options.document_ref.clone(),
        version_ref: options.version_ref.clone(),
        editor,
        search,
        metadata,
        provenance: ProjectionProvenance {
            extractor: "xberg".to_owned(),
            redactor: "hacienda".to_owned(),
            redaction_mode: options.redaction_mode,
            offsets_are_source_only: true,
            redactions: options.redactions.clone(),
        },
    }
}

/// Alias with a descriptive name for callers that prefer the output type as the verb.
pub fn build_editor_projection(
    document: &xberg::ExtractedDocument,
    options: &EditorProjectionOptions,
) -> EditorEnvelope {
    project_document(document, options)
}

/// Convenience form for callers that only have opaque document/version references.
pub fn project_document_with_refs(
    document: &xberg::ExtractedDocument,
    document_ref: impl Into<String>,
    version_ref: impl Into<String>,
) -> EditorEnvelope {
    project_document(
        document,
        &EditorProjectionOptions::new(document_ref, version_ref),
    )
}

#[derive(Default)]
struct TokenIndex {
    tokens: BTreeMap<String, SearchToken>,
}

impl TokenIndex {
    fn record(&mut self, token: &SensitiveToken) {
        self.tokens
            .entry(token.token_id.clone())
            .and_modify(|existing| existing.occurrences += 1)
            .or_insert_with(|| SearchToken {
                token_class: token.token_class.clone(),
                token_id: token.token_id.clone(),
                mode: token.mode.clone(),
                source: token.source.clone(),
                disclosure_state: token.disclosure_state.clone(),
                occurrences: 1,
            });
    }

    fn into_search_tokens(self) -> Vec<SearchToken> {
        self.tokens.into_values().collect()
    }
}

fn token_from_text(token: &str, source: &str) -> Option<SensitiveToken> {
    let inner = token.strip_prefix('[')?.strip_suffix(']')?;
    let mut fields = inner.split(':');
    let token_class = fields.next()?.to_owned();
    let key_id = fields.next()?;
    let body = fields.next()?;
    if fields.next().is_some()
        || token_class.is_empty()
        || !token_class
            .bytes()
            .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'_')
        || key_id.is_empty()
        || !key_id
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
        || body.is_empty()
        || !body
            .bytes()
            .all(|byte| byte.is_ascii_uppercase() || (b'2'..=b'7').contains(&byte))
    {
        return None;
    }
    Some(SensitiveToken {
        token_id: token.to_owned(),
        token_class,
        mode: HACIENDA_TOKEN_MODE.to_owned(),
        source: source.to_owned(),
        disclosure_state: NOT_DISCLOSED.to_owned(),
    })
}

fn token_spans(text: &str, source: &str) -> Vec<(usize, usize, SensitiveToken)> {
    let mut found = Vec::new();
    let mut cursor = 0;
    while let Some(relative_start) = text[cursor..].find('[') {
        let start = cursor + relative_start;
        let Some(relative_end) = text[start..].find(']') else {
            break;
        };
        let end = start + relative_end + 1;
        if let Some(token) = token_from_text(&text[start..end], source) {
            found.push((start, end, token));
        }
        cursor = end;
    }
    found
}

fn prose_mirror_document(
    content: &str,
    options: &EditorProjectionOptions,
    index: &mut TokenIndex,
) -> Value {
    let paragraphs = content
        .split('\n')
        .map(|line| {
            let mut nodes = Vec::new();
            let spans = token_spans(line, &options.source);
            let mut cursor = 0;
            for (start, end, token) in spans {
                if start > cursor {
                    nodes.push(serde_json::json!({
                        "type": "text",
                        "text": &line[cursor..start]
                    }));
                }
                index.record(&token);
                nodes.push(serde_json::json!({
                    "type": "text",
                    "text": &line[start..end],
                    "marks": [{
                        "type": "sensitive",
                        "attrs": {
                            "tokenClass": token.token_class,
                            "tokenId": token.token_id,
                            "mode": token.mode,
                            "source": token.source,
                            "disclosureState": token.disclosure_state
                        }
                    }]
                }));
                cursor = end;
            }
            if cursor < line.len() {
                nodes.push(serde_json::json!({
                    "type": "text",
                    "text": &line[cursor..]
                }));
            }
            let mut paragraph = serde_json::json!({ "type": "paragraph" });
            if !nodes.is_empty() {
                paragraph["content"] = Value::Array(nodes);
            }
            paragraph
        })
        .collect::<Vec<_>>();
    serde_json::json!({ "type": "doc", "content": paragraphs })
}

fn token_list(value: Option<&str>, source: &str, index: &mut TokenIndex) -> Vec<SensitiveToken> {
    let mut tokens = Vec::new();
    if let Some(value) = value {
        for (_, _, token) in token_spans(value, source) {
            index.record(&token);
            tokens.push(token);
        }
    }
    tokens
}

fn token_lists(
    values: Option<&[String]>,
    source: &str,
    index: &mut TokenIndex,
) -> Vec<Vec<SensitiveToken>> {
    values
        .into_iter()
        .flatten()
        .map(|value| token_list(Some(value), source, index))
        .collect()
}

fn token_only_metadata(
    document: &xberg::ExtractedDocument,
    index: &mut TokenIndex,
) -> TokenOnlyMetadata {
    let metadata = &document.metadata;
    let title = token_list(metadata.title.as_deref(), "xberg.metadata.title", index);
    let subject = token_list(metadata.subject.as_deref(), "xberg.metadata.subject", index);
    let authors = token_lists(metadata.authors.as_deref(), "xberg.metadata.authors", index);
    let keywords = token_lists(
        metadata.keywords.as_deref(),
        "xberg.metadata.keywords",
        index,
    );
    let created_by = token_list(
        metadata.created_by.as_deref(),
        "xberg.metadata.createdBy",
        index,
    );
    let modified_by = token_list(
        metadata.modified_by.as_deref(),
        "xberg.metadata.modifiedBy",
        index,
    );
    let category = token_list(
        metadata.category.as_deref(),
        "xberg.metadata.category",
        index,
    );
    let tags = token_lists(metadata.tags.as_deref(), "xberg.metadata.tags", index);
    let document_version = token_list(
        metadata.document_version.as_deref(),
        "xberg.metadata.documentVersion",
        index,
    );
    let abstract_text = token_list(
        metadata.abstract_text.as_deref(),
        "xberg.metadata.abstractText",
        index,
    );

    let additional_value = serde_json::to_value(&metadata.additional).unwrap_or(Value::Null);
    let filename = object_tokens(
        &additional_value,
        "filename",
        "xberg.metadata.filename",
        index,
    );
    let mut additional = BTreeMap::new();
    if let Value::Object(values) = &additional_value {
        for (key, value) in values {
            let tokens = tokens_in_value(value, &format!("xberg.metadata.additional.{key}"), index);
            if !tokens.is_empty() {
                additional.insert(key.clone(), tokens);
            }
        }
    }

    let format_value = serde_json::to_value(&metadata.format).unwrap_or(Value::Null);
    let attachments = attachment_values(&format_value, index);
    let format_kind = format_value
        .get("format_type")
        .and_then(Value::as_str)
        .map(str::to_owned);

    TokenOnlyMetadata {
        title,
        filename,
        authors,
        subject,
        keywords,
        attachments,
        created_by,
        modified_by,
        category,
        tags,
        document_version,
        abstract_text,
        additional,
        legal: LegalMetadata {
            mime_type: document.mime_type.to_string(),
            language: metadata.language.clone(),
            page_count: document.counts.pages,
            table_count: document.counts.tables,
            image_count: document.counts.images,
            extraction_method: document
                .extraction_method
                .map(|method| method.as_str().to_owned()),
            ocr_used: metadata.ocr_used,
            format_kind,
        },
    }
}

fn object_tokens(
    value: &Value,
    key: &str,
    source: &str,
    index: &mut TokenIndex,
) -> Vec<SensitiveToken> {
    value
        .get(key)
        .and_then(Value::as_str)
        .map(|text| token_list(Some(text), source, index))
        .unwrap_or_default()
}

fn tokens_in_value(value: &Value, source: &str, index: &mut TokenIndex) -> Vec<SensitiveToken> {
    match value {
        Value::String(text) => token_list(Some(text), source, index),
        Value::Array(values) => values
            .iter()
            .flat_map(|value| tokens_in_value(value, source, index))
            .collect(),
        Value::Object(values) => values
            .iter()
            .flat_map(|(key, value)| tokens_in_value(value, &format!("{source}.{key}"), index))
            .collect(),
        Value::Null | Value::Bool(_) | Value::Number(_) => Vec::new(),
    }
}

fn attachment_values(value: &Value, index: &mut TokenIndex) -> Vec<TokenOnlyAttachment> {
    let Some(attachments) = value.get("attachments").and_then(Value::as_array) else {
        return Vec::new();
    };
    attachments
        .iter()
        .filter_map(|attachment| {
            if let Some(filename) = attachment.as_str() {
                let filename =
                    token_list(Some(filename), "xberg.metadata.attachment.filename", index);
                return (!filename.is_empty()).then_some(TokenOnlyAttachment {
                    name: Vec::new(),
                    filename,
                    mime_type: None,
                    size: None,
                    is_image: None,
                });
            }
            let object = attachment.as_object()?;
            let name = object
                .get("name")
                .and_then(Value::as_str)
                .map(|value| token_list(Some(value), "xberg.metadata.attachment.name", index))
                .unwrap_or_default();
            let filename = object
                .get("filename")
                .and_then(Value::as_str)
                .map(|value| token_list(Some(value), "xberg.metadata.attachment.filename", index))
                .unwrap_or_default();
            let mime_type = object
                .get("mime_type")
                .and_then(Value::as_str)
                .map(str::to_owned);
            let size = object.get("size").and_then(Value::as_u64);
            let is_image = object.get("is_image").and_then(Value::as_bool);
            if name.is_empty() && filename.is_empty() && mime_type.is_none() && size.is_none() {
                None
            } else {
                Some(TokenOnlyAttachment {
                    name,
                    filename,
                    mime_type,
                    size,
                    is_image,
                })
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::borrow::Cow;

    const RAW_NAME: &str = "Jane Doe";
    const TOKEN: &str = "[PERSON:session:MFRGG2LTMVRXEZLU]";

    fn fixture() -> xberg::ExtractedDocument {
        let mut document = xberg::ExtractedDocument::default();
        document.content = format!("Agreement with {TOKEN}.");
        document.mime_type = Cow::Borrowed("application/pdf");
        document.metadata.title = Some(RAW_NAME.to_owned());
        document.metadata.subject = Some("Confidential matter".to_owned());
        document.metadata.authors = Some(vec![RAW_NAME.to_owned()]);
        document.metadata.keywords = Some(vec!["Acquisition".to_owned(), TOKEN.to_owned()]);
        document.metadata.format = Some(xberg::FormatMetadata::Email(xberg::EmailMetadata {
            attachments: vec!["unredacted-contract.pdf".to_owned(), TOKEN.to_owned()],
            ..Default::default()
        }));
        document
            .metadata
            .additional
            .insert(Cow::Borrowed("filename"), json!("client-identity.pdf"));
        document
            .metadata
            .additional
            .insert(Cow::Borrowed("owner_token"), json!(TOKEN));
        document
    }

    #[test]
    fn envelope_is_versioned_and_keeps_fixture_pii_out_of_metadata() {
        let envelope = project_document(
            &fixture(),
            &EditorProjectionOptions::new("doc-opaque", "version-opaque"),
        );
        let serialized = serde_json::to_string(&envelope).unwrap();
        assert_eq!(envelope.schema, EDITOR_SCHEMA);
        assert_eq!(envelope.projection_version, EDITOR_PROJECTION_VERSION);
        assert!(
            !serialized.contains(RAW_NAME),
            "plaintext name leaked: {serialized}"
        );
        assert!(
            !serialized.contains("Confidential matter"),
            "plaintext subject leaked"
        );
        assert!(
            !serialized.contains("client-identity.pdf"),
            "plaintext filename leaked"
        );
        assert!(
            !serialized.contains("unredacted-contract.pdf"),
            "plaintext attachment leaked"
        );
        assert!(
            serialized.contains("owner_token"),
            "token-bearing metadata was lost"
        );
        assert!(
            !serialized.contains("Acquisition"),
            "plaintext keyword leaked"
        );
    }

    #[test]
    fn token_marks_and_search_projection_are_token_only() {
        let envelope = project_document(
            &fixture(),
            &EditorProjectionOptions::new("doc-opaque", "version-opaque")
                .with_source("hacienda-redaction"),
        );
        let serialized = serde_json::to_string(&envelope).unwrap();
        assert!(serialized.contains("\"type\":\"sensitive\""));
        assert!(serialized.contains("\"tokenClass\":\"PERSON\""));
        assert!(serialized.contains("\"tokenId\":\"[PERSON:session:MFRGG2LTMVRXEZLU]\""));
        assert_eq!(envelope.search.tokens.len(), 1);
        assert_eq!(envelope.search.tokens[0].occurrences, 4);
        assert!(envelope
            .metadata
            .keywords
            .iter()
            .flatten()
            .any(|token| token.token_id == TOKEN));
        assert_eq!(envelope.metadata.attachments.len(), 1);
        assert_eq!(envelope.metadata.attachments[0].filename[0].token_id, TOKEN);
    }

    #[test]
    fn provenance_offsets_are_explicitly_source_only() {
        let mut options = EditorProjectionOptions::new("d", "v");
        options.redactions.push(SourceRedactionProvenance {
            token_class: "PERSON".to_owned(),
            token_id: Some(TOKEN.to_owned()),
            start: 14,
            end: 22,
            source: EntitySource::Model,
            mode: RedactionMode::Pseudonymize,
        });
        let envelope = project_document(&fixture(), &options);
        assert!(envelope.provenance.offsets_are_source_only);
        assert_eq!(envelope.provenance.redactions[0].start, 14);
        let json = serde_json::to_value(envelope).unwrap();
        assert_eq!(json["provenance"]["redactions"][0]["start"], 14);
        assert!(json["provenance"].get("editorOffsets").is_none());
    }

    #[test]
    fn malformed_bracketed_values_are_not_treated_as_hacienda_tokens() {
        assert!(token_from_text("[PERSON]", "test").is_none());
        assert!(token_from_text("[PERSON:session:MFRGG2LTMVRXEZLU]", "test").is_some());
        assert!(token_from_text("[person:session:ABC]", "test").is_none());
    }
}
