//! Context-word confidence boosting for regex matches.
//!
//! Simplified port of [presidio-rs](https://github.com/jqueguiner/presidio-rs)'s
//! `crates/presidio-analyzer/src/context.rs` (`LemmaContextAwareEnhancer`, MIT License,
//! Copyright (c) 2026 presidio-rust contributors). Their version tokenizes and lemmatizes
//! with a real NLP pipeline; this one splits on word boundaries and compares
//! case-insensitively, with no lemmatization. That's a deliberate simplification, not a
//! missing feature: every context word calibrated in
//! [`super::patterns::builtin_patterns`] (ported from
//! presidio's own `predefined.rs` word lists) is already a base word form — "card",
//! "iban", "ssn" — so stemming/lemmatizing would not change whether any of them match.
//!
//! [`enhance`] is what turns [`PatternMeta::base_confidence`](super::types::PatternMeta::base_confidence)
//! from a fixed number into something that actually reflects the surrounding text: a
//! medium-confidence match (an SSN-shaped number with no way to checksum-confirm it, or an
//! IBAN pattern with no validator attached) gets promoted when a supporting word like
//! "ssn" or "iban" appears nearby, and left alone otherwise.

use super::types::RegexEntity;

/// Added to a match's confidence when a supportive context word is found nearby.
const CONTEXT_SIMILARITY_FACTOR: f32 = 0.35;
/// Floor applied once context support is found, even if the match's own confidence plus
/// [`CONTEXT_SIMILARITY_FACTOR`] would land below it.
const MIN_SCORE_WITH_CONTEXT: f32 = 0.4;
/// Tokens before the match's start to inspect for a supportive word.
const PREFIX_WORDS: usize = 5;
/// Tokens after the match's end to inspect for a supportive word.
const SUFFIX_WORDS: usize = 0;

/// One word-boundary token: its lowercased text and its byte-offset span in the source.
struct Token<'a> {
    word: &'a str,
    start: u32,
    end: u32,
}

/// Split `text` on non-alphanumeric boundaries into word tokens (original case preserved;
/// [`has_supportive_word`] compares case-insensitively). Not a real
/// tokenizer (no locale-aware word segmentation, no handling of contractions or
/// hyphenation as single tokens) — see this module's doc for why that's acceptable here.
fn tokenize(text: &str) -> Vec<Token<'_>> {
    let mut tokens = Vec::new();
    let mut start: Option<usize> = None;
    for (i, ch) in text.char_indices() {
        if ch.is_alphanumeric() {
            start.get_or_insert(i);
        } else if let Some(s) = start.take() {
            tokens.push(Token {
                word: &text[s..i],
                start: s as u32,
                end: i as u32,
            });
        }
    }
    if let Some(s) = start {
        tokens.push(Token {
            word: &text[s..],
            start: s as u32,
            end: text.len() as u32,
        });
    }
    tokens
}

/// Boost each entity's confidence toward `1.0` when one of its category's context words
/// (`entity.context_words`, set by [`super::engine::RegexEngine::find_all`] from the
/// originating [`super::types::PatternMeta`]) appears within [`PREFIX_WORDS`] tokens
/// before or [`SUFFIX_WORDS`] tokens after its span in `text`.
///
/// A no-op for an entity that's already at `1.0` (nothing to boost) or that has no
/// context words calibrated for its category.
pub fn enhance(entities: &mut [RegexEntity], text: &str) {
    if entities
        .iter()
        .all(|e| e.confidence >= 1.0 || e.context_words.is_empty())
    {
        return;
    }
    let tokens = tokenize(text);
    for entity in entities.iter_mut() {
        if entity.confidence >= 1.0 || entity.context_words.is_empty() {
            continue;
        }
        if has_supportive_word(&tokens, entity.start, entity.end, entity.context_words) {
            let mut new_score = (entity.confidence + CONTEXT_SIMILARITY_FACTOR).min(1.0);
            if new_score < MIN_SCORE_WITH_CONTEXT {
                new_score = MIN_SCORE_WITH_CONTEXT;
            }
            entity.confidence = new_score;
        }
    }
}

