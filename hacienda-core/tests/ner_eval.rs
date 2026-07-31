//! Span-level NER evaluation harness (Task 1, spec §8 step 1).
//!
//! See `superpowers/specs/2026-07-31-vertical-model-specialisation-design.md` §8 and
//! `superpowers/plans/2026-07-31-vertical-model-specialisation-implementation.md` Task 1
//! for the full rationale. In short: every later claim in that plan ("zero-shot is good
//! enough", "vocabulary pruning costs no F1", "Tier 1 calibration closes a precision
//! gap") is unmeasurable without this harness, so this file has no upstream gate — it
//! *is* the gate.
//!
//! This file has two independent halves:
//!
//! 1. **Always compiled, always run** (no feature flags, no `#[ignore]`): fixture
//!    schema validation and the scoring/metrics math, including its unit tests. These
//!    need no model and must keep passing in default `cargo test` — they are this
//!    harness's own safety net, catching a bad hand-written byte offset in
//!    `fixtures/ner-eval/*.json` or a bug in the P/R/F1 math itself before either one
//!    is trusted to grade a model.
//! 2. **`#[ignore]`d, `ner-candle`-gated**: the runner that actually loads a local
//!    GLiNER2 model and scores the fixtures against it. Not run in CI (no model is
//!    available there); must still *compile* under
//!    `cargo check -p hacienda-core --features ner-candle --tests` so it does not bit-rot
//!    silently. See `fixtures/ner-eval/README.md` for the invocation.
//!
//! ## The recall ceiling this harness cannot see (spec §4.2, plan §1.4)
//!
//! `xberg::text::ner::candle::CandleBackend::detect` (verified against the pinned
//! `v1.0.2` tag) ignores the caller-supplied threshold and always passes its own
//! `const DEFAULT_THRESHOLD: f32 = 0.5` to `extract_ner`. `NerDetector::threshold` then
//! only *filters* what the backend already returned. So `NerDetector::with_threshold`
//! can only ever raise the effective threshold above 0.5 — it can never lower it — and
//! this harness **cannot observe any entity the backend scored below 0.5**, no matter
//! what threshold is requested. Every recall figure this harness reports is therefore
//! *truncated recall at 0.5*, not recall, and the report schema labels it that way
//! rather than leaving it to be discovered later. A precision/recall curve is
//! unobtainable from this harness for the same reason; Tier 1 calibration (spec §4.2)
//! is scoped to precision-side (raising) thresholds until `NerBackend::detect` threads
//! a threshold through to the backend upstream. This is *not* worked around by patching
//! the vendored dependency — see the threshold-sweep section of the runner, which
//! exists to measure how much recall is plausibly sitting below 0.5 rather than assume
//! an answer.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

// ============================================================================
// Fixture schema
// ============================================================================

#[derive(Debug, Clone, Deserialize)]
struct FixtureEntity {
    start: u32,
    end: u32,
    label: String,
}

#[derive(Debug, Clone, Deserialize)]
struct FixtureCase {
    id: String,
    lang: String,
    text: String,
    entities: Vec<FixtureEntity>,
}

#[derive(Debug, Clone, Deserialize)]
struct Fixture {
    cases: Vec<FixtureCase>,
}

const BASE_FR: &str = include_str!("../../fixtures/ner-eval/base-fr.json");
const BASE_EN: &str = include_str!("../../fixtures/ner-eval/base-en.json");
const VERTICAL_FINANCE: &str = include_str!("../../fixtures/ner-eval/vertical-finance.json");

/// Every fixture file this harness knows about, paired with a stable name used in
/// reports and assertion messages. Deliberately not lazily cached: this runs a handful
/// of times per `cargo test` invocation and `serde_json::from_str` on a few dozen KB of
/// JSON is not worth a `OnceLock` for.
fn all_fixtures() -> Vec<(&'static str, Fixture)> {
    vec![
        (
            "base-fr.json",
            serde_json::from_str(BASE_FR)
                .expect("fixtures/ner-eval/base-fr.json must be valid JSON"),
        ),
        (
            "base-en.json",
            serde_json::from_str(BASE_EN)
                .expect("fixtures/ner-eval/base-en.json must be valid JSON"),
        ),
        (
            "vertical-finance.json",
            serde_json::from_str(VERTICAL_FINANCE)
                .expect("fixtures/ner-eval/vertical-finance.json must be valid JSON"),
        ),
    ]
}

