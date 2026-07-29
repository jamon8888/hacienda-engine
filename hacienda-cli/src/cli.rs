//! The clap surface.
//!
//! Scope is Phases 2 and 4 of
//! `superpowers/specs/2026-07-28-hacienda-cli-api-integration-design.md` §9: `extract`,
//! `scan`, `config show`, and `serve`. The remaining commands of §6.2 — `audit`,
//! `review`, `compliance`, `glossary`, `completions`, and the `xberg` passthrough —
//! belong to Phases 5 through 7 and are deliberately absent rather than present and
//! stubbed. A subcommand that parses and then apologises is indistinguishable from one
//! that is broken.

use clap::{Parser, Subcommand, ValueEnum};
use std::net::SocketAddr;
use std::path::PathBuf;

/// Redaction-by-default document extraction.
///
/// The polarity is inverted from `xberg` on purpose: `xberg extract` yields text,
/// `hacienda extract` yields *redacted* text, and anything else must be asked for loudly.
#[derive(Debug, Parser)]
#[command(name = "hacienda", version, about, long_about = None)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,

    /// Path to a configuration file, overriding any discovered `hacienda.toml`.
    #[arg(long, global = true, value_name = "PATH")]
    pub config: Option<PathBuf>,

    /// Inline JSON configuration, overriding `--config`.
    #[arg(long, global = true, value_name = "JSON")]
    pub config_json: Option<String>,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Extract documents, redacted by default.
    Extract(ExtractArgs),
    /// Detect PII without rewriting anything. Emits no document text.
    Scan(ScanArgs),
    /// Inspect configuration.
    Config {
        #[command(subcommand)]
        command: ConfigCommand,
    },
    /// Serve the HTTP API.
    Serve(ServeArgs),
}

#[derive(Debug, Parser)]
pub struct ServeArgs {
    /// Address to bind.
    ///
    /// Loopback by default. This API serves document content, so a default of
    /// `0.0.0.0` would put every client's corpus one `docker run -p` away from the
    /// network — and the operator would never have typed a flag that said so.
    ///
    /// Binding a non-loopback address is refused unless `auth.enabled` is `true` in the
    /// configuration. There is no override flag: the combination "reachable from the
    /// network" plus "no authentication" has no legitimate use on this product.
    #[arg(long, value_name = "ADDR", default_value = "127.0.0.1:8787")]
    pub bind: SocketAddr,
}

#[derive(Debug, Subcommand)]
pub enum ConfigCommand {
    /// Print the effective configuration and where each value came from.
    Show {
        #[arg(long, value_enum, default_value_t = Format::Text)]
        format: Format,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum Format {
    Text,
    Json,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum Mode {
    Mask,
    Hash,
    Pseudonymize,
}

#[derive(Debug, Parser)]
pub struct ExtractArgs {
    /// Files or URIs to extract.
    #[arg(required = true, value_name = "INPUT")]
    pub inputs: Vec<String>,

    /// Redaction mode. `pseudonymize` requires a key; it fails rather than falling back
    /// to masking, because masking and pseudonymisation are different controls.
    #[arg(long, value_enum)]
    pub mode: Option<Mode>,

    /// Minimum detection confidence.
    #[arg(long, value_name = "F32")]
    pub threshold: Option<f32>,

    /// Directory holding the NER model.
    #[arg(long, value_name = "DIR")]
    pub model_dir: Option<PathBuf>,

    /// Directory holding LoRA adapters.
    #[arg(long, value_name = "DIR")]
    pub lora_dir: Option<PathBuf>,

    #[arg(long, value_enum, default_value_t = Format::Text)]
    pub format: Format,

    /// Write the audit chain for this run to a directory.
    #[arg(long, value_name = "PATH")]
    pub audit_out: Option<PathBuf>,

    /// Emit unredacted text. Refused on its own — see `--i-accept-unredacted-pii`.
    #[arg(long)]
    pub no_redact: bool,

    /// The explicit acknowledgement `--no-redact` requires.
    #[arg(long, requires = "no_redact")]
    pub i_accept_unredacted_pii: bool,

    #[command(flatten)]
    pub concurrency: ConcurrencyArgs,
}

#[derive(Debug, Parser)]
pub struct ScanArgs {
    /// Files or URIs to scan.
    #[arg(required = true, value_name = "INPUT")]
    pub inputs: Vec<String>,

    #[arg(long, value_name = "F32")]
    pub threshold: Option<f32>,

    #[arg(long, value_enum, default_value_t = Format::Text)]
    pub format: Format,

    #[command(flatten)]
    pub concurrency: ConcurrencyArgs,
}

/// Shared so that the ceiling below is stated identically wherever the flag appears.
#[derive(Debug, Parser)]
pub struct ConcurrencyArgs {
    /// Documents processed in parallel through the PII stage. Defaults to CPU count.
    ///
    /// This is hacienda's PII worker count, not xberg's extraction thread budget — that
    /// one lives at `extraction.concurrency.max_threads` in the config file and is not
    /// changed by this flag. Run `hacienda config show` to see both.
    ///
    /// Raising it past a few workers yields less than it looks like it should: every
    /// document's audit entry is appended through one segment, and that append is
    /// serialised and fsynced. Audit append is the ceiling.
    #[arg(long, value_name = "N")]
    pub concurrency: Option<usize>,
}