fn has_supportive_word(tokens: &[Token<'_>], start: u32, end: u32, words: &[&str]) -> bool {
    let before_end = tokens.iter().filter(|t| t.end <= start).count();
    let after_idx = tokens
        .iter()
        .position(|t| t.start >= end)
        .unwrap_or(tokens.len());

    let prefix_lo = before_end.saturating_sub(PREFIX_WORDS);
    let suffix_hi = (after_idx + SUFFIX_WORDS).min(tokens.len());

    tokens[prefix_lo..before_end]
        .iter()
        .chain(tokens[after_idx..suffix_hi].iter())
        .any(|tok| words.iter().any(|w| w.eq_ignore_ascii_case(tok.word)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pii::types::PiiCategory;

    fn entity(
        category: PiiCategory,
        start: u32,
        end: u32,
        confidence: f32,
        words: &'static [&'static str],
    ) -> RegexEntity {
        RegexEntity {
            confidence,
            context_words: words,
            ..RegexEntity::new(category, start, end)
        }
    }

    #[test]
    fn should_boost_confidence_when_a_context_word_precedes_the_match() {
        let text = "my ssn is 123-45-6789 on file";
        let start = text.find("123-45-6789").unwrap() as u32;
        let end = start + "123-45-6789".len() as u32;
        let mut entities = vec![entity(
            PiiCategory::Ssn,
            start,
            end,
            0.4,
            &["ssn", "social", "security"],
        )];
        enhance(&mut entities, text);
        assert!(
            entities[0].confidence > 0.4,
            "expected a boost, got {}",
            entities[0].confidence
        );
    }

    #[test]
    fn should_leave_confidence_unchanged_with_no_nearby_context_word() {
        let text = "reference number 123-45-6789 for the order";
        let start = text.find("123-45-6789").unwrap() as u32;
        let end = start + "123-45-6789".len() as u32;
        let mut entities = vec![entity(
            PiiCategory::Ssn,
            start,
            end,
            0.4,
            &["ssn", "social", "security"],
        )];
        enhance(&mut entities, text);
        assert_eq!(entities[0].confidence, 0.4);
    }

    #[test]
    fn should_not_boost_a_match_already_at_full_confidence() {
        let text = "credit card 4111111111111111 charged";
        let start = text.find("4111111111111111").unwrap() as u32;
        let end = start + "4111111111111111".len() as u32;
        let mut entities = vec![entity(
            PiiCategory::CreditCard,
            start,
            end,
            1.0,
            &["credit", "card"],
        )];
        enhance(&mut entities, text);
        assert_eq!(entities[0].confidence, 1.0);
    }

    #[test]
    fn should_ignore_a_context_word_outside_the_prefix_window() {
        // "ssn" is 6 word-tokens before the match's first token ("123"); PREFIX_WORDS is 5,
        // so the window (tokens[2..7]: one, two, three, four, five) never reaches it.
        let text = "the ssn one two three four five 123-45-6789";
        let start = text.find("123-45-6789").unwrap() as u32;
        let end = start + "123-45-6789".len() as u32;
        let mut entities = vec![entity(PiiCategory::Ssn, start, end, 0.4, &["ssn"])];
        enhance(&mut entities, text);
        assert_eq!(entities[0].confidence, 0.4);
    }

    #[test]
    fn should_be_case_insensitive() {
        let text = "SSN: 123-45-6789";
        let start = text.find("123-45-6789").unwrap() as u32;
        let end = start + "123-45-6789".len() as u32;
        let mut entities = vec![entity(PiiCategory::Ssn, start, end, 0.4, &["ssn"])];
        enhance(&mut entities, text);
        assert!(entities[0].confidence > 0.4);
    }

    #[test]
    fn should_do_nothing_when_the_category_has_no_context_words() {
        let text = "reference 1234567890123 today";
        let start = text.find("1234567890123").unwrap() as u32;
        let end = start + "1234567890123".len() as u32;
        let mut entities = vec![entity(PiiCategory::CreditCard, start, end, 0.3, &[])];
        enhance(&mut entities, text);
        assert_eq!(entities[0].confidence, 0.3);
    }
}