/// The label vocabulary each fixture is allowed to use, per
/// `fixtures/ner-eval/README.md`'s "Label vocabulary" section. Catches a typo'd label
/// in a hand-written fixture (e.g. `"loaction"`) that would otherwise silently score as
/// its own never-predicted label instead of failing loudly.
fn allowed_labels(fixture_name: &str) -> BTreeSet<&'static str> {
    if fixture_name.starts_with("vertical-") {
        ["iban", "swift_code", "account_number"]
            .into_iter()
            .collect()
    } else {
        ["person", "organization", "address", "email", "phone_number"]
            .into_iter()
            .collect()
    }
}

#[test]
fn should_have_at_least_forty_cases_per_base_language() {
    for (name, fixture) in all_fixtures() {
        if name == "base-fr.json" || name == "base-en.json" {
            assert!(
                fixture.cases.len() >= 40,
                "{name} has only {} cases, need at least 40",
                fixture.cases.len()
            );
        }
    }
}

#[test]
fn should_use_the_documented_label_vocabulary_per_fixture() {
    for (name, fixture) in all_fixtures() {
        let allowed = allowed_labels(name);
        for case in &fixture.cases {
            for entity in &case.entities {
                assert!(
                    allowed.contains(entity.label.as_str()),
                    "{name}/{}: label {:?} is not in the documented vocabulary {allowed:?} \
                     (see fixtures/ner-eval/README.md's \"Label vocabulary\" section)",
                    case.id,
                    entity.label
                );
            }
        }
    }
}

#[test]
fn should_have_unique_case_ids_within_each_fixture() {
    for (name, fixture) in all_fixtures() {
        let mut seen = BTreeSet::new();
        for case in &fixture.cases {
            assert!(
                seen.insert(case.id.clone()),
                "{name}: duplicate case id {:?}",
                case.id
            );
        }
    }
}

/// The safety net for step 2's hand-written byte offsets: every entity must slice
/// `text` at a valid UTF-8 boundary and yield non-empty content. This is deliberately
/// **not** `#[ignore]`d — the fixtures stay valid in default `cargo test` even though no
/// model runs there, so a bad offset introduced by a future edit fails the build
/// immediately rather than corrupting a report nobody looks at until step 4.
#[test]
fn should_slice_every_fixture_entity_at_valid_utf8_boundaries_with_nonempty_content() {
    for (name, fixture) in all_fixtures() {
        for case in &fixture.cases {
            assert!(
                case.lang == "fr" || case.lang == "en",
                "{name}/{}: unexpected lang {:?}",
                case.id,
                case.lang
            );
            for entity in &case.entities {
                let start = entity.start as usize;
                let end = entity.end as usize;
                assert!(
                    start < end,
                    "{name}/{}: entity start {start} must be strictly less than end {end}",
                    case.id
                );
                assert!(
                    end <= case.text.len(),
                    "{name}/{}: entity end {end} is past the end of text ({} bytes)",
                    case.id,
                    case.text.len()
                );
                assert!(
                    case.text.is_char_boundary(start),
                    "{name}/{}: start offset {start} is not a UTF-8 char boundary in {:?} \
                     (offsets must be BYTE offsets, not character offsets — see \
                     fixtures/ner-eval/ANNOTATION.md rule 6)",
                    case.id,
                    case.text
                );
                assert!(
                    case.text.is_char_boundary(end),
                    "{name}/{}: end offset {end} is not a UTF-8 char boundary in {:?} \
                     (offsets must be BYTE offsets, not character offsets — see \
                     fixtures/ner-eval/ANNOTATION.md rule 6)",
                    case.id,
                    case.text
                );
                let slice = &case.text[start..end];
                assert!(
                    !slice.is_empty(),
                    "{name}/{}: entity at [{start}, {end}) slices to an empty string",
                    case.id
                );
            }
        }
    }
}

// ============================================================================
// Metrics
// ============================================================================

/// A labelled span, in UTF-8 byte offsets — the shared currency between fixture gold
/// entities and a backend's predicted entities once both are reduced to (offsets,
/// label).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct Span {
    start: u32,
    end: u32,
    label: String,
}

/// How a predicted span is judged against a gold span of the same label.
#[derive(Debug, Clone, Copy)]
enum MatchMode {
    /// Predicted and gold spans must have identical `[start, end)` bounds.
    Exact,
    /// Predicted and gold spans count as a match when their IoU (intersection over
    /// union of the two byte ranges) is at least `min_ratio`.
    Overlap { min_ratio: f32 },
}

