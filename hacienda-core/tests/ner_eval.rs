//! Span-level evaluation harness for `hacienda_core::pii::NerDetector` (Task 1, spec §8
//! step 1, `superpowers/plans/2026-07-31-vertical-model-specialisation-implementation.md`).
//!
//! Three things live in this one file, deliberately:
//!
//! 1. A schema-validity test over `fixtures/ner-eval/*.json` that runs in a default
//!    `cargo test` (no model, no `ner-candle` feature) so the fixtures stay well-formed in
//!    CI even though nothing here can score them without a model.
//! 2. Span-matching/scoring code (`score`, `MatchMode`, `Metrics`) plus a bootstrap
//!    confidence-interval helper, both hand-verified against worked examples in the unit
//!    tests below.
//! 3. An `#[ignore]`d, `ner-candle`-gated runner that loads a real GLiNER2 model from
//!    `HACIENDA_EVAL_MODEL_DIR` and emits a JSON report, including the §1.4 threshold
//!    sweep and the §1.5 label-interference comparison.
//!
//! **The fixtures this harness scores are draft scaffolding, not a validated corpus.** See
//! `fixtures/ner-eval/README.md` and `fixtures/ner-eval/ANNOTATION.md` for what "draft"
//! means here and what a second annotator still needs to do before any number computed
//! against them is trustworthy.

use serde::Deserialize;
use std::collections::{BTreeMap, BTreeSet};

// ---------------------------------------------------------------------------------------
// Fixture schema
// ---------------------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct Fixture {
    cases: Vec<Case>,
}

#[derive(Debug, Deserialize)]
struct Case {
    id: String,
    #[allow(dead_code)] // not read by the harness yet; kept for a future per-language split
    lang: String,
    text: String,
    entities: Vec<GoldEntity>,
}

#[derive(Debug, Deserialize)]
struct GoldEntity {
    /// Byte offset of the first byte of the span (UTF-8 byte, not char, index).
    start: u32,
    /// Byte offset one past the last byte of the span.
    end: u32,
    // Only read by the `ner-candle`-gated runner (`Span::label` in `score_fixture`); a
    // default `cargo test` build never reaches it, hence the explicit `allow`.
    #[allow(dead_code)]
    label: String,
}

const BASE_FR: &str = include_str!("../../fixtures/ner-eval/base-fr.json");
const BASE_EN: &str = include_str!("../../fixtures/ner-eval/base-en.json");
const VERTICAL_FINANCE: &str = include_str!("../../fixtures/ner-eval/vertical-finance.json");

fn fixture(raw: &str) -> Fixture {
    serde_json::from_str(raw)
        .expect("fixtures/ner-eval/*.json must be valid JSON matching the documented schema")
}

fn all_fixtures() -> Vec<(&'static str, Fixture)> {
    vec![
        ("base-fr.json", fixture(BASE_FR)),
        ("base-en.json", fixture(BASE_EN)),
        ("vertical-finance.json", fixture(VERTICAL_FINANCE)),
    ]
}

// ---------------------------------------------------------------------------------------
// 1. Schema validity — runs in a default `cargo test`, no model required.
// ---------------------------------------------------------------------------------------

/// Every entity in every fixture must slice `text` on a UTF-8 boundary and yield
/// non-empty content. This is a structural check only: it says the offsets are
/// *well-formed*, not that the label a human assigned to that span is *correct* — that is
/// what `ANNOTATION.md`'s inter-annotator agreement step is for, and it has not run yet.
#[test]
fn should_have_schema_valid_entities_in_every_fixture() {
    for (name, data) in all_fixtures() {
        assert!(!data.cases.is_empty(), "{name} has no cases");
        for case in &data.cases {
            assert!(
                !case.text.is_empty(),
                "{name} case '{}' has empty text",
                case.id
            );
            assert!(
                !case.entities.is_empty(),
                "{name} case '{}' has no entities",
                case.id
            );
            for entity in &case.entities {
                assert!(
                    entity.start < entity.end,
                    "{name} case '{}': entity {:?} has start >= end",
                    case.id,
                    entity
                );
                let start = entity.start as usize;
                let end = entity.end as usize;
                // `.get()` never panics on a bad range; a byte-boundary or out-of-range
                // mistake surfaces as an assertion failure here instead of a slicing panic.
                let raw = case.text.as_bytes().get(start..end).unwrap_or_else(|| {
                    panic!(
                        "{name} case '{}': entity {:?} does not slice `text` within its bounds",
                        case.id, entity
                    )
                });
                let slice = std::str::from_utf8(raw).unwrap_or_else(|_| {
                    panic!(
                        "{name} case '{}': entity {:?} does not land on a UTF-8 char boundary",
                        case.id, entity
                    )
                });
                assert!(
                    !slice.is_empty(),
                    "{name} case '{}': entity {:?} slices to empty content",
                    case.id,
                    entity
                );
            }
        }
    }
}

