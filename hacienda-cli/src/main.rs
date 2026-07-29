//! The `hacienda` binary.

mod cli;

use clap::Parser;
use cli::{Cli, Command, ConfigCommand};

fn main() -> std::process::ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn")),
        )
        .with_writer(std::io::stderr)
        .init();

    let cli = Cli::parse();

    // Task 2 ships the parse surface and nothing behind it. Each arm lands in its own
    // task: `extract` and `scan` in Task 6, `config show` in Task 3.
    let unimplemented = match &cli.command {
        Command::Extract(_) => "extract",
        Command::Scan(_) => "scan",
        Command::Config {
            command: ConfigCommand::Show { .. },
        } => "config show",
    };
    eprintln!("hacienda: `{unimplemented}` is not implemented yet");
    std::process::ExitCode::from(70)
}