fn spans_match(a: &Span, b: &Span, mode: MatchMode) -> bool {
    match mode {
        MatchMode::Exact => a.start == b.start && a.end == b.end,
        MatchMode::Overlap { min_ratio } => {
            let inter_start = a.start.max(b.start);
            let inter_end = a.end.min(b.end);
            if inter_end <= inter_start {
                return false;
            }
            let intersection = f64::from(inter_end - inter_start);
            let union_start = a.start.min(b.start);
            let union_end = a.end.max(b.end);
            let union = f64::from(union_end - union_start);
            if union <= 0.0 {
                return false;
            }
            (intersection / union) as f32 >= min_ratio
        }
    }
}

/// Raw true/false-positive/negative counts for one label (or the micro-average).
#[derive(Debug, Clone, Copy, Default, Serialize)]
struct Counts {
    true_positives: usize,
    false_positives: usize,
    false_negatives: usize,
}

impl Counts {
    /// Vacuous-truth convention: with no predictions at all, precision is trivially
    /// 1.0 (there is no false positive among zero predictions); recall follows the
    /// mirror rule for zero gold spans. This is what makes the "zero gold, zero
    /// predictions ⇒ F1 = 1.0" case (below) fall out of the general formula rather than
    /// needing its own special case in the code.
    fn precision(&self) -> f32 {
        let denom = self.true_positives + self.false_positives;
        if denom == 0 {
            1.0
        } else {
            self.true_positives as f32 / denom as f32
        }
    }

    fn recall(&self) -> f32 {
        let denom = self.true_positives + self.false_negatives;
        if denom == 0 {
            1.0
        } else {
            self.true_positives as f32 / denom as f32
        }
    }

    fn f1(&self) -> f32 {
        let p = self.precision();
        let r = self.recall();
        if p + r == 0.0 {
            0.0
        } else {
            2.0 * p * r / (p + r)
        }
    }
}

#[derive(Debug, Clone, Serialize)]
struct LabelMetrics {
    label: String,
    gold_count: usize,
    predicted_count: usize,
    counts: Counts,
    precision: f32,
    recall: f32,
    f1: f32,
}

impl LabelMetrics {
    fn new(
        label: impl Into<String>,
        gold_count: usize,
        predicted_count: usize,
        counts: Counts,
    ) -> Self {
        Self {
            label: label.into(),
            gold_count,
            predicted_count,
            counts,
            precision: counts.precision(),
            recall: counts.recall(),
            f1: counts.f1(),
        }
    }
}

/// The sentinel key `Metrics::micro` is stored under conceptually; not present in
/// `per_label`, kept as its own field instead so a report reader cannot mistake it for
/// a real label.
const MICRO_LABEL: &str = "__micro__";

#[derive(Debug, Clone, Serialize)]
struct Metrics {
    per_label: BTreeMap<String, LabelMetrics>,
    micro: LabelMetrics,
}

/// Scores `pred` against `gold` under `mode`, per label, plus a micro-average across
/// all labels.
///
/// Matching is greedy one-to-one **per label**: spans with different labels are never
/// candidates for each other regardless of offset overlap (a label mismatch at
/// identical offsets is both a miss for the gold label and a false alarm for the
/// predicted label — see the unit test below). Within a label, each predicted span
/// (processed in ascending `(start, end)` order for determinism) may consume at most
/// one not-yet-matched gold span; a predicted span with no available match is a false
/// positive, so **duplicate predictions count as false positives** past the first.
fn score(gold: &[Span], pred: &[Span], mode: MatchMode) -> Metrics {
    let mut labels: BTreeSet<&str> = BTreeSet::new();
    labels.extend(gold.iter().map(|s| s.label.as_str()));
    labels.extend(pred.iter().map(|s| s.label.as_str()));

    let mut per_label = BTreeMap::new();
    let mut micro_counts = Counts::default();
    let mut micro_gold = 0usize;
    let mut micro_pred = 0usize;

    for label in labels {
        let gold_l: Vec<&Span> = gold.iter().filter(|s| s.label == label).collect();
        let mut pred_l: Vec<&Span> = pred.iter().filter(|s| s.label == label).collect();
        pred_l.sort_by_key(|s| (s.start, s.end));

        let mut matched = vec![false; gold_l.len()];
        let mut counts = Counts::default();
        for p in &pred_l {
            let candidate = gold_l
                .iter()
                .enumerate()
                .find(|(i, g)| !matched[*i] && spans_match(p, g, mode));
            match candidate {
                Some((i, _)) => {
                    matched[i] = true;
                    counts.true_positives += 1;
                }
                None => counts.false_positives += 1,
            }
        }
        counts.false_negatives = matched.iter().filter(|m| !**m).count();

        micro_counts.true_positives += counts.true_positives;
        micro_counts.false_positives += counts.false_positives;
        micro_counts.false_negatives += counts.false_negatives;
        micro_gold += gold_l.len();
        micro_pred += pred_l.len();

        per_label.insert(
            label.to_string(),
            LabelMetrics::new(label, gold_l.len(), pred_l.len(), counts),
        );
    }

    let micro = LabelMetrics::new(MICRO_LABEL, micro_gold, micro_pred, micro_counts);
    Metrics { per_label, micro }
}