// ---------------------------------------------------------------------------------------
// 2. Span matching and scoring
// ---------------------------------------------------------------------------------------

/// A labelled span, in either the gold or the predicted set. Offsets are UTF-8 byte
/// offsets, matching `GoldEntity` and `xberg::types::entity::Entity`.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Span {
    start: u32,
    end: u32,
    label: String,
}

/// How a predicted span is credited against a gold span of the same label.
#[derive(Debug, Clone, Copy)]
enum MatchMode {
    /// Predicted and gold spans must have identical `[start, end)`.
    Exact,
    /// Predicted and gold spans count as a match when their intersection-over-union
    /// meets `min_ratio`. Reported alongside `Exact` because the two diverge sharply on
    /// names (a one-token boundary miss is a total failure under `Exact` but often still
    /// useful under `Overlap`), and reporting only one hides that failure mode.
    Overlap { min_ratio: f32 },
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct Counts {
    tp: usize,
    fp: usize,
    fn_: usize,
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct Prf {
    precision: f32,
    recall: f32,
    f1: f32,
}

// `label`, `support`, and `predicted` are only read by the `ner-candle`-gated runner's
// `LabelMetricsReport::from` (report serialisation); a default `cargo test` build never
// constructs a report, hence the explicit `allow`.
#[derive(Debug, Clone)]
#[allow(dead_code)]
struct LabelMetrics {
    label: String,
    /// Number of gold instances of this label (`tp + fn_`).
    support: usize,
    /// Number of predicted instances of this label (`tp + fp`).
    predicted: usize,
    counts: Counts,
    prf: Prf,
}

#[derive(Debug, Clone)]
struct Metrics {
    per_label: BTreeMap<String, LabelMetrics>,
    micro: LabelMetrics,
}

/// Precision/recall/F1 from raw counts, with the zero-gold-zero-pred edge case defined as
/// F1 = 1.0 (there is nothing to find and nothing was claimed to be found — that is a
/// correct result, not an undefined one).
fn compute_prf(counts: Counts) -> Prf {
    let Counts { tp, fp, fn_ } = counts;
    if tp + fp + fn_ == 0 {
        return Prf {
            precision: 1.0,
            recall: 1.0,
            f1: 1.0,
        };
    }
    let precision = if tp + fp == 0 {
        0.0
    } else {
        tp as f32 / (tp + fp) as f32
    };
    let recall = if tp + fn_ == 0 {
        0.0
    } else {
        tp as f32 / (tp + fn_) as f32
    };
    let f1 = if precision + recall == 0.0 {
        0.0
    } else {
        2.0 * precision * recall / (precision + recall)
    };
    Prf {
        precision,
        recall,
        f1,
    }
}

fn spans_match(gold: &Span, pred: &Span, mode: MatchMode) -> bool {
    match mode {
        MatchMode::Exact => gold.start == pred.start && gold.end == pred.end,
        MatchMode::Overlap { min_ratio } => {
            let inter_start = gold.start.max(pred.start);
            let inter_end = gold.end.min(pred.end);
            if inter_end <= inter_start {
                return false;
            }
            let intersection = (inter_end - inter_start) as f32;
            let union_start = gold.start.min(pred.start);
            let union_end = gold.end.max(pred.end);
            let union = (union_end - union_start) as f32;
            union > 0.0 && intersection / union >= min_ratio
        }
    }
}

/// Score `pred` against `gold` under `mode`.
///
/// Matching is greedy one-to-one per label: each predicted span may satisfy at most one
/// gold span of the same label, so a duplicate prediction (or a second prediction that
/// lands on an already-matched gold span) counts as a false positive rather than a second
/// true positive. Predictions and gold spans of different labels never match each other,
/// even at identical offsets — that counts as both a false positive (for the predicted
/// label) and a false negative (for the gold label).
fn score(gold: &[Span], pred: &[Span], mode: MatchMode) -> Metrics {
    let labels: BTreeSet<&str> = gold
        .iter()
        .chain(pred.iter())
        .map(|s| s.label.as_str())
        .collect();

    let mut per_label = BTreeMap::new();
    let mut total = Counts::default();

    for label in labels {
        let gold_l: Vec<&Span> = gold.iter().filter(|s| s.label == label).collect();
        let pred_l: Vec<&Span> = pred.iter().filter(|s| s.label == label).collect();

        let mut matched_gold = vec![false; gold_l.len()];
        let mut counts = Counts::default();

        for p in &pred_l {
            let mut matched = false;
            for (i, g) in gold_l.iter().enumerate() {
                if matched_gold[i] {
                    continue;
                }
                if spans_match(g, p, mode) {
                    matched_gold[i] = true;
                    counts.tp += 1;
                    matched = true;
                    break;
                }
            }
            if !matched {
                counts.fp += 1;
            }
        }
        counts.fn_ = matched_gold.iter().filter(|matched| !**matched).count();

        total.tp += counts.tp;
        total.fp += counts.fp;
        total.fn_ += counts.fn_;

        per_label.insert(
            label.to_string(),
            LabelMetrics {
                label: label.to_string(),
                support: gold_l.len(),
                predicted: pred_l.len(),
                counts,
                prf: compute_prf(counts),
            },
        );
    }

    let micro = LabelMetrics {
        label: "__micro__".to_string(),
        support: gold.len(),
        predicted: pred.len(),
        counts: total,
        prf: compute_prf(total),
    };

    Metrics { per_label, micro }
}

#[cfg(test)]
mod metrics_tests {
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

        let metrics = score(&gold, &pred, MatchMode::Exact);

        let person = &metrics.per_label["person"];
        assert_eq!(
            person.counts,
            Counts {
                tp: 1,
                fp: 0,
                fn_: 0
            }
        );
        assert_eq!(
            person.prf,
            Prf {
                precision: 1.0,
                recall: 1.0,
                f1: 1.0
            }
        );
        assert_eq!(
            metrics.micro.counts,
            Counts {
                tp: 1,
                fp: 0,
                fn_: 0
            }
        );
        assert_eq!(metrics.micro.prf.f1, 1.0);
    }

    #[test]
    fn should_score_a_complete_miss_as_f1_zero() {
        let gold = vec![span(0, 5, "person")];
        let pred = vec![span(10, 15, "person")];

        let metrics = score(&gold, &pred, MatchMode::Exact);

        let person = &metrics.per_label["person"];
        assert_eq!(
            person.counts,
            Counts {
                tp: 0,
                fp: 1,
                fn_: 1
            }
        );
        assert_eq!(
            person.prf,
            Prf {
                precision: 0.0,
                recall: 0.0,
                f1: 0.0
            }
        );
    }

    /// A prediction one byte longer than gold: `Exact` rejects it outright, `Overlap`
    /// with a lenient ratio accepts it. The two modes are meant to diverge here.
    #[test]
    fn should_diverge_between_exact_and_overlap_on_an_off_by_one_boundary() {
        let gold = vec![span(0, 5, "person")]; // "Alice"
        let pred = vec![span(0, 6, "person")]; // "Alice " (one extra byte)

        let exact = score(&gold, &pred, MatchMode::Exact);
        assert_eq!(
            exact.per_label["person"].counts,
            Counts {
                tp: 0,
                fp: 1,
                fn_: 1
            }
        );
        assert_eq!(exact.per_label["person"].prf.f1, 0.0);

        // intersection = 5, union = 6 -> ratio = 0.8333, clears a 0.5 threshold.
        let overlap = score(&gold, &pred, MatchMode::Overlap { min_ratio: 0.5 });
        assert_eq!(
            overlap.per_label["person"].counts,
            Counts {
                tp: 1,
                fp: 0,
                fn_: 0
            }
        );
        assert_eq!(overlap.per_label["person"].prf.f1, 1.0);
    }

    #[test]
    fn should_count_a_duplicate_prediction_as_a_false_positive() {
        let gold = vec![span(0, 5, "person")];
        let pred = vec![span(0, 5, "person"), span(0, 5, "person")];

        let metrics = score(&gold, &pred, MatchMode::Exact);

        let person = &metrics.per_label["person"];
        assert_eq!(
            person.counts,
            Counts {
                tp: 1,
                fp: 1,
                fn_: 0
            }
        );
        // precision = 1/2, recall = 1/1, f1 = 2 * 0.5 * 1 / 1.5 = 0.6667
        assert_eq!(person.prf.precision, 0.5);
        assert_eq!(person.prf.recall, 1.0);
        assert!(
            (person.prf.f1 - (2.0 / 3.0)).abs() < 1e-6,
            "f1 = {}",
            person.prf.f1
        );
    }

    #[test]
    fn should_treat_a_label_mismatch_at_identical_offsets_as_both_fp_and_fn() {
        let gold = vec![span(0, 5, "person")];
        let pred = vec![span(0, 5, "organization")];

        let metrics = score(&gold, &pred, MatchMode::Exact);

        assert_eq!(
            metrics.per_label["person"].counts,
            Counts {
                tp: 0,
                fp: 0,
                fn_: 1
            }
        );
        assert_eq!(
            metrics.per_label["organization"].counts,
            Counts {
                tp: 0,
                fp: 1,
                fn_: 0
            }
        );
        assert_eq!(
            metrics.micro.counts,
            Counts {
                tp: 0,
                fp: 1,
                fn_: 1
            }
        );
        assert_eq!(metrics.micro.prf.f1, 0.0);
    }

    #[test]
    fn should_define_f1_as_one_when_there_is_no_gold_and_no_prediction() {
        let metrics = score(&[], &[], MatchMode::Exact);

        assert!(metrics.per_label.is_empty());
        assert_eq!(
            metrics.micro.counts,
            Counts {
                tp: 0,
                fp: 0,
                fn_: 0
            }
        );
        assert_eq!(
            metrics.micro.prf,
            Prf {
                precision: 1.0,
                recall: 1.0,
                f1: 1.0
            }
        );
    }
}

// ---------------------------------------------------------------------------------------
// Bootstrap confidence interval
// ---------------------------------------------------------------------------------------

/// A small, dependency-free splitmix64 PRNG. This crate has no `rand` dependency, and
/// resampling for a 95% interval does not need cryptographic quality randomness — it
/// needs a deterministic, evenly-distributed stream so the same seed reproduces the same
/// interval across runs.
struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self {
        Self(seed)
    }

    fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// A uniform index in `0..n`. `n` must be non-zero.
    fn gen_range(&mut self, n: usize) -> usize {
        (self.next_u64() % n as u64) as usize
    }
}

#[derive(Debug, Clone, Copy)]
struct BootstrapInterval {
    point: f32,
    lower: f32,
    upper: f32,
}

/// Resample `cases` (each a per-case `(gold, pred)` pair) with replacement `draws` times,
/// recomputing micro-F1 under `mode` each time, and report the point estimate alongside
/// the 95% interval over the resampled distribution.
///
/// A bare F1 point estimate over ~10-15 cases per label is not distinguishable from noise
/// at the granularity spec step 4's regression gate needs — this is what makes that gate
/// enforceable instead of a coin flip.
fn bootstrap_micro_f1_ci(
    cases: &[(Vec<Span>, Vec<Span>)],
    mode: MatchMode,
    draws: usize,
    seed: u64,
) -> BootstrapInterval {
    let flat_gold: Vec<Span> = cases.iter().flat_map(|(g, _)| g.iter().cloned()).collect();
    let flat_pred: Vec<Span> = cases.iter().flat_map(|(_, p)| p.iter().cloned()).collect();
    let point = score(&flat_gold, &flat_pred, mode).micro.prf.f1;

    if cases.is_empty() || draws == 0 {
        return BootstrapInterval {
            point,
            lower: point,
            upper: point,
        };
    }

    let mut rng = Rng::new(seed);
    let mut samples = Vec::with_capacity(draws);
    for _ in 0..draws {
        let mut gold = Vec::new();
        let mut pred = Vec::new();
        for _ in 0..cases.len() {
            let idx = rng.gen_range(cases.len());
            gold.extend(cases[idx].0.iter().cloned());
            pred.extend(cases[idx].1.iter().cloned());
        }
        samples.push(score(&gold, &pred, mode).micro.prf.f1);
    }
    samples.sort_by(|a, b| a.partial_cmp(b).expect("F1 is always finite"));

    let lower_idx = ((draws as f32) * 0.025).floor() as usize;
    let upper_idx = (((draws as f32) * 0.975).floor() as usize).min(draws - 1);

    BootstrapInterval {
        point,
        lower: samples[lower_idx],
        upper: samples[upper_idx],
    }
}

