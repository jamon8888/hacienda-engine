//! Serialise an [`AuditChain`] for hand-off to auditors.

use crate::audit::chain::AuditChain;
use crate::audit::error::AuditError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportFormat {
    JsonLines,
    Json,
    Csv,
}

/// Export the chain in `format`.
///
/// # Errors
///
/// Returns [`AuditError::Json`] if an entry cannot be serialised.
pub fn export(chain: &AuditChain, format: ExportFormat) -> Result<Vec<u8>, AuditError> {
    match format {
        ExportFormat::JsonLines => export_json_lines(chain),
        ExportFormat::Json => export_json(chain),
        ExportFormat::Csv => export_csv(chain),
    }
}

/// One JSON object per line, in chain order.
pub fn export_json_lines(chain: &AuditChain) -> Result<Vec<u8>, AuditError> {
    let mut output = Vec::new();
    for entry in chain.entries() {
        serde_json::to_writer(&mut output, entry)?;
        output.push(b'\n');
    }
    Ok(output)
}

/// A single pretty-printed JSON array.
pub fn export_json(chain: &AuditChain) -> Result<Vec<u8>, AuditError> {
    Ok(serde_json::to_vec_pretty(chain.entries())?)
}

/// RFC 4180 CSV with a header row.
pub fn export_csv(chain: &AuditChain) -> Result<Vec<u8>, AuditError> {
    let mut output = Vec::new();

    // `vertical` is appended last, not interleaved with the existing columns, so a
    // reader who already has a fixed column-index parser only has to append one field
    // instead of reindexing every column after this one. This is still a breaking
    // change to an unversioned format — see CHANGELOG.md.
    output.extend_from_slice(
        b"id,timestamp,category,action,span_hash,span_length,confidence,source,pipeline_version,config_hash,chain_hash,vertical\n",
    );

    for entry in chain.entries() {
        let action = serde_json::to_string(&entry.action)?;
        let confidence = entry.confidence.map(|c| c.to_string()).unwrap_or_default();
        let span_length = entry.span_length.to_string();
        let source = entry.source.to_string();
        let vertical = entry.vertical.clone().unwrap_or_default();

        write_csv_row(
            &mut output,
            &[
                &entry.id,
                &entry.timestamp,
                &entry.category,
                &action,
                &entry.span_hash,
                &span_length,
                &confidence,
                &source,
                &entry.pipeline_version,
                &entry.config_hash,
                &entry.chain_hash,
                &vertical,
            ],
        );
    }

    Ok(output)
}

fn write_csv_row(output: &mut Vec<u8>, fields: &[&str]) {
    for (i, field) in fields.iter().enumerate() {
        if i > 0 {
            output.push(b',');
        }
        if field.contains([',', '"', '\n']) {
            output.push(b'"');
            for b in field.bytes() {
                if b == b'"' {
                    output.push(b'"');
                }
                output.push(b);
            }
            output.push(b'"');
        } else {
            output.extend_from_slice(field.as_bytes());
        }
    }
    output.push(b'\n');
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audit::entry::{AuditEntryInput, EntitySource, RedactionAction};

    fn chain_with(count: usize) -> AuditChain {
        let mut chain = AuditChain::new("cfg");
        for i in 0..count {
            chain.push(AuditEntryInput {
                id: format!("id-{i}"),
                category: "Email".into(),
                action: RedactionAction::Mask,
                span_hash: "abc".into(),
                span_length: 10,
                confidence: Some(0.9),
                source: EntitySource::Regex,
                pipeline_version: "1.0".into(),
                config_hash: String::new(),
                principal: None,
                vertical: None,
            });
        }
        chain
    }

    #[test]
    fn should_write_one_json_object_per_line() {
        let bytes = export_json_lines(&chain_with(3)).unwrap();
        let text = String::from_utf8(bytes).unwrap();
        let lines: Vec<_> = text.lines().collect();
        assert_eq!(lines.len(), 3);
        for line in lines {
            let value: serde_json::Value = serde_json::from_str(line).unwrap();
            assert!(value.get("chain_hash").is_some());
        }
    }

    #[test]
    fn should_write_a_json_array_of_every_entry() {
        let bytes = export_json(&chain_with(2)).unwrap();
        let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(value.as_array().unwrap().len(), 2);
    }

    #[test]
    fn should_write_a_csv_header_plus_one_row_per_entry() {
        let bytes = export_csv(&chain_with(2)).unwrap();
        let text = String::from_utf8(bytes).unwrap();
        let lines: Vec<_> = text.lines().collect();
        assert_eq!(lines.len(), 3);
        assert!(lines[0].starts_with("id,timestamp,category"));
    }

    #[test]
    fn should_quote_and_escape_csv_fields_containing_delimiters() {
        let mut output = Vec::new();
        write_csv_row(&mut output, &["plain", "has,comma", "has\"quote"]);
        assert_eq!(
            String::from_utf8(output).unwrap(),
            "plain,\"has,comma\",\"has\"\"quote\"\n"
        );
    }

    #[test]
    fn should_export_an_empty_chain_as_a_header_only_csv() {
        let bytes = export_csv(&chain_with(0)).unwrap();
        assert_eq!(String::from_utf8(bytes).unwrap().lines().count(), 1);
    }

    #[test]
    fn should_append_the_vertical_column_last_in_the_csv_header() {
        let bytes = export_csv(&chain_with(0)).unwrap();
        let text = String::from_utf8(bytes).unwrap();
        let header = text.lines().next().unwrap();
        assert!(
            header.ends_with(",vertical"),
            "vertical must be the last CSV column, got header {header:?}"
        );
    }

    #[test]
    fn should_round_trip_an_entry_with_a_vertical_through_csv() {
        let mut chain = AuditChain::new("cfg");
        chain.push(AuditEntryInput {
            id: "id-0".into(),
            category: "Email".into(),
            action: RedactionAction::Mask,
            span_hash: "abc".into(),
            span_length: 10,
            confidence: Some(0.9),
            source: EntitySource::Regex,
            pipeline_version: "1.0".into(),
            config_hash: String::new(),
            principal: None,
            vertical: Some("finance@3f9a1c02".into()),
        });

        let bytes = export_csv(&chain).unwrap();
        let text = String::from_utf8(bytes).unwrap();
        let row = text.lines().nth(1).unwrap();
        assert!(
            row.ends_with(",finance@3f9a1c02"),
            "expected the vertical value as the last CSV field, got row {row:?}"
        );
    }

    #[test]
    fn should_export_an_absent_vertical_as_an_empty_csv_field() {
        let bytes = export_csv(&chain_with(1)).unwrap();
        let text = String::from_utf8(bytes).unwrap();
        let row = text.lines().nth(1).unwrap();
        assert!(
            row.ends_with(','),
            "an absent vertical must export as an empty trailing field, got row {row:?}"
        );
    }
}