#[cfg(test)]
mod metric_unit_tests {
    use super::*;

    fn span(start: u32, end: u32, label: &str) -> Span {
        Span {
            start,
            end,
            label: label.to_string(),
        }
    }

    #[test]
    fn should_score_a_perfect_match_as_f1_one() {
        let gold = vec![span(0, 5, "person")];
        let pred = vec![span(0, 5, "person")];
        let m = score(&gold, &pred, MatchMode::Exact);
        assert_eq!(m.micro.counts.true_positives, 1);
        assert_eq!(m.micro.counts.false_positives, 0);
        assert_eq!(m.micro.counts.false_negatives, 0);
        assert_eq!(m.micro.f1, 1.0);
        assert_eq!(m.per_label["person"].f1, 1.0);
    }

    #[test]
    fn should_score_all_miss_as_f1_zero() {
        let gold = vec![span(0, 5, "person"), span(10, 15, "person")];
        let pred = vec![span(20, 25, "person"), span(30, 35, "person")];
        let m = score(&gold, &pred, MatchMode::Exact);
        assert_eq!(m.micro.counts.true_positives, 0);
        assert_eq!(m.micro.counts.false_positives, 2);
        assert_eq!(m.micro.counts.false_negatives, 2);
        assert_eq!(m.micro.f1, 0.0);
    }

    #[test]
    fn should_diverge_between_exact_and_overlap_on_an_off_by_one_boundary() {
        // Gold [0,5), predicted [0,6): a single trailing byte overshoot.
        let gold = vec![span(0, 5, "person")];
        let pred = vec![span(0, 6, "person")];

        let exact = score(&gold, &pred, MatchMode::Exact);
        assert_eq!(exact.micro.counts.true_positives, 0);
        assert_eq!(
            exact.micro.f1, 0.0,
            "exact match must not tolerate an off-by-one boundary"
        );

        // IoU = 5/6 ≈ 0.833, comfortably above a 0.7 overlap threshold.
        let overlap = score(&gold, &pred, MatchMode::Overlap { min_ratio: 0.7 });
        assert_eq!(overlap.micro.counts.true_positives, 1);
        assert_eq!(
            overlap.micro.f1, 1.0,
            "overlap match should tolerate a high-IoU boundary miss"
        );
    }

    #[test]
    fn should_count_a_duplicate_prediction_as_a_false_positive() {
        let gold = vec![span(0, 5, "person")];
        let pred = vec![span(0, 5, "person"), span(0, 5, "person")];
        let m = score(&gold, &pred, MatchMode::Exact);
        assert_eq!(m.micro.counts.true_positives, 1);
        assert_eq!(
            m.micro.counts.false_positives, 1,
            "the second identical prediction must not also score as a TP"
        );
        assert_eq!(m.micro.counts.false_negatives, 0);
        assert!((m.micro.precision - 0.5).abs() < 1e-6);
        assert!((m.micro.recall - 1.0).abs() < 1e-6);
    }

    #[test]
    fn should_treat_a_label_mismatch_at_identical_offsets_as_a_miss_and_a_false_alarm() {
        let gold = vec![span(0, 5, "person")];
        let pred = vec![span(0, 5, "organization")];
        let m = score(&gold, &pred, MatchMode::Exact);
        assert_eq!(m.micro.counts.true_positives, 0);
        assert_eq!(m.micro.counts.false_positives, 1);
        assert_eq!(m.micro.counts.false_negatives, 1);
        assert_eq!(m.per_label["person"].counts.false_negatives, 1);
        assert_eq!(m.per_label["person"].counts.true_positives, 0);
        assert_eq!(m.per_label["organization"].counts.false_positives, 1);
    }

