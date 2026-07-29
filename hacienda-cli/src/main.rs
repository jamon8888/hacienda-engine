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
        }
    });

    match result {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("Error: {e}");
            std::process::ExitCode::FAILURE
        }
    }
}