#[cfg(test)]
mod bootstrap_tests {
    use super::*;

    fn span(start: u32, end: u32, label: &str) -> Span {
        Span {
            start,
            end,
            label: label.to_string(),
        }
    }

    #[test]
    fn should_return_a_degenerate_interval_when_every_resample_scores_identically() {
        // Every case is a perfect match, so every possible resample (with replacement,
        // any combination of these three identical cases) also scores f1 = 1.0. The
        // interval must collapse to a single point.
        let cases: Vec<(Vec<Span>, Vec<Span>)> = (0..3)
            .map(|i| {
                let start = i * 10;
                (
                    vec![span(start, start + 5, "person")],
                    vec![span(start, start + 5, "person")],
                )
            })
            .collect();

        let ci = bootstrap_micro_f1_ci(&cases, MatchMode::Exact, 200, 42);

        assert_eq!(ci.point, 1.0);
        assert_eq!(ci.lower, 1.0);
        assert_eq!(ci.upper, 1.0);
    }

    #[test]
    fn should_return_the_point_estimate_unchanged_for_an_empty_case_set() {
        let ci = bootstrap_micro_f1_ci(&[], MatchMode::Exact, 1000, 42);
        assert_eq!(ci.point, 1.0); // zero-gold-zero-pred
        assert_eq!(ci.lower, 1.0);
        assert_eq!(ci.upper, 1.0);
    }

