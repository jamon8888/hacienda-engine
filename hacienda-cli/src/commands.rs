//! Command implementations for the hacienda CLI.

use crate::cli::{
    AuditVerifyArgs, ConcurrencyArgs, ExtractArgs, Format, RevealArgs, ScanArgs, ServeArgs,
};
use crate::config::load_config;
use anyhow::{Context, Result};
use hacienda::audit::{export_json, verify_entries, AuditChain, AuditEntry, GENESIS_HASH};
use hacienda::{HaciendaConfig, HaciendaError, HaciendaFacade, HaciendaResult};
use hacienda_api::{ApiLimits, ApiState};
use hacienda_core::auth::{AuthState, Caller};
use hacienda_core::jobs::InMemoryJobStore;
use hacienda_core::pii::PipelineConfig;
use hacienda_core::redaction::{EnvKeyResolver, RedactionMode};
use hacienda_rag::InMemoryVectorStore;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::task;
use xberg::ExtractInput;

fn parse_concurrency(args: &ConcurrencyArgs) -> usize {
    args.concurrency.unwrap_or_else(num_cpus::get)
}

/// Build a facade, wiring a pseudonym key resolver only when the effective redaction mode
/// needs one.
///
/// `Pseudonymiser::new` resolves the active key eagerly, so calling `with_key_resolver`
/// unconditionally would fail every run on a host with no `HACIENDA_PSEUDONYM_ACTIVE_KEY`
/// set — not just pseudonymize runs. Conditioning on the mode keeps every other mode exactly
/// as tolerant of a missing key as it already was. There is no CLI/config surface for
/// retired keys today, so `retired` is always empty.
fn build_facade(config: HaciendaConfig) -> Result<HaciendaFacade, hacienda::HaciendaError> {
    let needs_pseudonymiser = config
        .pii
        .as_ref()
        .is_some_and(|p| p.redaction.mode == RedactionMode::Pseudonymize);
    if needs_pseudonymiser {
        HaciendaFacade::with_key_resolver(config, &EnvKeyResolver::new(), &[])
    } else {
        HaciendaFacade::new(config)
    }
}

/// Close `facade`'s stores after `result`, regardless of whether `result` is `Ok`.
///
/// Phase 1 made `HaciendaFacade::close` recommended rather than required — skipping it
/// only costs the next process an extra recovery step. A CLI process that always exits
/// after one command is exactly the case where that recovery step is paid on every run
/// rather than occasionally, so every path out of `run_scan`/`run_extract` that has a
/// constructed facade closes it, error paths included.
///
/// A prior error from `result` takes priority: it is what the user asked about. A close
/// failure that follows a prior error is logged rather than returned, so a
/// close-time-only problem does not shadow the original failure.
async fn close_after<T>(facade: &HaciendaFacade, result: Result<T>) -> Result<T> {
    match facade.close().await {
        Ok(()) => result,
        Err(close_err) if result.is_ok() => Err(close_err).context("closing the audit store"),
        Err(close_err) => {
            tracing::warn!(
                error = %close_err,
                "failed to close the audit store while handling a prior error"
            );
            result
        }
    }
}

/// Run `hacienda pii reveal <token>`.
pub async fn run_pii_reveal(
    args: RevealArgs,
    config_path: Option<PathBuf>,
    config_json: Option<String>,
) -> Result<()> {
    let config = load_config(config_path, config_json, None, None)?;
    let facade = Arc::new(
        HaciendaFacade::with_key_resolver(config, &EnvKeyResolver::new(), &[])
            .context("no pseudonym key is configured for this process")?,
    );

    let result = run_pii_reveal_inner(&args, &facade).await;
    close_after(&facade, result).await
}

async fn run_pii_reveal_inner(args: &RevealArgs, facade: &HaciendaFacade) -> Result<()> {
    // Collapsed, mirroring `hacienda-api/src/error.rs`'s `PseudonymError` mapping: never
    // let the specific variant (malformed token vs. wrong key vs. unknown key id) reach
    // stderr — a caller probing a token must not learn which guess was closer.
    let plaintext = match facade
        .reveal_token_with_auth(Caller::Trusted, &args.token)
        .await
    {
        Ok(text) => text,
        Err(HaciendaError::Pseudonym(_) | HaciendaError::PiiDisabled) => {
            anyhow::bail!("invalid pseudonym token");
        }
        Err(other) => return Err(other.into()),
    };
    let tip = facade.audit_tip().await?;

    match args.format {
        Format::Json => {
            let json = serde_json::to_string_pretty(&serde_json::json!({
                "plaintext": plaintext,
                "audit_chain_tip": tip,
            }))?;
            println!("{json}");
        }
        Format::Text => {
            println!("{plaintext}");
            if let Some(tip) = tip {
                println!("audit_chain_tip: {tip}");
            }
        }
    }
    Ok(())
}