    #[test]
    fn should_define_f1_as_one_when_there_is_no_gold_and_no_prediction() {
        let m = score(&[], &[], MatchMode::Exact);
        assert_eq!(m.micro.precision, 1.0);
        assert_eq!(m.micro.recall, 1.0);
        assert_eq!(m.micro.f1, 1.0);
        assert!(m.per_label.is_empty());
    }

    #[test]
    fn should_report_gold_and_predicted_instance_counts_per_label() {
        let gold = vec![
            span(0, 5, "person"),
            span(10, 15, "person"),
            span(20, 30, "organization"),
        ];
        let pred = vec![span(0, 5, "person")];
        let m = score(&gold, &pred, MatchMode::Exact);
        assert_eq!(m.per_label["person"].gold_count, 2);
        assert_eq!(m.per_label["person"].predicted_count, 1);
        assert_eq!(m.per_label["organization"].gold_count, 1);
        assert_eq!(m.per_label["organization"].predicted_count, 0);
        assert_eq!(m.micro.gold_count, 3);
        assert_eq!(m.micro.predicted_count, 1);
    }
}

// ============================================================================
// Bootstrap confidence intervals
// ============================================================================

/// A minimal, dependency-free xorshift64 PRNG. The workspace has no `rand` dependency
/// (see root `Cargo.toml`'s `[workspace.dependencies]`, deliberately minimal), and a
/// bootstrap resample only needs "looks random enough, is reproducible given a seed" —
/// not cryptographic quality.
struct Xorshift64 {
    state: u64,
}

impl Xorshift64 {
    fn new(seed: u64) -> Self {
        // xorshift64 is undefined at state 0; force it odd/non-zero.
        Self { state: seed | 1 }
    }

    fn next_u64(&mut self) -> u64 {
        let mut x = self.state;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.state = x;
        x
    }

    /// A uniform index in `[0, bound)`. `bound` is always small (case counts in the
    /// tens), so the modulo bias from `u64::MAX` not being a multiple of `bound` is not
    /// worth a rejection-sampling loop here.
    fn next_index(&mut self, bound: usize) -> usize {
        (self.next_u64() % bound as u64) as usize
    }
}

#[derive(Debug, Clone, Serialize)]
struct BootstrapCi {
    point_estimate: f32,
    lower_95: f32,
    upper_95: f32,
    resamples: usize,
}

/// Bootstraps a 95% confidence interval on micro-averaged F1 by resampling *cases*
/// (not individual spans) with replacement — each resample redraws `cases.len()` cases,
/// pools their gold/predicted spans, and rescoring. Resampling cases rather than spans
/// keeps a multi-entity case's spans together, which matches how error is actually
/// correlated in this data (a single incorrectly tokenised name produces several wrong spans in
/// the same case, not independently wrong spans scattered across cases).
///
/// With ~40-45 cases per base-language fixture, a per-label F1 rests on the tens of
/// instances, not thousands; two point estimates 0.03 apart are noise, not signal. This
/// is what makes the interval necessary rather than decorative for spec step 4's gate
/// ("no F1 regression beyond agreed tolerance") — that gate is unenforceable against
/// bare point estimates.
fn bootstrap_micro_f1_ci(
    cases: &[(Vec<Span>, Vec<Span>)],
    mode: MatchMode,
    resamples: usize,
    seed: u64,
) -> BootstrapCi {
    let n = cases.len();
    let point_gold: Vec<Span> = cases.iter().flat_map(|(g, _)| g.iter().cloned()).collect();
    let point_pred: Vec<Span> = cases.iter().flat_map(|(_, p)| p.iter().cloned()).collect();
    let point_estimate = score(&point_gold, &point_pred, mode).micro.f1;

    if n == 0 || resamples == 0 {
        return BootstrapCi {
            point_estimate,
            lower_95: point_estimate,
            upper_95: point_estimate,
            resamples: 0,
        };
    }

    let mut rng = Xorshift64::new(seed);
    let mut samples = Vec::with_capacity(resamples);
    for _ in 0..resamples {
        let mut gold_acc = Vec::new();
        let mut pred_acc = Vec::new();
        for _ in 0..n {
            let idx = rng.next_index(n);
            gold_acc.extend(cases[idx].0.iter().cloned());
            pred_acc.extend(cases[idx].1.iter().cloned());
        }
        samples.push(score(&gold_acc, &pred_acc, mode).micro.f1);
    }
    samples.sort_by(|a, b| a.partial_cmp(b).expect("F1 is never NaN"));

    let lower_idx = ((resamples as f64) * 0.025).floor() as usize;
    let upper_idx = (((resamples as f64) * 0.975).ceil() as usize).min(resamples - 1);

    BootstrapCi {
        point_estimate,
        lower_95: samples[lower_idx],
        upper_95: samples[upper_idx.max(lower_idx)],
        resamples,
    }
}

