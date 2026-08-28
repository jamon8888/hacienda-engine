//! Merges deterministic (regex) and statistical (model) detections into one span list.

use super::types::{
    EntitySource, MergeConfig, MergePriority, ModelEntity, PiiCategory, RegexEntity,
};
use serde::{Deserialize, Serialize};

/// A detection after regex and model results have been reconciled.
#[derive(Debug, Clone, Serialize, Deserialize)]
/// MergedEntity struct
pub struct MergedEntity {
    pub category: PiiCategory,
    /// Mention text. Empty for regex detections, which carry offsets only.
    pub text: String,
    pub start: u32,
    pub end: u32,
    pub confidence: f32,
    pub source: EntitySource,
    pub format_preserving: bool,
    pub redact_template: String,
}

/// Reconcile regex and model detections into a single non-overlapping, offset-ordered list.
///
/// Two spans are treated as the same detection when their overlap ratio exceeds
/// `config.overlap_threshold`; the winner is chosen by `config.priority`.
pub fn merge_entities(
    regex_entities: Vec<RegexEntity>,
    model_entities: Vec<ModelEntity>,
    config: &MergeConfig,
) -> Vec<MergedEntity> {
    let mut all = Vec::with_capacity(regex_entities.len() + model_entities.len());

    for e in regex_entities {
        all.push(MergedEntity {
            category: e.category,
            text: String::new(),
            start: e.start,
            end: e.end,
            confidence: e.confidence,
            source: EntitySource::Regex,
            format_preserving: e.format_preserving,
            redact_template: e.redact_template,
        });
    }

    for e in model_entities {
        let redact_template = format!("[{:?}]", e.category).to_uppercase();
        all.push(MergedEntity {
            category: e.category,
            text: e.text,
            start: e.start,
            end: e.end,
            confidence: e.confidence,
            source: EntitySource::Model,
            format_preserving: false,
            redact_template,
        });
    }

    all.sort_by(|a, b| a.start.cmp(&b.start).then(a.end.cmp(&b.end)));

    let mut merged: Vec<MergedEntity> = Vec::with_capacity(all.len());
    for entity in all {
        match merged.last() {
            Some(last) if overlap_ratio(last, &entity) > config.overlap_threshold => {
                if should_replace(last, &entity, config) {
                    merged.pop();
                    merged.push(entity);
                }
            }
            // A partial overlap below the threshold is still a real overlap. Redaction
            // rewrites the text by slicing between spans, so keeping both would produce
            // a reversed range. Drop the later span.
            Some(last) if entity.start < last.end => {}
            _ => merged.push(entity),
        }
    }
    merged
}

fn overlap_ratio(a: &MergedEntity, b: &MergedEntity) -> f32 {
    let inter_start = a.start.max(b.start);
    let inter_end = a.end.min(b.end);
    if inter_start >= inter_end {
        return 0.0;
    }
    let inter = (inter_end - inter_start) as f32;
    let union = (a.end - a.start).max(b.end - b.start) as f32;
    inter / union
}

fn should_replace(existing: &MergedEntity, candidate: &MergedEntity, config: &MergeConfig) -> bool {
    match config.priority {
        MergePriority::RegexFirst => match (existing.source, candidate.source) {
            (EntitySource::Regex, EntitySource::Model) => false,
            (EntitySource::Model, EntitySource::Regex) => true,
            _ => prefer_stronger(existing, candidate, config),
        },
        MergePriority::HigherConfidence => {
            candidate.confidence - existing.confidence > config.confidence_epsilon
        }
        MergePriority::LongerSpan => {
            (candidate.end - candidate.start) > (existing.end - existing.start)
        }
    }
}

fn prefer_stronger(
    existing: &MergedEntity,
    candidate: &MergedEntity,
    config: &MergeConfig,
) -> bool {
    if candidate.confidence - existing.confidence > config.confidence_epsilon {
        return true;
    }
    (candidate.end - candidate.start) > (existing.end - existing.start)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn regex_entity(category: PiiCategory, start: u32, end: u32) -> RegexEntity {
        RegexEntity::new(category, start, end)
    }

    fn model_entity(category: PiiCategory, start: u32, end: u32, confidence: f32) -> ModelEntity {
        ModelEntity {
            category,
            text: "x".into(),
            start,
            end,
            confidence,
        }
    }

    #[test]
    fn should_keep_disjoint_spans_from_both_sources() {
        let merged = merge_entities(
            vec![regex_entity(PiiCategory::Email, 0, 10)],
            vec![model_entity(PiiCategory::Person, 20, 30, 0.9)],
            &MergeConfig::default(),
        );
        assert_eq!(merged.len(), 2);
        assert_eq!(merged[0].source, EntitySource::Regex);
        assert_eq!(merged[1].source, EntitySource::Model);
    }

    #[test]
    fn should_prefer_regex_over_model_when_spans_overlap() {
        let merged = merge_entities(
            vec![regex_entity(PiiCategory::Email, 0, 10)],
            vec![model_entity(PiiCategory::Person, 0, 10, 0.99)],
            &MergeConfig::default(),
        );
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].source, EntitySource::Regex);
        assert_eq!(merged[0].category, PiiCategory::Email);
    }

    #[test]
    fn should_drop_partial_overlaps_below_the_threshold() {
        let merged = merge_entities(
            vec![regex_entity(PiiCategory::Email, 0, 10)],
            vec![model_entity(PiiCategory::Person, 8, 40, 0.9)],
            &MergeConfig::default(),
        );
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].end, 10);
    }

    #[test]
    fn should_return_spans_in_offset_order() {
        let merged = merge_entities(
            vec![regex_entity(PiiCategory::Email, 30, 40)],
            vec![model_entity(PiiCategory::Person, 0, 10, 0.9)],
            &MergeConfig::default(),
        );
        assert_eq!(merged[0].start, 0);
        assert_eq!(merged[1].start, 30);
    }

    #[test]
    fn should_never_emit_overlapping_spans() {
        let merged = merge_entities(
            vec![
                regex_entity(PiiCategory::Email, 0, 10),
                regex_entity(PiiCategory::Iban, 5, 25),
            ],
            vec![
                model_entity(PiiCategory::Person, 3, 12, 0.9),
                model_entity(PiiCategory::Address, 24, 60, 0.7),
            ],
            &MergeConfig::default(),
        );
        for pair in merged.windows(2) {
            assert!(pair[0].end <= pair[1].start, "spans overlap: {pair:?}");
        }
    }

    #[test]
    fn should_take_the_more_confident_model_span_under_higher_confidence_priority() {
        let config = MergeConfig {
            priority: MergePriority::HigherConfidence,
            ..Default::default()
        };
        let merged = merge_entities(
            vec![],
            vec![
                model_entity(PiiCategory::Person, 0, 10, 0.4),
                model_entity(PiiCategory::FullName, 0, 10, 0.95),
            ],
            &config,
        );
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].category, PiiCategory::FullName);
    }
}
