//! Command implementations for the hacienda CLI.

use crate::cli::{ConcurrencyArgs, ExtractArgs, Format, Mode, ScanArgs};
use anyhow::{Context, Result};
use hacienda::{HaciendaConfig, HaciendaFacade};
use hacienda_core::pii::PipelineConfig;
use hacienda_core::redaction::RedactionMode;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::task;
use tracing::warn;
use xberg::ExtractInput;

/// Load configuration with precedence: defaults < file < --config < --config-json < CLI flags
fn load_config(
    config_path: Option<PathBuf>,
    config_json: Option<String>,
    extract_args: Option<&ExtractArgs>,
    scan_args: Option<&ScanArgs>,
) -> Result<HaciendaConfig> {
    let mut config = HaciendaConfig::default();

    // Load from file if specified
    if let Some(path) = config_path {
        let content = std::fs::read_to_string(&path)
            .with_context(|| format!("reading config file {}", path.display()))?;
        config = toml::from_str(&content)
            .with_context(|| format!("parsing config file {}", path.display()))?;
    }

    // Overlay inline JSON if provided
    if let Some(json) = config_json {
        let json_config: HaciendaConfig =
            serde_json::from_str(&json).context("parsing --config-json")?;
        config = json_config;
    }

    // Apply CLI overrides from extract args
    if let Some(args) = extract_args {
        if let Some(pii) = config.pii.as_mut() {
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

    // Apply CLI overrides from scan args
    if let Some(args) = scan_args {
        if let Some(pii) = config.pii.as_mut() {
            if let Some(threshold) = args.threshold {
                pii.model_threshold_default = threshold;
            }
        }
    }

    Ok(config)
}

fn parse_concurrency(args: &ConcurrencyArgs) -> usize {
    args.concurrency.unwrap_or_else(num_cpus::get)
}

/// Run `hacienda config show`
pub async fn run_config_show(
    format: Format,
    config_path: Option<PathBuf>,
    config_json: Option<String>,
) -> Result<()> {
    let config = load_config(config_path, config_json, None, None)?;

    match format {
        Format::Json => {
            let json = serde_json::to_string_pretty(&config)?;
            println!("{json}");
        }
        Format::Text => {
            print_config_text(&config);
        }
    }
    Ok(())
}

fn print_config_text(config: &HaciendaConfig) {
    println!("hacienda configuration (effective values with provenance)");
    println!("============================================================");
    println!();

    // Extraction config
    println!("[extraction]");
    if let Some(ref concurrency) = config.extraction.concurrency {
        println!(
            "  max_threads = {:?}  (from: xberg default)",
            concurrency.max_threads
        );
    } else {
        println!("  max_threads = (not configured)");
    }
    println!();

    // PII config
    if let Some(pii) = &config.pii {
        println!("[pii]");
        let default_pii = PipelineConfig::default();
        println!(
            "  regex_first = {}  (from: {})",
            pii.regex_first,
            if pii.regex_first == default_pii.regex_first {
                "default"
            } else {
                "config"
            }
        );
        println!(
            "  model_threshold_default = {}  (from: {})",
            pii.model_threshold_default,
            if pii.model_threshold_default == default_pii.model_threshold_default {
                "default"
            } else {
                "config"
            }
        );
        println!(
            "  merge_overlap_threshold = {}  (from: {})",
            pii.merge_overlap_threshold,
            if pii.merge_overlap_threshold == default_pii.merge_overlap_threshold {
                "default"
            } else {
                "config"
            }
        );
        println!();

        println!("  [pii.redaction]");
        let default_redaction = hacienda_core::redaction::RedactionConfig::default();
        println!(
            "    mode = {:?}  (from: {})",
            pii.redaction.mode,
            if pii.redaction.mode == default_redaction.mode {
                "default"
            } else {
                "config"
            }
        );
        println!(
            "    key_id = {:?}  (from: {})",
            pii.redaction.key_id,
            pii.redaction
                .key_id
                .as_ref()
                .map(|_| "config")
                .unwrap_or("default")
        );
        println!();

        println!("  [pii.audit]");
        let default_audit = hacienda_core::audit::AuditConfig::default();
        println!(
            "    enabled = {}  (from: {})",
            pii.audit.enabled,
            if pii.audit.enabled == default_audit.enabled {
                "default"
            } else {
                "config"
            }
        );
        println!(
            "    config_hash = {}  (from: config)",
            pii.audit.config_hash
        );
        println!();

        println!("  [pii.model]");
        let default_model = hacienda_core::pii::ModelConfig::default();
        println!(
            "    enabled = {}  (from: {})",
            pii.model.enabled,
            if pii.model.enabled == default_model.enabled {
                "default"
            } else {
                "config"
            }
        );
        println!(
            "    model_dir = {:?}  (from: {})",
            pii.model.model_dir,
            pii.model
                .model_dir
                .as_ref()
                .map(|_| "config")
                .unwrap_or("default")
        );
        println!(
            "    lora_adapter_dir = {:?}  (from: {})",
            pii.model.lora_adapter_dir,
            pii.model
                .lora_adapter_dir
                .as_ref()
                .map(|_| "config")
                .unwrap_or("default")
        );
        println!();
    } else {
        println!("[pii]");
        println!("  (disabled - no pii section in config)");
        println!();
    }

    // Compliance
    if let Some(comp) = &config.compliance {
        println!("[compliance]");
        let default_comp = hacienda_core::compliance::ComplianceConfig::default();
        println!(
            "  model_name = {}  (from: {})",
            comp.model_name,
            if comp.model_name == default_comp.model_name {
                "default"
            } else {
                "config"
            }
        );
        println!(
            "  enabled_reports = {:?}  (from: {})",
            comp.enabled_reports,
            if comp.enabled_reports == default_comp.enabled_reports {
                "default"
            } else {
                "config"
            }
        );
        println!();
    }

    // Review
    if let Some(review) = &config.review {
        println!("[review]");
        let default_review = hacienda_core::review::ReviewConfig::default();
        println!(
            "  confidence_threshold = {}  (from: {})",
            review.confidence_threshold,
            if review.confidence_threshold == default_review.confidence_threshold {
                "default"
            } else {
                "config"
            }
        );
        println!(
            "  deadline_hours = {:?}  (from: {})",
            review.deadline_hours,
            if review.deadline_hours == default_review.deadline_hours {
                "default"
            } else {
                "config"
            }
        );
        println!();
    }

    // Glossary
    if let Some(glossary) = &config.glossary {
        println!("[glossary]");
        let default_glossary = hacienda_core::glossary::GlossaryConfig::default();
        println!(
            "  enabled = {}  (from: {})",
            glossary.enabled,
            if glossary.enabled == default_glossary.enabled {
                "default"
            } else {
                "config"
            }
        );
        println!(
            "  link_style = {:?}  (from: {})",
            glossary.link_style,
            if glossary.link_style == default_glossary.link_style {
                "default"
            } else {
                "config"
            }
        );
        println!(
            "  min_confidence = {}  (from: {})",
            glossary.min_confidence,
            if glossary.min_confidence == default_glossary.min_confidence {
                "default"
            } else {
                "config"
            }
        );
        println!(
            "  min_count = {}  (from: {})",
            glossary.min_count,
            if glossary.min_count == default_glossary.min_count {
                "default"
            } else {
                "config"
            }
        );
        println!();
    }
}

/// Run `hacienda scan` - detect PII without rewriting
pub async fn run_scan(
    args: ScanArgs,
    config_path: Option<PathBuf>,
    config_json: Option<String>,
) -> Result<()> {
    let config = load_config(config_path, config_json, None, Some(&args))?;

    let pii_config = config.pii.clone().unwrap_or_else(|| {
        let mut p = PipelineConfig::default();
        p.redaction.mode = RedactionMode::Mask; // doesn't matter for scan
        p
    });

    let facade = Arc::new(HaciendaFacade::new(HaciendaConfig {
        extraction: config.extraction,
        pii: Some(pii_config),
        compliance: config.compliance,
        review: config.review,
        glossary: config.glossary,
    })?);

    let _concurrency = parse_concurrency(&args.concurrency);
    let mut results = Vec::new();

    for input in &args.inputs {
        let extract_input = ExtractInput::from_uri(input);
        let result = facade.process(extract_input).await?;
        results.push(ScanResult {
            input: input.clone(),
            pii: result.pii.into_iter().next(),
        });
    }

    match args.format {
        Format::Json => {
            let json = serde_json::to_string_pretty(&results)?;
            println!("{json}");
        }
        Format::Text => {
            for r in results {
                println!("=== {} ===", r.input);
                if let Some(pii) = r.pii {
                    for entity in &pii.entities {
                        println!(
                            "  [{}] {}..{}  conf={:.2}  source={}",
                            entity.category,
                            entity.start,
                            entity.end,
                            entity.confidence,
                            entity.source
                        );
                    }
                    if pii.entities.is_empty() {
                        println!("  (no PII detected)");
                    }
                } else {
                    println!("  (no PII detection run)");
                }
                println!();
            }
        }
    }
    Ok(())
}

#[derive(serde::Serialize)]
struct ScanResult {
    input: String,
    pii: Option<hacienda_core::pii::PipelineResult>,
}

/// Run `hacienda extract` - redacted extraction by default
pub async fn run_extract(
    args: ExtractArgs,
    config_path: Option<PathBuf>,
    config_json: Option<String>,
) -> Result<()> {
    let config = load_config(config_path, config_json, Some(&args), None)?;

    let pii_config = config.pii.clone().unwrap_or_else(|| {
        let mut p = PipelineConfig::default();
        p.redaction.mode = RedactionMode::Mask;
        p
    });

    // Validate --no-redact requires explicit acknowledgement
    if args.no_redact && !args.i_accept_unredacted_pii {
        anyhow::bail!(
            "--no-redact requires --i-accept-unredacted-pii (explicit acknowledgement that raw PII will be output)"
        );
    }

    let facade = Arc::new(HaciendaFacade::new(HaciendaConfig {
        extraction: config.extraction,
        pii: Some(pii_config),
        compliance: config.compliance,
        review: config.review,
        glossary: config.glossary,
    })?);

    let concurrency = parse_concurrency(&args.concurrency);
    let mut all_results = Vec::new();

    // Process in batches with concurrency limit
    let inputs: Vec<_> = args.inputs.iter().map(ExtractInput::from_uri).collect();

    for chunk in inputs.chunks(concurrency) {
        let mut handles = Vec::new();
        for input in chunk {
            let facade = Arc::clone(&facade);
            let input = input.clone();
            handles.push(task::spawn(async move { facade.process(input).await }));
        }

        for handle in handles {
            let result = handle.await??;
            all_results.push(result);
        }
    }

    // Output results
    match args.format {
        Format::Json => {
            let json = serde_json::to_string_pretty(&all_results)?;
            println!("{json}");
        }
        Format::Text => {
            for (i, result) in all_results.iter().enumerate() {
                if i > 0 {
                    println!("---");
                }
                println!("Document {}", i + 1);
                // ExtractionResult has a `results` field with ExtractedDocument
                if let Some(doc) = result.extraction.results.first() {
                    println!("Content: {}", doc.content);
                }
                if !result.pii.is_empty() {
                    println!("PII detections:");
                    for pii in &result.pii {
                        for entity in &pii.entities {
                            println!(
                                "  [{}] {}..{}  conf={:.2}  source={}",
                                entity.category,
                                entity.start,
                                entity.end,
                                entity.confidence,
                                entity.source
                            );
                        }
                    }
                }
            }
        }
    }

    // Write audit output if requested
    if let Some(audit_out) = args.audit_out {
        // This would need facade access to the audit store
        // For now, we can't easily get the chain out without adding a method
        warn!("--audit-out not yet implemented (needs facade audit_store access)");
        let _ = audit_out; // suppress unused warning
    }

    Ok(())
}