    #[test]
    fn should_widen_around_the_point_estimate_when_cases_disagree() {
        // Two cases: one a perfect match (f1=1), one a complete miss (f1=0). Some
        // resamples will be all-hit, some all-miss, some mixed — the interval must not
        // collapse to a point, and must bracket values on both sides of a mid score.
        let cases: Vec<(Vec<Span>, Vec<Span>)> = vec![
            (vec![span(0, 5, "person")], vec![span(0, 5, "person")]),
            (vec![span(0, 5, "person")], vec![span(50, 55, "person")]),
        ];

        let ci = bootstrap_micro_f1_ci(&cases, MatchMode::Exact, 1000, 42);

        assert!(
            ci.lower <= ci.point,
            "lower ({}) should not exceed point ({})",
            ci.lower,
            ci.point
        );
        assert!(
            ci.upper >= ci.point,
            "upper ({}) should not be below point ({})",
            ci.upper,
            ci.point
        );
        assert!(
            ci.lower < ci.upper,
            "interval should not collapse when cases disagree: {ci:?}"
        );
    }
}

// ---------------------------------------------------------------------------------------
// 3. Model-backed runner — #[ignore]d, native + `ner-candle` only.
// ---------------------------------------------------------------------------------------
//
// The recall ceiling this harness cannot see (spec plan §1.4): `CandleBackend::detect`
// (xberg v1.0.2) hardcodes `const DEFAULT_THRESHOLD: f32 = 0.5` and ignores the caller's
// threshold entirely. `NerDetector::threshold` then filters what the backend already
// returned, so hacienda can only ever raise the *effective* threshold above 0.5, never
// observe anything the backend itself scored below it. Every "recall" value this runner
// reports is **truncated recall at 0.5**, not recall — see `fixtures/ner-eval/README.md`.
#[cfg(all(feature = "ner-candle", not(target_arch = "wasm32")))]
mod runner {
    use super::*;
    use hacienda_core::pii::{NerDetector, PiiCategory};
    use serde::Serialize;
    use std::path::{Path, PathBuf};
    use xberg::types::entity::EntityCategory;

    /// The finance vertical's label set (mirrors `vertical-finance.json`). Hand-written
    /// here rather than derived from the fixture: the labels *requested* from the model
    /// are a config decision, independent of which fixture happens to exercise them.
    const FINANCE_LABELS: &[&str] = &[
        "iban",
        "swift_code",
        "account_number",
        "routing_number",
        "card_number",
    ];

    fn base_categories() -> Vec<EntityCategory> {
        vec![
            EntityCategory::Person,
            EntityCategory::Organization,
            EntityCategory::Location,
            EntityCategory::Email,
            EntityCategory::Phone,
        ]
    }

    fn base_category_labels() -> Vec<String> {
        ["person", "organization", "location", "email", "phone"]
            .into_iter()
            .map(str::to_string)
            .collect()
    }

    fn base_plus_finance_categories() -> Vec<EntityCategory> {
        let mut categories = base_categories();
        categories.extend(
            FINANCE_LABELS
                .iter()
                .map(|label| EntityCategory::Custom((*label).to_string())),
        );
        categories
    }