/// Run `hacienda audit verify <dir>`.
///
/// Pure file I/O — no facade, key resolver, or store involved, so there is nothing for
/// `close_after` to close. Verifies the flat `audit.json` export `--audit-out` writes; see
/// `AuditCommand::Verify`'s doc comment for why this is not the segmented `FileAuditStore`
/// format.
pub async fn run_audit_verify(args: AuditVerifyArgs) -> Result<()> {
    let path = args.dir.join("audit.json");
    let bytes = std::fs::read(&path).with_context(|| format!("reading {}", path.display()))?;
    let entries: Vec<AuditEntry> = serde_json::from_slice(&bytes)
        .with_context(|| format!("{} is not a valid audit entry array", path.display()))?;

    verify_entries(&entries).context("the audit chain does not verify")?;

    let tip = entries
        .last()
        .map(|e| e.chain_hash.clone())
        .unwrap_or_else(|| GENESIS_HASH.to_string());

    match args.format {
        Format::Json => {
            let json = serde_json::to_string_pretty(&serde_json::json!({
                "valid": true,
                "audit_chain_tip": tip,
            }))?;
            println!("{json}");
        }
        Format::Text => {
            println!("audit chain OK ({} entries), tip: {tip}", entries.len());
        }
    }
    Ok(())
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

        println!("  [pii.vertical]");
        match &pii.vertical {
            Some(vertical) => {
                println!("    id = {:?}  (from: config)", vertical.id);
                println!("    labels = {:?}  (from: config)", vertical.labels);
            }
            None => println!("    (not configured — no vertical is active)"),
        }
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

    let mut pii_config = config.pii.clone().unwrap_or_else(|| {
        let mut p = PipelineConfig::default();
        p.redaction.mode = RedactionMode::Mask; // doesn't matter for scan
        p
    });
    // `--concurrency` governs documents in flight through the PII stage
    // (`PiiPipeline::process`), not the CLI's file list — a single scanned input can
    // still decompose into several documents (e.g. a multi-page PDF).
    pii_config.concurrency = parse_concurrency(&args.concurrency);

    let facade = Arc::new(build_facade(HaciendaConfig {
        extraction: config.extraction,
        pii: Some(pii_config),
        compliance: config.compliance,
        review: config.review,
        glossary: config.glossary,
        auth: config.auth,
    })?);

    let result = run_scan_inputs(&args, &facade).await;
    close_after(&facade, result).await
}

