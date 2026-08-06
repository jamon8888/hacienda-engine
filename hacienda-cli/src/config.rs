//! Configuration loading: discovery, format parsing, precedence, and CLI overrides.
//!
//! Precedence (integration design §6.3, mirroring
//! `xberg/crates/xberg-cli/src/commands/config.rs`):
//!
//! ```text
//! defaults < discovered hacienda.{toml,yaml,json} < --config < --config-json < flags
//! ```
//!
//! Every source up to and including `--config-json` is merged as JSON *before*
//! `HaciendaConfig` is ever deserialised — see [`merge_json`] for why the merge cannot
//! happen after parsing. `HaciendaConfig`'s own `#[serde(default, deny_unknown_fields)]`
//! then fills in anything still missing and rejects anything unrecognised.

use crate::cli::{ExtractArgs, Mode, ScanArgs};
use anyhow::{Context, Result};
use hacienda::HaciendaConfig;
use hacienda_core::glossary::GlossaryConfig;
use hacienda_core::pii::PipelineConfig;
use hacienda_core::redaction::RedactionMode;
use serde_json::Value;
use std::path::{Path, PathBuf};

/// Probed in this order so discovery is deterministic when more than one is present in
/// the same directory.
const CONFIG_BASENAMES: [&str; 4] = [
    "hacienda.toml",
    "hacienda.yaml",
    "hacienda.yml",
    "hacienda.json",
];

/// Load configuration with precedence: defaults < discovered file < `--config` <
/// `--config-json` < CLI flags.
pub fn load_config(
    config_path: Option<PathBuf>,
    config_json: Option<String>,
    extract_args: Option<&ExtractArgs>,
    scan_args: Option<&ScanArgs>,
) -> Result<HaciendaConfig> {
    let mut merged = Value::Object(serde_json::Map::new());

    // An explicit --config always wins over discovery; discovery only runs when the
    // operator did not name a file.
    let file_path = match config_path {
        Some(path) => Some(path),
        None => discover_config_file()?,
    };
    if let Some(path) = file_path {
        merge_json(&mut merged, value_from_file(&path)?);
    }

    // Overlay inline JSON. A partial document like `{"pii":{"model_threshold_default":
    // 0.8}}` must leave every other loaded field — including ones the file set —
    // untouched, so this merges rather than replaces.
    if let Some(json) = config_json {
        let inline: Value = serde_json::from_str(&json).context("parsing --config-json")?;
        merge_json(&mut merged, inline);
    }

    let mut config: HaciendaConfig =
        serde_json::from_value(merged).context("applying configuration precedence")?;

    apply_cli_overrides(&mut config, extract_args, scan_args);

    Ok(config)
}

/// Search the current directory and its ancestors for a
/// `hacienda.{toml,yaml,yml,json}`, falling back to the platform config directory
/// (`dirs::config_dir()/hacienda/`).
///
/// Mirrors `xberg::ExtractionConfig::discover()`, so the two tools agree on where a
/// project-local config lives.
fn discover_config_file() -> Result<Option<PathBuf>> {
    let mut current = std::env::current_dir().context("reading the current directory")?;

    loop {
        if let Some(found) = find_config_in_dir(&current) {
            return Ok(Some(found));
        }
        match current.parent() {
            Some(parent) => current = parent.to_path_buf(),
            None => break,
        }
    }

    if let Some(config_dir) = dirs::config_dir() {
        if let Some(found) = find_config_in_dir(&config_dir.join("hacienda")) {
            return Ok(Some(found));
        }
    }

    Ok(None)
}

/// The first `hacienda.{toml,yaml,yml,json}` present in `dir`, if any.
fn find_config_in_dir(dir: &Path) -> Option<PathBuf> {
    CONFIG_BASENAMES
        .iter()
        .map(|name| dir.join(name))
        .find(|candidate| candidate.is_file())
}