#[cfg(test)]
mod bootstrap_unit_tests {
    use super::*;

    fn span(start: u32, end: u32, label: &str) -> Span {
        Span {
            start,
            end,
            label: label.to_string(),
        }
    }

    #[test]
    fn should_bound_the_point_estimate_within_its_bootstrap_interval() {
        let cases = vec![
            (vec![span(0, 5, "person")], vec![span(0, 5, "person")]),
            (vec![span(0, 5, "person")], vec![]),
            (
                vec![span(0, 5, "person"), span(10, 15, "person")],
                vec![span(0, 5, "person")],
            ),
        ];
        let ci = bootstrap_micro_f1_ci(&cases, MatchMode::Exact, 1000, 42);
        assert!(ci.lower_95 <= ci.point_estimate + 1e-6);
        assert!(ci.point_estimate <= ci.upper_95 + 1e-6);
        assert_eq!(ci.resamples, 1000);
    }

    #[test]
    fn should_collapse_the_interval_to_a_point_when_every_case_is_a_perfect_match() {
        let cases = vec![
            (vec![span(0, 5, "person")], vec![span(0, 5, "person")]),
            (
                vec![span(0, 5, "organization")],
                vec![span(0, 5, "organization")],
            ),
        ];
        let ci = bootstrap_micro_f1_ci(&cases, MatchMode::Exact, 1000, 7);
        assert_eq!(ci.point_estimate, 1.0);
        assert_eq!(ci.lower_95, 1.0);
        assert_eq!(ci.upper_95, 1.0);
    }

    #[test]
    fn should_return_the_point_estimate_as_the_whole_interval_for_zero_cases() {
        let ci = bootstrap_micro_f1_ci(&[], MatchMode::Exact, 1000, 1);
        assert_eq!(
            ci.point_estimate, 1.0,
            "zero gold and zero predictions ⇒ F1 defined as 1.0"
        );
        assert_eq!(ci.lower_95, 1.0);
        assert_eq!(ci.upper_95, 1.0);
        assert_eq!(ci.resamples, 0);
    }

    /// `resamples: 0` with non-empty cases must not panic: `samples` stays empty, so
    /// `resamples - 1` would underflow (usize) before an out-of-bounds index without
    /// the `resamples == 0` guard. It must instead fall back to a point-estimate-only
    /// interval, same shape as the zero-cases case above.
    #[test]
    fn should_return_the_point_estimate_as_the_whole_interval_for_zero_resamples() {
        let cases = vec![
            (vec![span(0, 5, "person")], vec![span(0, 5, "person")]),
            (vec![span(0, 5, "person")], vec![]),
        ];
        let ci = bootstrap_micro_f1_ci(&cases, MatchMode::Exact, 0, 42);
        assert_eq!(ci.lower_95, ci.point_estimate);
        assert_eq!(ci.upper_95, ci.point_estimate);
        assert_eq!(ci.resamples, 0);
    }
}

// ============================================================================
// Runner (ignored; requires a local model + the `ner-candle` feature)
// ============================================================================

#[cfg(all(feature = "ner-candle", not(target_arch = "wasm32")))]
mod runner {
    use super::*;
    use hacienda_core::pii::{NerDetector, PiiCategory};
    use std::path::{Path, PathBuf};
    use std::time::{SystemTime, UNIX_EPOCH};
    use xberg::types::entity::EntityCategory;

