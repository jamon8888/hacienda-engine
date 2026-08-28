//! Accumulates detected mentions into a glossary and links them in a document.

use crate::glossary::config::{GlossaryConfig, LinkStyle};
use crate::pii::merge::MergedEntity;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// One glossary term and the evidence behind it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
/// GlossaryEntry struct
pub struct GlossaryEntry {
    pub category: String,
    pub term: String,
    pub count: usize,
    /// Mean detection confidence across every observed mention.
    pub mean_confidence: f32,
}

/// Collects mentions across one or more documents.
///
/// Entries are keyed by `(category, term)` and held in a [`BTreeMap`] so generated
/// glossaries are byte-identical across runs — an artefact that reorders itself is
/// useless as review evidence.
#[derive(Debug, Clone)]
/// EntityGlossary struct
pub struct EntityGlossary {
    entries: BTreeMap<(String, String), Accumulator>,
    config: GlossaryConfig,
}

#[derive(Debug, Clone, Default)]
struct Accumulator {
    count: usize,
    confidence_sum: f64,
}

impl EntityGlossary {
/// new function
    pub fn new(config: GlossaryConfig) -> Self {
        Self {
            entries: BTreeMap::new(),
            config,
        }
    }

    /// Record every entity detected in `text`.
    ///
    /// The mention is read back out of `text` by byte offset rather than taken from
    /// [`MergedEntity::text`], which is empty for regex detections — those carry
    /// offsets only.
    pub fn observe(&mut self, text: &str, entities: &[MergedEntity]) {
        for entity in entities {
            if entity.confidence < self.config.min_confidence {
                continue;
            }
            let Some(term) = slice_span(text, entity) else {
                continue;
            };
            let key = (entity.category.to_string(), term.to_string());
            let accumulator = self.entries.entry(key).or_default();
            accumulator.count += 1;
            accumulator.confidence_sum += f64::from(entity.confidence);
        }
    }

    /// Terms that meet [`GlossaryConfig::min_count`], most frequent first.
    pub fn entries(&self) -> Vec<GlossaryEntry> {
        let mut entries: Vec<_> = self
            .entries
            .iter()
            .filter(|(_, a)| a.count >= self.config.min_count)
            .map(|((category, term), a)| GlossaryEntry {
                category: category.clone(),
                term: term.clone(),
                count: a.count,
                mean_confidence: (a.confidence_sum / a.count as f64) as f32,
            })
            .collect();
        entries.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.term.cmp(&b.term)));
        entries
    }

/// is_empty function
    pub fn is_empty(&self) -> bool {
        self.entries().is_empty()
    }

    /// Rewrite every published term in `text` as a link to its glossary entry.
    ///
    /// Matches are resolved in one pass over non-overlapping ranges, so a term that
    /// contains another term never produces a nested link.
    pub fn link(&self, text: &str) -> String {
        let published = self.entries();
        if published.is_empty() {
            return text.to_string();
        }

        let mut matches: Vec<(usize, usize, &GlossaryEntry)> = Vec::new();
        for entry in &published {
            for (start, _) in text.match_indices(&entry.term) {
                matches.push((start, start + entry.term.len(), entry));
            }
        }
        // Longest match wins at any given offset, so "Alice Martin" is preferred over "Alice".
        matches.sort_by(|a, b| a.0.cmp(&b.0).then(b.1.cmp(&a.1)));

        let mut linked = String::with_capacity(text.len());
        let mut cursor = 0;
        for (start, end, entry) in matches {
            if start < cursor {
                continue;
            }
            linked.push_str(&text[cursor..start]);
            linked.push_str(&self.render_link(entry));
            cursor = end;
        }
        linked.push_str(&text[cursor..]);
        linked
    }

    /// Render the glossary itself as a markdown section.
    pub fn to_markdown(&self) -> String {
        let mut out = String::from("## Glossary\n");
        for entry in self.entries() {
            out.push_str(&format!(
                "\n### {}\n\n- Category: {}\n- Mentions: {}\n- Mean confidence: {:.2}\n",
                entry.term, entry.category, entry.count, entry.mean_confidence
            ));
        }
        out
    }

    fn render_link(&self, entry: &GlossaryEntry) -> String {
        let anchor = anchor(&entry.term);
        match self.config.link_style {
            LinkStyle::Markdown => format!("[{}](#glossary-{})", entry.term, anchor),
            LinkStyle::Html => format!("<a href=\"#glossary-{}\">{}</a>", anchor, entry.term),
            LinkStyle::Wiki => format!("[[{}]]", entry.term),
        }
    }
}

fn slice_span<'a>(text: &'a str, entity: &MergedEntity) -> Option<&'a str> {
    let (start, end) = (entity.start as usize, entity.end as usize);
    if start >= end || end > text.len() {
        return None;
    }
    text.get(start..end)
}