/// Parse a config file into a JSON value, auto-detecting format from its extension.
fn value_from_file(path: &Path) -> Result<Value> {
    let content = std::fs::read_to_string(path)
        .with_context(|| format!("reading config file {}", path.display()))?;
    let extension = path
        .extension()
        .and_then(|ext| ext.to_str())
        .map(str::to_lowercase);

    match extension.as_deref() {
        Some("toml") => toml::from_str::<Value>(&content)
            .with_context(|| format!("parsing config file {} as TOML", path.display())),
        Some("yaml" | "yml") => serde_yaml_ng::from_str::<Value>(&content)
            .with_context(|| format!("parsing config file {} as YAML", path.display())),
        Some("json") => serde_json::from_str::<Value>(&content)
            .with_context(|| format!("parsing config file {} as JSON", path.display())),
        _ => anyhow::bail!(
            "config file {} must have a .toml, .yaml, .yml, or .json extension",
            path.display()
        ),
    }
}

/// Deep-merge `overlay` onto `base`: nested objects merge key by key; scalars, arrays,
/// and type changes are replaced wholesale.
///
/// This operates purely on [`Value`], never on a parsed `HaciendaConfig`. Round-tripping
/// an already-loaded config back through `Serialize` would lose anything marked
/// `skip_serializing` — the bearer token in `StaticTokenConfig`, most notably — silently
/// turning a config with a real secret into one deserialised with an empty string in its
/// place. Merging the raw JSON of each source instead means a secret loaded from a file
/// survives being overlaid by an inline `--config-json` that never mentions `auth` at all.
fn merge_json(base: &mut Value, overlay: Value) {
    match overlay {
        Value::Object(overlay_map) => {
            if !base.is_object() {
                *base = Value::Object(serde_json::Map::new());
            }
            let base_map = base.as_object_mut().expect("just ensured Object above");
            for (key, value) in overlay_map {
                match base_map.get_mut(&key) {
                    Some(existing) => merge_json(existing, value),
                    None => {
                        base_map.insert(key, value);
                    }
                }
            }
        }
        other => *base = other,
    }
}