    /// Maps a detected `PiiCategory` back onto the label string a fixture would use.
    ///
    /// `NerDetector::detect` already runs entities through `to_pii_category`'s alias
    /// table (`ner.rs`), which is *not* the identity function: `EntityCategory::Custom("iban")`
    /// arrives here as `PiiCategory::Iban`, not `PiiCategory::Custom("iban")`, because the
    /// alias table claims that label. `"card_number"`, `"swift_code"`, and
    /// `"account_number"` are *not* in that table, so they stay `Custom(..)` and round-trip
    /// unchanged. This is spec plan §2.3's aliasing footgun, observable here rather than
    /// only in a unit test: a vertical author who names a label `iban` gets different
    /// downstream behaviour than one who names it `iban_number`.
    fn pii_category_label(category: &PiiCategory) -> String {
        match category {
            PiiCategory::Person => "person".to_string(),
            PiiCategory::Organization => "organization".to_string(),
            PiiCategory::Address => "location".to_string(),
            PiiCategory::Email => "email".to_string(),
            PiiCategory::PhoneNumber => "phone".to_string(),
            PiiCategory::Iban => "iban".to_string(),
            PiiCategory::CreditCard => "credit_card".to_string(),
            PiiCategory::Custom(label) => label.to_ascii_lowercase(),
            other => format!("{other:?}").to_ascii_lowercase(),
        }
    }

    /// Run `detector` over every case in `data`, returning `(case_id, gold, pred)` triples.
    async fn score_fixture(
        detector: &NerDetector,
        data: &Fixture,
    ) -> Vec<(String, Vec<Span>, Vec<Span>)> {
        let mut results = Vec::with_capacity(data.cases.len());
        for case in &data.cases {
            let gold: Vec<Span> = case
                .entities
                .iter()
                .map(|e| Span {
                    start: e.start,
                    end: e.end,
                    label: e.label.clone(),
                })
                .collect();
            let detected = detector
                .detect(&case.text)
                .await
                .unwrap_or_else(|err| panic!("NER backend failed on case '{}': {err}", case.id));
            let pred: Vec<Span> = detected
                .into_iter()
                .map(|e| Span {
                    start: e.start,
                    end: e.end,
                    label: pii_category_label(&e.category),
                })
                .collect();
            results.push((case.id.clone(), gold, pred));
        }
        results
    }

    fn model_safetensors_digest(model_dir: &Path) -> String {
        let bytes = std::fs::read(model_dir.join("model.safetensors")).unwrap_or_else(|err| {
            panic!(
                "reading {}: {err}",
                model_dir.join("model.safetensors").display()
            )
        });
        blake3::hash(&bytes).to_hex().to_string()
    }

    // ----- Report shapes -----

    #[derive(Serialize)]
    struct LabelMetricsReport {
        label: String,
        support_gold: usize,
        predicted: usize,
        tp: usize,
        fp: usize,
        fn_: usize,
        precision: f32,
        /// Truncated recall — see the module-level doc comment on the 0.5 ceiling.
        recall_truncated_at_0_5: f32,
        f1: f32,
    }

    impl From<&LabelMetrics> for LabelMetricsReport {
        fn from(m: &LabelMetrics) -> Self {
            Self {
                label: m.label.clone(),
                support_gold: m.support,
                predicted: m.predicted,
                tp: m.counts.tp,
                fp: m.counts.fp,
                fn_: m.counts.fn_,
                precision: m.prf.precision,
                recall_truncated_at_0_5: m.prf.recall,
                f1: m.prf.f1,
            }
        }
    }

    #[derive(Serialize)]
    struct MetricsReport {
        per_label: Vec<LabelMetricsReport>,
        micro: LabelMetricsReport,
        /// (lower, upper) of the 95% bootstrap interval on micro F1, 1000 resamples.
        micro_f1_bootstrap_ci_95: (f32, f32),
    }

    fn build_metrics_report(
        cases: &[(String, Vec<Span>, Vec<Span>)],
        mode: MatchMode,
    ) -> MetricsReport {
        let gold: Vec<Span> = cases
            .iter()
            .flat_map(|(_, g, _)| g.iter().cloned())
            .collect();
        let pred: Vec<Span> = cases
            .iter()
            .flat_map(|(_, _, p)| p.iter().cloned())
            .collect();
        let metrics = score(&gold, &pred, mode);

        let bootstrap_pairs: Vec<(Vec<Span>, Vec<Span>)> = cases
            .iter()
            .map(|(_, g, p)| (g.clone(), p.clone()))
            .collect();
        let ci = bootstrap_micro_f1_ci(&bootstrap_pairs, mode, 1000, 0xC0FF_EE42);

        MetricsReport {
            per_label: metrics
                .per_label
                .values()
                .map(LabelMetricsReport::from)
                .collect(),
            micro: LabelMetricsReport::from(&metrics.micro),
            micro_f1_bootstrap_ci_95: (ci.lower, ci.upper),
        }
    }

