//! Command implementations for the hacienda CLI.

use crate::cli::{ConcurrencyArgs, ExtractArgs, Format, ScanArgs, ServeArgs};
use crate::config::load_config;
use anyhow::{Context, Result};
use hacienda::audit::{export_json, AuditChain};
use hacienda::{HaciendaConfig, HaciendaFacade};
use hacienda_api::{ApiLimits, ApiState};
use hacienda_core::auth::AuthState;
use hacienda_core::jobs::InMemoryJobStore;
use hacienda_core::pii::PipelineConfig;
use hacienda_core::redaction::RedactionMode;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::task;
use xberg::ExtractInput;

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

    let mut pii_config = config.pii.clone().unwrap_or_else(|| {
        let mut p = PipelineConfig::default();
        p.redaction.mode = RedactionMode::Mask; // doesn't matter for scan
        p
    });
    // `--concurrency` governs documents in flight through the PII stage
    // (`PiiPipeline::process`), not the CLI's file list — a single scanned input can
    // still decompose into several documents (e.g. a multi-page PDF).
    pii_config.concurrency = parse_concurrency(&args.concurrency);

    let facade = Arc::new(HaciendaFacade::new(HaciendaConfig {
        extraction: config.extraction,
        pii: Some(pii_config),
        compliance: config.compliance,
        review: config.review,
        glossary: config.glossary,
        auth: config.auth,
    })?);

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

    // Validate --no-redact requires explicit acknowledgement
    if args.no_redact && !args.i_accept_unredacted_pii {
        anyhow::bail!(
            "--no-redact requires --i-accept-unredacted-pii (explicit acknowledgement that raw PII will be output)"
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

    let facade = Arc::new(HaciendaFacade::new(HaciendaConfig {
        extraction: config.extraction,
        pii: pii_config,
        compliance: config.compliance,
        review: config.review,
        glossary: config.glossary,
        auth: config.auth,
    })?);

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

    if let Some(audit_out) = &args.audit_out {
        write_audit_chain(&facade, audit_out).await?;
    }

    Ok(())
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

    let facade = Arc::new(HaciendaFacade::new(config).context("building the facade")?);
    let jobs = InMemoryJobStore::new().into_arc();
    let state = ApiState::new(facade, jobs, auth, ApiLimits::default());

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