/// Apply `--mode`/`--threshold`/`--model-dir`/`--lora-dir` on top of the loaded config.
///
/// Materialises `config.pii` with [`PipelineConfig::default`] when a flag is given but
/// no `[pii]` section was loaded. The previous version only mutated `config.pii` when it
/// was already `Some`, so `--mode hash` against a config with no `[pii]` section silently
/// did nothing — the flag parsed, and was then discarded.
fn apply_cli_overrides(
    config: &mut HaciendaConfig,
    extract_args: Option<&ExtractArgs>,
    scan_args: Option<&ScanArgs>,
) {
    if let Some(args) = extract_args {
        if args.mode.is_some()
            || args.threshold.is_some()
            || args.model_dir.is_some()
            || args.lora_dir.is_some()
        {
            let pii = config.pii.get_or_insert_with(PipelineConfig::default);
            if let Some(mode) = args.mode {
                pii.redaction.mode = match mode {
                    Mode::Mask => RedactionMode::Mask,
                    Mode::Hash => RedactionMode::Hash,
                    Mode::Pseudonymize => RedactionMode::Pseudonymize,
                };
            }
            if let Some(threshold) = args.threshold {
                pii.model_threshold_default = threshold;
            }
            if let Some(dir) = args.model_dir.clone() {
                pii.model.model_dir = Some(dir);
                pii.model.enabled = true;
            }
            if let Some(dir) = args.lora_dir.clone() {
                pii.model.lora_adapter_dir = Some(dir);
            }
        }
    }

    if let Some(args) = scan_args {
        if let Some(threshold) = args.threshold {
            let pii = config.pii.get_or_insert_with(PipelineConfig::default);
            pii.model_threshold_default = threshold;
        }
    }

    // `--glossary-out` reads `HaciendaFacade::glossary_snapshot_with_auth`, which is only
    // ever populated when `config.glossary` is `Some` and `enabled` — a config with no
    // `[glossary]` section loaded would otherwise silently produce an empty
    // `glossary.json`, the same "flag parsed, then discarded" failure shape the `[pii]`
    // materialisation above already guards against. `GlossaryConfig::default()` already
    // has `enabled: true`, so inserting the default is sufficient.
    let wants_glossary = extract_args.is_some_and(|a| a.glossary_out.is_some())
        || scan_args.is_some_and(|a| a.glossary_out.is_some());
    if wants_glossary {
        config.glossary.get_or_insert_with(GlossaryConfig::default);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn should_leave_base_untouched_when_overlay_is_empty() {
        let mut base = json!({"pii": {"model_threshold_default": 0.5}});
        merge_json(&mut base, json!({}));
        assert_eq!(base, json!({"pii": {"model_threshold_default": 0.5}}));
    }

    /// The whole point of merging rather than replacing: an inline document that only
    /// names one nested key must not erase its siblings.
    #[test]
    fn should_merge_a_nested_key_without_erasing_its_siblings() {
        let mut base = json!({
            "pii": {"model_threshold_default": 0.5, "regex_first": true},
            "extraction": {"concurrency": {"max_threads": 4}},
        });
        merge_json(&mut base, json!({"pii": {"model_threshold_default": 0.9}}));
        assert_eq!(
            base,
            json!({
                "pii": {"model_threshold_default": 0.9, "regex_first": true},
                "extraction": {"concurrency": {"max_threads": 4}},
            })
        );
    }

    #[test]
    fn should_replace_an_array_wholesale_rather_than_concatenating() {
        let mut base = json!({"auth": {"static_tokens": [{"id": "a"}]}});
        merge_json(&mut base, json!({"auth": {"static_tokens": [{"id": "b"}]}}));
        assert_eq!(base, json!({"auth": {"static_tokens": [{"id": "b"}]}}));
    }

    #[test]
    fn should_replace_a_scalar_with_a_different_type_rather_than_erroring() {
        let mut base = json!({"pii": "not an object yet"});
        merge_json(&mut base, json!({"pii": {"regex_first": false}}));
        assert_eq!(base, json!({"pii": {"regex_first": false}}));
    }

    #[test]
    fn should_load_defaults_when_nothing_is_configured_and_nothing_is_discovered() {
        // No --config, no --config-json, and (barring a stray hacienda.toml on the
        // machine running this test) nothing discovered: must fall back to defaults
        // rather than erroring.
        let config = load_config(None, None, None, None);
        assert!(config.is_ok(), "{config:?}");
    }

    #[test]
    fn should_overlay_inline_json_onto_an_explicit_config_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("hacienda.toml");
        std::fs::write(
            &path,
            "[pii]\nregex_first = false\nmodel_threshold_default = 0.3\n",
        )
        .unwrap();

        let config = load_config(
            Some(path),
            Some(r#"{"pii":{"model_threshold_default":0.9}}"#.to_string()),
            None,
            None,
        )
        .unwrap();

        let pii = config.pii.expect("file set [pii]");
        assert_eq!(pii.model_threshold_default, 0.9, "inline JSON must win");
        assert!(
            !pii.regex_first,
            "a field the inline JSON never mentioned must survive from the file"
        );
    }

    #[test]
    fn should_materialise_pii_config_from_a_cli_flag_when_no_pii_section_was_loaded() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("hacienda.toml");
        std::fs::write(&path, "").unwrap();

        let extract_args = ExtractArgs {
            inputs: vec!["doc.txt".into()],
            mode: Some(Mode::Hash),
            threshold: None,
            model_dir: None,
            lora_dir: None,
            format: crate::cli::Format::Text,
            audit_out: None,
            vault: None,
            glossary_out: None,
            no_redact: false,
            i_accept_unredacted_pii: false,
            concurrency: crate::cli::ConcurrencyArgs { concurrency: None },
        };

        let config = load_config(Some(path), None, Some(&extract_args), None).unwrap();
        let pii = config
            .pii
            .expect("--mode must materialise a [pii] section even without a config file");
        assert_eq!(pii.redaction.mode, RedactionMode::Hash);
    }

    #[test]
    fn should_reject_a_config_file_with_an_unrecognised_extension() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("hacienda.ini");
        std::fs::write(&path, "").unwrap();

        let error = load_config(Some(path), None, None, None)
            .expect_err("an .ini config file must be refused, not silently skipped");
        assert!(error.to_string().contains(".toml"));
    }
}