    /// The categories requested from the backend, and the label strings used to
    /// document the request in the report — kept as a pair so the report's `label_set`
    /// never drifts from what was actually sent to the model.
    fn categories_for_fixture(fixture_name: &str) -> (Vec<EntityCategory>, Vec<String>) {
        if fixture_name.starts_with("vertical-") {
            // The finance vertical's zero-shot label set (spec §4.1's example).
            let labels = ["iban", "swift_code", "account_number"];
            let categories = labels
                .iter()
                .map(|l| EntityCategory::Custom((*l).to_string()))
                .collect();
            (categories, labels.into_iter().map(str::to_string).collect())
        } else {
            let categories = vec![
                EntityCategory::Person,
                EntityCategory::Organization,
                EntityCategory::Location,
                EntityCategory::Email,
                EntityCategory::Phone,
            ];
            let labels = ["person", "organization", "location", "email", "phone"]
                .into_iter()
                .map(str::to_string)
                .collect();
            (categories, labels)
        }
    }

    /// Mirrors `to_pii_category`'s alias handling (`hacienda-core/src/pii/ner.rs`,
    /// `pub(crate)` and therefore not reachable from this integration test) closely
    /// enough to render a `PiiCategory` back to the same canonical `snake_case` string
    /// used as the `label` field in every fixture — see
    /// `fixtures/ner-eval/README.md`'s "Label vocabulary" section for why the fixtures
    /// are written in terms of the *post-mapping* category name rather than the raw
    /// label text sent to the model. Unit-variant categories serialize to exactly that
    /// string via `PiiCategory`'s own `#[serde(rename_all = "snake_case")]`
    /// `Serialize` impl; only the `Custom` arm needs handling here.
    fn category_label(category: &PiiCategory) -> String {
        match category {
            PiiCategory::Custom(s) => s.to_ascii_lowercase(),
            other => serde_json::to_value(other)
                .ok()
                .and_then(|v| v.as_str().map(str::to_string))
                .unwrap_or_else(|| format!("{other:?}").to_ascii_lowercase()),
        }
    }

    fn gold_spans(case: &FixtureCase) -> Vec<Span> {
        case.entities
            .iter()
            .map(|e| Span {
                start: e.start,
                end: e.end,
                label: e.label.clone(),
            })
            .collect()
    }

    async fn predicted_spans(detector: &NerDetector, text: &str) -> Vec<Span> {
        detector
            .detect(text)
            .await
            .expect("NER backend inference must not fail against a fixture case")
            .into_iter()
            .map(|e| Span {
                start: e.start,
                end: e.end,
                label: category_label(&e.category),
            })
            .collect()
    }

    fn blake3_digest(path: &Path) -> String {
        let bytes =
            std::fs::read(path).unwrap_or_else(|e| panic!("reading {}: {e}", path.display()));
        blake3::hash(&bytes).to_hex().to_string()
    }

    fn report_path() -> PathBuf {
        if let Ok(configured) = std::env::var("HACIENDA_EVAL_OUT") {
            return PathBuf::from(configured);
        }
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock is after the Unix epoch")
            .as_secs();
        PathBuf::from(format!("target/ner-eval/{timestamp}.json"))
    }

    #[derive(Debug, Clone, Serialize)]
    struct ThresholdSweepPoint {
        threshold: f32,
        metrics_exact: Metrics,
        metrics_overlap: Metrics,
    }

    #[derive(Debug, Clone, Serialize)]
    struct FixtureReport {
        fixture: String,
        label_set: Vec<String>,
        case_count: usize,
        /// Scored at the default threshold (0.5) — see the module's top-level recall
        /// note; this is the number that would appear in a normal report, before the
        /// sweep below characterises how much is sitting right at the ceiling.
        metrics_exact: Metrics,
        metrics_overlap: Metrics,
        bootstrap_micro_f1_exact: BootstrapCi,
        bootstrap_micro_f1_overlap: BootstrapCi,
        /// spec §4.2 / plan §1.4: sweeps 0.50..=0.95 in steps of 0.05. Because the
        /// backend hardcodes 0.5 internally, this can only ever demonstrate how much
        /// *precision* is bought by raising the threshold — it cannot demonstrate
        /// anything about recall below 0.5, which this harness cannot see at any
        /// threshold setting.
        threshold_sweep: Vec<ThresholdSweepPoint>,
    }

    #[derive(Debug, Clone, Serialize)]
    struct EvalReport {
        model_dir: String,
        model_safetensors_blake3: String,
        default_threshold: f32,
        recall_caveat: String,
        fixtures: Vec<FixtureReport>,
    }

    fn threshold_sweep_values() -> Vec<f32> {
        (0u8..=9).map(|i| 0.50 + f32::from(i) * 0.05).collect()
    }

