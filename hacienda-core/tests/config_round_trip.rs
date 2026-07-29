//! Task 1 Step 1 of Phase 2: prove `HaciendaConfig` survives a TOML round trip
//! before the CLI's configuration file format is built on top of it.
//!
//! `extraction` is xberg's own `ExtractionConfig` — a large upstream struct this repo
//! does not control. A single field that does not round-trip would not be a detail; it
//! would mean the config file of §6.3 cannot express extraction settings at all.

use hacienda_core::HaciendaConfig;

fn round_trip(config: &HaciendaConfig) -> String {
    let text = toml::to_string_pretty(config).expect("serialize");
    let back: HaciendaConfig = toml::from_str(&text).expect("deserialize");
    let again = toml::to_string_pretty(&back).expect("reserialize");
    assert_eq!(text, again, "round trip is not stable");
    text
}

#[test]
fn should_round_trip_a_default_config_through_toml() {
    round_trip(&HaciendaConfig::default());
}

#[test]
fn should_round_trip_every_optional_stage_enabled() {
    let config = HaciendaConfig {
        pii: Some(Default::default()),
        compliance: Some(Default::default()),
        review: Some(Default::default()),
        glossary: Some(Default::default()),
        ..Default::default()
    };
    let text = round_trip(&config);
    for section in ["[pii]", "[compliance]", "[review]", "[glossary]"] {
        assert!(text.contains(section), "missing {section} in:\n{text}");
    }
}

#[test]
fn should_round_trip_the_xberg_concurrency_budget() {
    // D1: hacienda's `--concurrency` and xberg's thread budget are separate knobs, and
    // `config show` has to report both. That is only possible if this one survives a file.
    let mut config = HaciendaConfig::default();
    // Reached through `xberg::core::config`, not the crate root: `ConcurrencyConfig` is
    // absent from xberg's root re-export list even though its sibling config types are
    // all there. An upstream omission, not a deliberate seal — `pub mod core` makes it
    // reachable, so this needs no patch to xberg.
    config.extraction.concurrency = Some(xberg::core::config::ConcurrencyConfig {
        max_threads: Some(3),
    });
    let text = round_trip(&config);
    assert!(
        text.contains("max_threads = 3"),
        "xberg's thread budget did not reach the file:\n{text}"
    );

    let back: HaciendaConfig = toml::from_str(&text).expect("deserialize");
    assert_eq!(
        back.extraction
            .concurrency
            .as_ref()
            .and_then(|c| c.max_threads),
        Some(3)
    );
}

// ── Unknown keys (#34) ───────────────────────────────────────────────────────
//
// A typo'd key in a config file is precisely the "why is this not redacting?" incident
// §6.3 names: the operator believes a control is configured, the file says it is
// configured, and it is not. Rejecting is only defensible because the alternative —
// silently discarding — is indistinguishable from a working configuration.
//
// The forward-compatibility objection (a file written for a newer hacienda failing to
// load on an older one) does not apply asymmetrically here: xberg's `ExtractionConfig`
// already carries `deny_unknown_fields`, so an unknown key under `[extraction]` is a
// hard error today, before any change of ours. Leaving hacienda's own sections
// permissive would mean the *upstream* surface is stricter than the one we own.

#[test]
fn should_reject_a_misspelt_key_inside_a_stage_section() {
    let error = toml::from_str::<HaciendaConfig>("[pii]\nregex_frist = true\n")
        .expect_err("a misspelt key must not be silently discarded");
    assert!(
        error.to_string().contains("regex_frist"),
        "the error must name the offending key, or it cannot be acted on:\n{error}"
    );
}

#[test]
fn should_reject_a_misspelt_section_name_at_the_top_level() {
    let error = toml::from_str::<HaciendaConfig>("[reveiw]\nconfidence_threshold = 0.9\n")
        .expect_err("a misspelt section must not be silently discarded");
    assert!(error.to_string().contains("reveiw"), "{error}");
}

#[test]
fn should_reject_a_key_that_was_removed_rather_than_ignoring_it() {
    // `log_path` was accepted and ignored for the whole of Phase 1 (#23). Operators have
    // it in their files. Loading is where they find out it never did anything.
    let error = toml::from_str::<HaciendaConfig>("[pii.audit]\nlog_path = \"audit.log\"\n")
        .expect_err("a removed key must be reported, not skipped");
    assert!(error.to_string().contains("log_path"), "{error}");
}

#[test]
fn should_already_reject_an_unknown_key_under_extraction() {
    // Not a change of ours — xberg's `ExtractionConfig` declares `deny_unknown_fields`.
    // Asserted so that if upstream relaxes it, the asymmetry is caught here rather than
    // silently reintroducing the defect on the one section hacienda does not own.
    let error = toml::from_str::<HaciendaConfig>("[extraction]\nuse_cahce = true\n")
        .expect_err("upstream already denies unknown extraction keys");
    assert!(error.to_string().contains("use_cahce"), "{error}");
}