fn anchor(term: &str) -> String {
    term.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pii::types::{EntitySource, PiiCategory};

    fn entity(category: PiiCategory, start: u32, end: u32, confidence: f32) -> MergedEntity {
        MergedEntity {
            category,
            text: String::new(),
            start,
            end,
            confidence,
            source: EntitySource::Model,
            format_preserving: false,
            redact_template: String::new(),
        }
    }

    fn glossary(min_count: usize) -> EntityGlossary {
        EntityGlossary::new(GlossaryConfig {
            min_count,
            ..Default::default()
        })
    }

    #[test]
    fn should_read_the_mention_out_of_the_source_text() {
        let text = "Alice Martin";
        let mut glossary = glossary(1);
        glossary.observe(text, &[entity(PiiCategory::Person, 0, 12, 0.9)]);

        let entries = glossary.entries();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].term, "Alice Martin");
        assert_eq!(entries[0].category, "Person");
    }

    #[test]
    fn should_count_repeated_mentions_of_the_same_term() {
        let text = "Alice Martin and Alice Martin";
        let mut glossary = glossary(1);
        glossary.observe(
            text,
            &[
                entity(PiiCategory::Person, 0, 12, 0.8),
                entity(PiiCategory::Person, 17, 29, 1.0),
            ],
        );

        let entries = glossary.entries();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].count, 2);
        assert!((entries[0].mean_confidence - 0.9).abs() < 1e-6);
    }

    #[test]
    fn should_ignore_mentions_below_the_confidence_floor() {
        let mut glossary = glossary(1);
        glossary.observe("Alice Martin", &[entity(PiiCategory::Person, 0, 12, 0.1)]);
        assert!(glossary.is_empty());
    }

    #[test]
    fn should_not_publish_a_term_seen_fewer_times_than_min_count() {
        let mut glossary = glossary(2);
        glossary.observe("Alice Martin", &[entity(PiiCategory::Person, 0, 12, 0.9)]);
        assert!(glossary.is_empty());
    }

    #[test]
    fn should_skip_a_span_that_falls_outside_the_text() {
        let mut glossary = glossary(1);
        glossary.observe("short", &[entity(PiiCategory::Person, 0, 999, 0.9)]);
        assert!(glossary.is_empty());
    }

    #[test]
    fn should_skip_a_span_that_does_not_land_on_a_character_boundary() {
        let text = "café";
        let mut glossary = glossary(1);
        // Byte 4 is inside the two-byte 'é'.
        glossary.observe(text, &[entity(PiiCategory::Person, 0, 4, 0.9)]);
        assert!(glossary.is_empty());
    }

    #[test]
    fn should_link_every_published_occurrence() {
        let text = "Alice Martin met Alice Martin";
        let mut glossary = glossary(2);
        glossary.observe(
            text,
            &[
                entity(PiiCategory::Person, 0, 12, 0.9),
                entity(PiiCategory::Person, 17, 29, 0.9),
            ],
        );

        let linked = glossary.link(text);
        assert_eq!(
            linked,
            "[Alice Martin](#glossary-alice-martin) met [Alice Martin](#glossary-alice-martin)"
        );
    }

    #[test]
    fn should_never_nest_a_link_inside_another() {
        let text = "Alice Martin, Alice Martin, Alice";
        let mut glossary = glossary(2);
        glossary.observe(
            text,
            &[
                entity(PiiCategory::Person, 0, 12, 0.9),
                entity(PiiCategory::Person, 14, 26, 0.9),
                entity(PiiCategory::Person, 0, 5, 0.9),
                entity(PiiCategory::Person, 28, 33, 0.9),
            ],
        );

        let linked = glossary.link(text);
        // The longer term wins where the two overlap; the standalone "Alice" still links.
        assert!(linked.starts_with("[Alice Martin](#glossary-alice-martin),"));
        assert!(linked.ends_with("[Alice](#glossary-alice)"));
        assert!(!linked.contains("[[Alice"));
    }

    #[test]
    fn should_return_the_text_unchanged_when_nothing_is_published() {
        let glossary = glossary(2);
        assert_eq!(glossary.link("Alice Martin"), "Alice Martin");
    }

    #[test]
    fn should_render_the_configured_link_style() {
        let text = "Alice Alice";
        let mut glossary = EntityGlossary::new(GlossaryConfig {
            min_count: 2,
            link_style: LinkStyle::Wiki,
            ..Default::default()
        });
        glossary.observe(
            text,
            &[
                entity(PiiCategory::Person, 0, 5, 0.9),
                entity(PiiCategory::Person, 6, 11, 0.9),
            ],
        );
        assert_eq!(glossary.link(text), "[[Alice]] [[Alice]]");
    }

    #[test]
    fn should_render_a_markdown_section_for_published_terms() {
        let text = "Alice Alice";
        let mut glossary = glossary(2);
        glossary.observe(
            text,
            &[
                entity(PiiCategory::Person, 0, 5, 0.9),
                entity(PiiCategory::Person, 6, 11, 0.9),
            ],
        );

        let markdown = glossary.to_markdown();
        assert!(markdown.starts_with("## Glossary\n"));
        assert!(markdown.contains("### Alice"));
        assert!(markdown.contains("- Mentions: 2"));
    }

    #[test]
    fn should_order_entries_by_frequency_then_name() {
        let text = "Bob Bob Bob Ann Ann";
        let mut glossary = glossary(2);
        glossary.observe(
            text,
            &[
                entity(PiiCategory::Person, 0, 3, 0.9),
                entity(PiiCategory::Person, 4, 7, 0.9),
                entity(PiiCategory::Person, 8, 11, 0.9),
                entity(PiiCategory::Person, 12, 15, 0.9),
                entity(PiiCategory::Person, 16, 19, 0.9),
            ],
        );

        let terms: Vec<_> = glossary.entries().into_iter().map(|e| e.term).collect();
        assert_eq!(terms, vec!["Bob", "Ann"]);
    }
}