    #[tokio::test]
    #[ignore = "requires HACIENDA_EVAL_MODEL_DIR pointing at a local GLiNER2 model \
                directory; see fixtures/ner-eval/README.md for the invocation"]
    async fn should_score_ner_eval_fixtures_against_a_local_model() {
        let model_dir = match std::env::var("HACIENDA_EVAL_MODEL_DIR") {
            Ok(dir) => PathBuf::from(dir),
            Err(_) => {
                eprintln!(
                    "HACIENDA_EVAL_MODEL_DIR is not set; skipping the NER eval runner. \
                     See fixtures/ner-eval/README.md for the invocation."
                );
                return;
            }
        };

        let default_threshold: f32 = 0.5;
        let model_safetensors_blake3 = blake3_digest(&model_dir.join("model.safetensors"));

        let mut fixtures_report = Vec::new();

        for (fixture_name, fixture) in all_fixtures() {
            let (categories, label_set) = categories_for_fixture(fixture_name);

            let detector = NerDetector::from_candle_local(&model_dir, None)
                .expect("loading the Candle GLiNER2 backend from HACIENDA_EVAL_MODEL_DIR")
                .with_categories(categories.clone())
                .with_threshold(default_threshold);

            let mut per_case = Vec::with_capacity(fixture.cases.len());
            for case in &fixture.cases {
                let gold = gold_spans(case);
                let pred = predicted_spans(&detector, &case.text).await;
                per_case.push((gold, pred));
            }

            let gold_all: Vec<Span> = per_case
                .iter()
                .flat_map(|(g, _)| g.iter().cloned())
                .collect();
            let pred_all: Vec<Span> = per_case
                .iter()
                .flat_map(|(_, p)| p.iter().cloned())
                .collect();

            let metrics_exact = score(&gold_all, &pred_all, MatchMode::Exact);
            let metrics_overlap =
                score(&gold_all, &pred_all, MatchMode::Overlap { min_ratio: 0.5 });
            let bootstrap_micro_f1_exact =
                bootstrap_micro_f1_ci(&per_case, MatchMode::Exact, 1000, 0xC0FFEE);
            let bootstrap_micro_f1_overlap = bootstrap_micro_f1_ci(
                &per_case,
                MatchMode::Overlap { min_ratio: 0.5 },
                1000,
                0xC0FFEE,
            );

            let mut threshold_sweep = Vec::new();
            for threshold in threshold_sweep_values() {
                let swept = NerDetector::from_candle_local(&model_dir, None)
                    .expect("loading the Candle GLiNER2 backend from HACIENDA_EVAL_MODEL_DIR")
                    .with_categories(categories.clone())
                    .with_threshold(threshold);

                let mut swept_gold = Vec::new();
                let mut swept_pred = Vec::new();
                for case in &fixture.cases {
                    swept_gold.extend(gold_spans(case));
                    swept_pred.extend(predicted_spans(&swept, &case.text).await);
                }
                threshold_sweep.push(ThresholdSweepPoint {
                    threshold,
                    metrics_exact: score(&swept_gold, &swept_pred, MatchMode::Exact),
                    metrics_overlap: score(
                        &swept_gold,
                        &swept_pred,
                        MatchMode::Overlap { min_ratio: 0.5 },
                    ),
                });
            }

            fixtures_report.push(FixtureReport {
                fixture: fixture_name.to_string(),
                label_set,
                case_count: fixture.cases.len(),
                metrics_exact,
                metrics_overlap,
                bootstrap_micro_f1_exact,
                bootstrap_micro_f1_overlap,
                threshold_sweep,
            });
        }

        let report = EvalReport {
            model_dir: model_dir.display().to_string(),
            model_safetensors_blake3,
            default_threshold,
            recall_caveat: "CandleBackend::detect (xberg v1.0.2) hardcodes threshold 0.5 \
                internally before NerDetector's own threshold filters the result. Every \
                recall figure in this report is therefore TRUNCATED RECALL AT 0.5, not \
                recall — entities the backend scored below 0.5 are invisible to this \
                harness at any configured threshold. See spec §4.2 and plan §1.4."
                .to_string(),
            fixtures: fixtures_report,
        };

        let out_path = report_path();
        if let Some(parent) = out_path.parent() {
            std::fs::create_dir_all(parent).expect("creating the report output directory");
        }
        let json = serde_json::to_string_pretty(&report).expect("serializing the eval report");
        std::fs::write(&out_path, json).expect("writing the eval report");
        eprintln!("NER eval report written to {}", out_path.display());
    }
}