    #[derive(Serialize)]
    struct MatchModeMetrics {
        exact: MetricsReport,
        overlap_0_5: MetricsReport,
    }

    fn build_match_mode_metrics(cases: &[(String, Vec<Span>, Vec<Span>)]) -> MatchModeMetrics {
        MatchModeMetrics {
            exact: build_metrics_report(cases, MatchMode::Exact),
            overlap_0_5: build_metrics_report(cases, MatchMode::Overlap { min_ratio: 0.5 }),
        }
    }

    #[derive(Serialize)]
    struct FixtureReport {
        name: String,
        label_set_requested: Vec<String>,
        case_count: usize,
        #[serde(flatten)]
        metrics: MatchModeMetrics,
    }

    fn build_fixture_report(
        name: &str,
        label_set_requested: &[String],
        cases: &[(String, Vec<Span>, Vec<Span>)],
    ) -> FixtureReport {
        FixtureReport {
            name: name.to_string(),
            label_set_requested: label_set_requested.to_vec(),
            case_count: cases.len(),
            metrics: build_match_mode_metrics(cases),
        }
    }

    /// Task 1.5's hard gate: base-category quality scored with `DEFAULT_CATEGORIES`
    /// alone, and again with the finance vertical's labels added to the same request,
    /// against the same fixtures, same model, same threshold. If `base_plus_finance_vertical`
    /// is materially worse than `base_only`, "extend the base category set" (plan Task
    /// 2.2) is the wrong design and Task 2 must be re-scoped before it starts.
    #[derive(Serialize)]
    struct LabelInterferenceReport {
        base_only: MatchModeMetrics,
        base_plus_finance_vertical: MatchModeMetrics,
    }

    #[derive(Serialize)]
    struct ThresholdSweepPoint {
        threshold: f32,
        micro_precision: f32,
        micro_recall_truncated_at_0_5: f32,
        micro_f1: f32,
    }

    #[derive(Serialize)]
    struct EvalReport {
        generated_at_unix: u64,
        model_dir: String,
        model_safetensors_blake3: String,
        threshold_default: f32,
        fixtures: Vec<FixtureReport>,
        label_interference: LabelInterferenceReport,
        /// `MatchMode::Exact`, base categories only, `base-fr.json` + `base-en.json`
        /// combined, one point per 0.05 step from 0.50 to 0.95 inclusive.
        threshold_sweep: Vec<ThresholdSweepPoint>,
        notes: Vec<String>,
    }