async fn run_scan_inputs(args: &ScanArgs, facade: &HaciendaFacade) -> Result<()> {
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

    if let Some(glossary_out) = &args.glossary_out {
        write_glossary(facade, glossary_out).await?;
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

    // Validate --no-redact requires explicit acknowledgement (D5): the message must
    // name both what would be emitted and `scan` as the honest alternative — detection
    // without a rewrite, and no unredacted text on stdout.
    if args.no_redact && !args.i_accept_unredacted_pii {
        anyhow::bail!(
            "--no-redact requires --i-accept-unredacted-pii: without it, this command \
             refuses to run rather than emit unredacted document text (containing raw \
             PII) to stdout. If you only need to know what PII is present, use `hacienda \
             scan` instead — it detects and reports without emitting document text at all."
        );
    }

    let concurrency = parse_concurrency(&args.concurrency);

    // `--no-redact` skips the PII stage entirely (`pii: None`), not just the redaction
    // step within it — a mode/threshold flag surviving alongside `--no-redact` would
    // silently imply detection still ran. Below, `--audit-out` is refused in that case:
    // there is nothing to audit when the stage that writes audit entries never runs.
    let pii_config = if args.no_redact {
        None
    } else {
        let mut p = config.pii.clone().unwrap_or_else(|| {
            let mut p = PipelineConfig::default();
            p.redaction.mode = RedactionMode::Mask;
            p
        });
        // `--concurrency` bounds documents in flight through the PII stage
        // (`PiiPipeline::process`) within one `facade.process` call — separate from,
        // and in addition to, the per-file `task::spawn` fan-out below.
        p.concurrency = concurrency;
        Some(p)
    };

    if let Some(dir) = &args.audit_out {
        let audit_enabled = pii_config.as_ref().is_some_and(|p| p.audit.enabled);
        if !audit_enabled {
            anyhow::bail!(
                "--audit-out {} requires PII auditing to be enabled, but this run has none: {}",
                dir.display(),
                if args.no_redact {
                    "--no-redact skips the PII stage entirely"
                } else {
                    "[pii.audit] enabled = false in the effective configuration"
                }
            );
        }
    }

    let facade = Arc::new(build_facade(HaciendaConfig {
        extraction: config.extraction,
        pii: pii_config,
        compliance: config.compliance,
        review: config.review,
        glossary: config.glossary,
        auth: config.auth,
    })?);

    let result = run_extract_inputs(&args, concurrency, &facade).await;
    close_after(&facade, result).await
}

async fn run_extract_inputs(
    args: &ExtractArgs,
    concurrency: usize,
    facade: &Arc<HaciendaFacade>,
) -> Result<()> {
    let mut all_results = Vec::new();

    // Process in batches with concurrency limit
    let inputs: Vec<_> = args.inputs.iter().map(ExtractInput::from_uri).collect();

    for chunk in inputs.chunks(concurrency) {
        let mut handles = Vec::new();
        for input in chunk {
            let facade = Arc::clone(facade);
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

    if let Some(audit_out) = &args.audit_out {
        write_audit_chain(facade, audit_out).await?;
    }

    if let Some(vault_dir) = &args.vault {
        write_vault(args, facade, &args.inputs, &all_results, vault_dir).await?;
        eprintln!("hacienda: wrote vault to {}", vault_dir.display());
    }

    if let Some(glossary_out) = &args.glossary_out {
        write_glossary(facade, glossary_out).await?;
    }

    Ok(())
}

/// Track J1: `hacienda extract --vault <dir>` — the Track I2 layout, deliberately thinner
/// than Studio's per Track J2's explicit decision (recorded in the program plan): the CLI
/// has no general-purpose NER entity pipeline (Track K, `ner-candle`, is not in any default
/// build), so there is no `entities/`, `GLOSSARY.md`, or `kg-export/` — inventing an entity
/// graph the CLI cannot actually produce would be worse than a README that says so. What the
/// CLI *does* have that Studio cannot — real audit chains, reversible pseudonyms — carries
/// through: `pii-registry.json` is the PII-only counterpart to Studio's entity registry, and
/// the audit chain is mirrored into the vault when `--audit-out` is also given.
async fn write_vault(
    args: &ExtractArgs,
    facade: &HaciendaFacade,
    inputs: &[String],
    results: &[HaciendaResult],
    vault_dir: &Path,
) -> Result<()> {
    let documents_dir = vault_dir.join("documents");
    std::fs::create_dir_all(&documents_dir)
        .with_context(|| format!("creating {}", documents_dir.display()))?;

    let mut manifest_files = Vec::new();
    let mut pii_registry = Vec::new();

    for (input, result) in inputs.iter().zip(results.iter()) {
        let doc = result.extraction.results.first();
        let content = doc.map(|d| d.content.as_str()).unwrap_or_default();
        let mime = doc.map(|d| d.mime_type.as_ref()).unwrap_or("unknown");

        let stem = Path::new(input)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("document");
        let doc_name = format!("{stem}.md");

        let pii = result.pii.first();
        let pii_count = pii.map(|p| p.entities.len()).unwrap_or(0);

        let markdown = format!(
            "---\nsource: {input}\ntype: {mime}\nprocessed: {processed}\npii_entities_found: {pii_count}\n---\n\n{content}\n",
            processed = chrono::Utc::now().to_rfc3339(),
        );
        std::fs::write(documents_dir.join(&doc_name), markdown)
            .with_context(|| format!("writing documents/{doc_name}"))?;

        manifest_files.push(serde_json::json!({
            "name": doc_name,
            "source": input,
            "pii_entities_found": pii_count,
        }));
        if let Some(pii) = pii {
            if !pii.entities.is_empty() {
                pii_registry.push(serde_json::json!({
                    "document": format!("documents/{doc_name}"),
                    "entities": pii.entities,
                }));
            }
        }
    }

    std::fs::write(
        vault_dir.join("_manifest.json"),
        serde_json::to_string_pretty(&serde_json::json!({
            "files": manifest_files,
            "generated": chrono::Utc::now().to_rfc3339(),
        }))?,
    )
    .context("writing _manifest.json")?;

    std::fs::write(
        vault_dir.join("pii-registry.json"),
        serde_json::to_string_pretty(&pii_registry)?,
    )
    .context("writing pii-registry.json")?;

    // Track J3: mirror the audit chain into the vault when the caller asked for one — a
    // vault that silently carries or omits an audit chain is exactly the guessing game the
    // plan calls out. `--audit-out`'s own standalone write already happened in the caller.
    let audit_included = if args.audit_out.is_some() {
        write_audit_chain(facade, &vault_dir.join("audit")).await?;
        true
    } else {
        false
    };

    std::fs::write(
        vault_dir.join("README.md"),
        build_vault_readme(manifest_files.len(), audit_included),
    )
    .context("writing README.md")?;

    Ok(())
}

fn build_vault_readme(file_count: usize, audit_included: bool) -> String {
    let documents = if file_count == 1 {
        "document"
    } else {
        "documents"
    };
    let audit_note = if audit_included {
        "This vault includes an audit chain at `audit/audit.json` (`--audit-out` was given \
         alongside `--vault`)."
    } else {
        "This vault does not include an audit chain. Pass `--audit-out <dir>` alongside \
         `--vault` to include one."
    };
    format!(
        "# Hacienda CLI export\n\n\
         This bundle was produced by `hacienda extract --vault`. It contains {file_count} \
         processed {documents}.\n\n\
         **This is a thinner vault than Hacienda Studio's export** (Track J2's explicit \
         decision): the CLI has no general-purpose named-entity pipeline, so there is no \
         `entities/` directory, `GLOSSARY.md`, or `kg-export/` here — only what the CLI can \
         actually detect, PII, not general entities like people or organizations.\n\n\
         ## What's in here\n\n\
         - **`documents/`** — one redacted markdown file per input, with YAML frontmatter \
           (source, type, processing time, PII count).\n\
         - **`_manifest.json`** — the file list for this run.\n\
         - **`pii-registry.json`** — every PII detection across the run, grouped by document. \
           Not the same schema as Studio's `entities-registry.json` — this is PII findings \
           only, with no cross-document entity resolution.\n\n\
         {audit_note}\n"
    )
}

/// Write this run's audit entries to `dir` as a single pretty-printed JSON array.
///
/// The facade built by `run_extract` is fresh per invocation, so `facade.audit_entries()`
/// already scopes to exactly this run — no separate `FileAuditStore`/`NodeId` durability
/// machinery is needed for a one-shot CLI command. `AuditChain::append` re-verifies each
/// entry's hash linkage on the way out, so a chain that failed to round-trip cleanly is
/// caught here rather than handed to an auditor.
async fn write_audit_chain(facade: &HaciendaFacade, dir: &Path) -> Result<()> {
    let entries = facade
        .audit_entries()
        .await
        .context("reading this run's audit entries")?;
    let config_hash = facade
        .config()
        .pii
        .as_ref()
        .map(|p| p.audit.config_hash.clone())
        .unwrap_or_else(|| "default".to_string());

    let mut chain = AuditChain::new(config_hash);
    for entry in entries {
        chain
            .append(entry)
            .context("the audit chain does not verify")?;
    }

    std::fs::create_dir_all(dir)
        .with_context(|| format!("creating audit output directory {}", dir.display()))?;
    let bytes = export_json(&chain).context("serialising the audit chain")?;
    let out_path = dir.join("audit.json");
    std::fs::write(&out_path, bytes)
        .with_context(|| format!("writing audit chain to {}", out_path.display()))?;

    Ok(())
}

/// Write this run's entity glossary to `<dir>/glossary.json`.
///
/// `EntityGlossary` lives only inside the facade that processed these inputs — there is
/// nothing durable to reread later, which is why `--glossary-out` is a flag on
/// `extract`/`scan` rather than a standalone command. Entries are
/// `{category, term, count, mean_confidence}`, never document text.
async fn write_glossary(facade: &HaciendaFacade, dir: &Path) -> Result<()> {
    let entries = facade
        .glossary_snapshot_with_auth(Caller::Trusted)
        .await
        .context("reading this run's glossary snapshot")?;

    std::fs::create_dir_all(dir)
        .with_context(|| format!("creating glossary output directory {}", dir.display()))?;
    let out_path = dir.join("glossary.json");
    std::fs::write(&out_path, serde_json::to_vec_pretty(&entries)?)
        .with_context(|| format!("writing glossary to {}", out_path.display()))?;

    Ok(())
}

/// Refuse a bind address that would expose the API to the network without authentication.
///
/// This is the one policy `serve` enforces before opening a socket, and it is
/// deliberately not overridable by a flag. `auth.enabled = false` means every request is
/// `Caller::Trusted` — no capability is checked, `pii:reveal` included — which is correct
/// when the only client is the desktop app on the same machine and wrong the instant the
/// socket is reachable from anywhere else. On a product that holds documents covered by
/// *secret professionnel*, "reachable and unauthenticated" is not a configuration, it is
/// an incident.
///
/// Loopback covers `127.0.0.0/8` and `::1`. `0.0.0.0` and `::` are *not* loopback and are
/// refused: they are the two spellings people reach for when containerising, which is
/// exactly when this check has to hold.
fn check_bind_policy(bind: SocketAddr, auth_enabled: bool) -> Result<()> {
    if auth_enabled || bind.ip().is_loopback() {
        return Ok(());
    }
    anyhow::bail!(
        "refusing to bind {bind}: it is reachable from the network and authentication is \
         disabled, so every caller would be trusted with unredacted PII.\n\
         Either bind a loopback address (the default, 127.0.0.1:8787), or enable \
         authentication in the configuration:\n\n\
         \x20   [auth]\n\
         \x20   enabled = true\n\
         \x20   resolver = \"memory\"\n\n\
         \x20   [[auth.static_tokens]]\n\
         \x20   id = \"studio\"\n\
         \x20   token = \"<secret>\"\n\
         \x20   principal_id = \"studio\"\n\
         \x20   capabilities = [\"documents:process\"]"
    )
}

/// Run `hacienda serve` — the HTTP API of the integration design §5.
pub async fn run_serve(
    args: ServeArgs,
    config_path: Option<PathBuf>,
    config_json: Option<String>,
) -> Result<()> {
    let config = load_config(config_path, config_json, None, None)?;

    // Before the socket, not after: a bind that succeeds and is then torn down has
    // already been reachable.
    check_bind_policy(args.bind, config.auth.enabled)?;

    let auth = AuthState::from_config(&config.auth).context("building the auth state")?;
    let auth_enabled = config.auth.enabled;

    let facade = Arc::new(build_facade(config).context("building the facade")?);
    let jobs = InMemoryJobStore::new().into_arc();
    // In-memory by default, same precedent as `jobs` above: durable is a caller
    // decision (embed a `PgVectorStore` instead), not this command's to make.
    let rag_store: Arc<dyn hacienda_rag::RagStore> = Arc::new(InMemoryVectorStore::new("default"));
    let state = ApiState::new(facade, jobs, auth, ApiLimits::default()).with_rag_store(rag_store);

    let listener = tokio::net::TcpListener::bind(args.bind)
        .await
        .with_context(|| format!("binding {}", args.bind))?;
    let local_addr = listener.local_addr().context("reading the bound address")?;

    // stderr, not stdout: stdout is where the other subcommands write machine-readable
    // output, and `serve` should compose with them without polluting a pipe.
    eprintln!("hacienda serving on http://{local_addr}");
    eprintln!(
        "  authentication: {}",
        if auth_enabled {
            "enabled"
        } else {
            "DISABLED — every caller is trusted (loopback only)"
        }
    );

    axum::serve(listener, hacienda_api::router(state))
        .await
        .context("serving the API")
}

#[cfg(test)]
mod tests {
    use super::check_bind_policy;
    use std::net::SocketAddr;

    fn addr(s: &str) -> SocketAddr {
        s.parse().unwrap()
    }

    /// The default is loopback, so the common case must not need auth configured.
    #[test]
    fn should_allow_loopback_without_authentication() {
        assert!(check_bind_policy(addr("127.0.0.1:8787"), false).is_ok());
        assert!(check_bind_policy(addr("[::1]:8787"), false).is_ok());
        // 127.0.0.0/8 in full, not just 127.0.0.1.
        assert!(check_bind_policy(addr("127.0.0.53:8787"), false).is_ok());
    }

    /// The whole point of the check. `0.0.0.0` and `::` are the spellings a container
    /// setup reaches for, which is exactly when an unauthenticated API becomes reachable.
    #[test]
    fn should_refuse_a_network_reachable_bind_when_authentication_is_disabled() {
        for spelling in ["0.0.0.0:8787", "[::]:8787", "192.168.1.10:8787"] {
            let error = check_bind_policy(addr(spelling), false)
                .expect_err("binding {spelling} without auth must be refused");
            let message = error.to_string();
            assert!(
                message.contains("[auth]"),
                "the refusal must tell the operator how to proceed, got: {message}"
            );
        }
    }

    /// Enabling authentication is the sanctioned way to serve on a network address —
    /// otherwise the check above would be indistinguishable from "never bind publicly".
    #[test]
    fn should_allow_a_network_reachable_bind_once_authentication_is_enabled() {
        assert!(check_bind_policy(addr("0.0.0.0:8787"), true).is_ok());
        assert!(check_bind_policy(addr("192.168.1.10:8787"), true).is_ok());
    }
}
