//! The `hacienda` binary.

mod cli;
mod commands;
mod config;

use clap::Parser;
use cli::{Cli, Command};

fn main() -> std::process::ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn")),
        )
        .with_writer(std::io::stderr)
        .init();

    let cli = Cli::parse();

    // `completions` generates a static script from the parser alone — no config to load,
    // no facade to build, no async I/O — so it runs before the runtime is even spun up.
    if let Command::Completions(args) = cli.command {
        return match commands::run_completions(args) {
            Ok(()) => std::process::ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("Error: {e}");
                std::process::ExitCode::FAILURE
            }
        };
    }

    let rt = tokio::runtime::Runtime::new().expect("failed to create tokio runtime");
    let result = rt.block_on(async {
        match cli.command {
            Command::Extract(args) => {
                commands::run_extract(args, cli.config, cli.config_json).await
            }
            Command::Scan(args) => commands::run_scan(args, cli.config, cli.config_json).await,
            Command::Config { command } => match command {
                cli::ConfigCommand::Show { format } => {
                    commands::run_config_show(format, cli.config, cli.config_json).await
                }
            },
            Command::Serve(args) => commands::run_serve(args, cli.config, cli.config_json).await,
            Command::Pii { command } => match command {
                cli::PiiCommand::Reveal(args) => {
                    commands::run_pii_reveal(args, cli.config, cli.config_json).await
                }
            },
            Command::Audit { command } => match command {
                cli::AuditCommand::Verify(args) => commands::run_audit_verify(args).await,
                cli::AuditCommand::List(args) => commands::run_audit_list(args).await,
                cli::AuditCommand::Export(args) => commands::run_audit_export(args).await,
            },
            Command::Mcp(args) => match args.command {
                cli::McpCommand::Serve => {
                    commands::run_mcp_serve(cli.config, cli.config_json).await
                }
            },
            Command::Review { command } => match command {
                cli::ReviewCommand::List(args) => commands::run_review_list(args).await,
                cli::ReviewCommand::Show(args) => commands::run_review_show(args).await,
                cli::ReviewCommand::Assign(args) => commands::run_review_assign(args).await,
                cli::ReviewCommand::Decide(args) => commands::run_review_decide(args).await,
                cli::ReviewCommand::Stats(args) => commands::run_review_stats(args).await,
            },
            Command::Compliance { command } => match command {
                cli::ComplianceCommand::Dpia(args) => {
                    commands::run_compliance_dpia(args, cli.config, cli.config_json).await
                }
                cli::ComplianceCommand::ModelCard(args) => {
                    commands::run_compliance_model_card(args, cli.config, cli.config_json).await
                }
                cli::ComplianceCommand::Checklist(args) => {
                    commands::run_compliance_checklist(args, cli.config, cli.config_json).await
                }
                cli::ComplianceCommand::Dora(args) => {
                    commands::run_compliance_dora(args, cli.config, cli.config_json).await
                }
                cli::ComplianceCommand::Report(args) => {
                    commands::run_compliance_report(args, cli.config, cli.config_json).await
                }
            },
            Command::Completions(_) => unreachable!("handled above, before the runtime starts"),
        }
    });

    match result {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(e) => {
            // `{:#}` (anyhow's alternate `Display`) prints the *whole* context chain
            // joined by ": ", not just the outermost `.context(...)`/`.with_context(...)`
            // layer. `review decide`'s "already decided" and "not found" and `assign`'s
            // "invalid transition" all come from `hacienda_core::review::ReviewError`
            // messages wrapped one layer deeper by `run_review_decide`/`run_review_assign`
            // — printing only the top layer would show "deciding review item '<id>'" and
            // silently drop the one detail (*why* it failed) the caller actually needs.
            eprintln!("Error: {e:#}");
            std::process::ExitCode::FAILURE
        }
    }
}