    // `flavor = "multi_thread"` is required, not cosmetic: xberg v1.0.2's `CandleBackend::detect`
    // calls `tokio::task::block_in_place` (candle.rs:169), which panics with "can call blocking
    // only when running on the multi-threaded runtime" under the default current-thread test
    // runtime. Confirmed by running this test under plain `#[tokio::test]` before this fix.
    #[tokio::test(flavor = "multi_thread")]
    #[ignore = "requires HACIENDA_EVAL_MODEL_DIR and a real GLiNER2 model; see fixtures/ner-eval/README.md"]
    async fn should_produce_an_evaluation_report_against_a_local_model() {
        let Some(model_dir) = std::env::var_os("HACIENDA_EVAL_MODEL_DIR") else {
            eprintln!(
                "SKIPPED: HACIENDA_EVAL_MODEL_DIR is not set. Invocation:\n  \
                 HACIENDA_EVAL_MODEL_DIR=~/model_f16 cargo test -p hacienda-core \
                 --features ner-candle --test ner_eval -- --ignored --nocapture\n\
                 See fixtures/ner-eval/README.md."
            );
            return;
        };
        let model_dir = PathBuf::from(model_dir);
        let digest = model_safetensors_digest(&model_dir);
        let threshold_default = 0.5_f32;

        let base_fr = fixture(BASE_FR);
        let base_en = fixture(BASE_EN);
        let vertical_finance = fixture(VERTICAL_FINANCE);

        let base_detector = NerDetector::from_candle_local(&model_dir, None)
            .expect("model should load from HACIENDA_EVAL_MODEL_DIR")
            .with_threshold(threshold_default)
            .with_categories(base_categories());
        let extended_detector = NerDetector::from_candle_local(&model_dir, None)
            .expect("model should load from cache (already loaded above)")
            .with_threshold(threshold_default)
            .with_categories(base_plus_finance_categories());

        let fr_base_cases = score_fixture(&base_detector, &base_fr).await;
        let en_base_cases = score_fixture(&base_detector, &base_en).await;
        let fin_cases = score_fixture(&extended_detector, &vertical_finance).await;

        let fixtures = vec![
            build_fixture_report("base-fr.json", &base_category_labels(), &fr_base_cases),
            build_fixture_report("base-en.json", &base_category_labels(), &en_base_cases),
            build_fixture_report(
                "vertical-finance.json",
                &FINANCE_LABELS
                    .iter()
                    .map(|s| s.to_string())
                    .collect::<Vec<_>>(),
                &fin_cases,
            ),
        ];

        // §1.5 gate: same base fixtures, base categories vs. base + finance vertical.
        let mut base_only_cases = fr_base_cases.clone();
        base_only_cases.extend(en_base_cases.clone());

        let fr_extended_cases = score_fixture(&extended_detector, &base_fr).await;
        let en_extended_cases = score_fixture(&extended_detector, &base_en).await;
        let mut extended_cases = fr_extended_cases;
        extended_cases.extend(en_extended_cases);

        let label_interference = LabelInterferenceReport {
            base_only: build_match_mode_metrics(&base_only_cases),
            base_plus_finance_vertical: build_match_mode_metrics(&extended_cases),
        };

        // §1.4: threshold sweep, base categories, base-fr + base-en combined, Exact match.
        let mut threshold_sweep = Vec::new();
        for step in 0..=9u32 {
            let t = 0.50 + 0.05 * step as f32;
            let detector = NerDetector::from_candle_local(&model_dir, None)
                .expect("model should load from cache (already loaded above)")
                .with_threshold(t)
                .with_categories(base_categories());
            let mut cases = score_fixture(&detector, &base_fr).await;
            cases.extend(score_fixture(&detector, &base_en).await);
            let gold: Vec<Span> = cases
                .iter()
                .flat_map(|(_, g, _)| g.iter().cloned())
                .collect();
            let pred: Vec<Span> = cases
                .iter()
                .flat_map(|(_, _, p)| p.iter().cloned())
                .collect();
            let metrics = score(&gold, &pred, MatchMode::Exact);
            threshold_sweep.push(ThresholdSweepPoint {
                threshold: t,
                micro_precision: metrics.micro.prf.precision,
                micro_recall_truncated_at_0_5: metrics.micro.prf.recall,
                micro_f1: metrics.micro.prf.f1,
            });
        }

        let generated_at_unix = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system clock should not be before the Unix epoch")
            .as_secs();

        let report = EvalReport {
            generated_at_unix,
            model_dir: model_dir.display().to_string(),
            model_safetensors_blake3: digest,
            threshold_default,
            fixtures,
            label_interference,
            threshold_sweep,
            notes: vec![
                "recall_truncated_at_0_5 fields are NOT full recall: xberg v1.0.2's \
                 CandleBackend::detect hardcodes DEFAULT_THRESHOLD = 0.5 and ignores the \
                 caller's threshold, so no entity the backend scored below 0.5 can ever \
                 appear in this report. See fixtures/ner-eval/README.md."
                    .to_string(),
                "base-fr.json, base-en.json, and vertical-finance.json are DRAFT fixtures \
                 drafted by an engineering agent, not yet reviewed by a second annotator. \
                 Treat every F1 number in this report as a harness smoke test, not a \
                 validated baseline, until fixtures/ner-eval/ANNOTATION.md's \
                 inter-annotator agreement section is filled in."
                    .to_string(),
            ],
        };

        let out_path = std::env::var_os("HACIENDA_EVAL_OUT")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                PathBuf::from("target/ner-eval").join(format!("{generated_at_unix}.json"))
            });
        if let Some(parent) = out_path.parent() {
            std::fs::create_dir_all(parent)
                .unwrap_or_else(|err| panic!("creating {}: {err}", parent.display()));
        }
        let json = serde_json::to_string_pretty(&report).expect("report should serialise to JSON");
        std::fs::write(&out_path, json)
            .unwrap_or_else(|err| panic!("writing {}: {err}", out_path.display()));
        println!("wrote NER evaluation report to {}", out_path.display());
    }
}
