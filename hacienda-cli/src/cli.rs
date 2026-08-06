//! The clap surface.
//!
//! Scope is Phases 2 and 4 of
//! `superpowers/specs/2026-07-28-hacienda-cli-api-integration-design.md` §9: `extract`,
//! `scan`, `config show`, and `serve`, plus a later, narrower slice closing part of the
//! CLI/API parity gap: `audit verify` (verifying the `--audit-out` export, not the full
//! `GET /v1/audit` surface) and `pii reveal`, plus a `--glossary-out` flag on `extract`
//! and `scan` (glossary state lives only inside a live run's facade, so there is nothing
//! for a standalone `glossary` subcommand to read — see `ExtractArgs::glossary_out`).
//! `review`, `compliance`, the rest of `audit`, `completions`, and the `xberg`
//! passthrough remain deliberately absent rather than present and stubbed. A
//! subcommand that parses and then apologises is indistinguishable from one that is
//! broken.

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
    /// Operations on pseudonym tokens.
    Pii {
        #[command(subcommand)]
        command: PiiCommand,
    },
    /// Operations on the audit chain.
    Audit {
        #[command(subcommand)]
        command: AuditCommand,
    },
}

#[derive(Debug, Subcommand)]
pub enum PiiCommand {
    /// Reverse a pseudonym token to its plaintext.
    ///
    /// The CLI is in-process, so this always runs as the trusted caller — the same
    /// precedent `serve` documents for `Caller::Trusted` (the process boundary is the
    /// trust boundary). Requires the same `HACIENDA_PSEUDONYM_*` environment variables
    /// as `--mode pseudonymize`. A token minted under a key this process cannot resolve
    /// and a malformed or tampered token both fail with the same generic message,
    /// deliberately — mirroring the HTTP API's `PseudonymError` collapsing, so a caller
    /// cannot use error text to probe which part of a token or key is wrong.
    Reveal(RevealArgs),
}

#[derive(Debug, Parser)]
pub struct RevealArgs {
    /// The pseudonym token to reverse, e.g. `[EMAIL:k1:base32...]`.
    #[arg(value_name = "TOKEN")]
    pub token: String,

    /// Output format for the revealed plaintext.
    #[arg(long, value_enum, default_value_t = Format::Text)]
    pub format: Format,
}

#[derive(Debug, Subcommand)]
pub enum AuditCommand {
    /// Independently re-verify an audit chain written by `--audit-out`.
    ///
    /// Verifies the flat `audit.json` export (a `Vec<AuditEntry>`), the same shape
    /// `extract --audit-out`/`--vault` write — not the segmented `FileAuditStore`
    /// format `hacienda serve` uses for durable storage. A one-shot CLI verification has
    /// no need for that store's `NodeId`/segment-sealing machinery; see
    /// `write_audit_chain`'s own doc comment in `commands.rs` for the same call applied
    /// to writing.
    Verify(AuditVerifyArgs),
}

#[derive(Debug, Parser)]
pub struct AuditVerifyArgs {
    /// Directory containing `audit.json`, as written by `--audit-out`.
    #[arg(value_name = "DIR")]
    pub dir: PathBuf,

    /// Output format for the verification result.
    #[arg(long, value_enum, default_value_t = Format::Text)]
    pub format: Format,
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

    /// Emit a Track I2 vault to this directory instead of (or alongside) stdout: a
    /// `documents/` folder of redacted markdown, `_manifest.json`, `pii-registry.json`, and
    /// `README.md`. Deliberately thinner than Studio's vault (no `entities/`, `GLOSSARY.md`,
    /// or `kg-export/`) — see Track J2: the CLI has no general-purpose NER entity pipeline,
    /// so there is no entity graph to emit one honestly. Combine with `--audit-out` to also
    /// carry the audit chain inside the vault.
    #[arg(long, value_name = "DIR")]
    pub vault: Option<PathBuf>,

    /// Write this run's entity glossary to `<DIR>/glossary.json`.
    ///
    /// Glossary state (`EntityGlossary`) lives only inside this run's facade, populated
    /// as documents are processed — there is nothing to read outside a live
    /// `extract`/`scan` run, which is why this is a flag here rather than a standalone
    /// `hacienda glossary` subcommand. Entries are `{category, term, count,
    /// mean_confidence}` — never document text. Distinct from `--vault`'s absent
    /// `GLOSSARY.md`: that one is a general NER entity-linking graph the CLI has no
    /// pipeline for (Track K); this is the PII-category glossary the facade already
    /// produces. Materialises a `[glossary]` config section (enabled) if none was
    /// loaded, mirroring how `--mode` materialises `[pii]`.
    #[arg(long, value_name = "DIR")]
    pub glossary_out: Option<PathBuf>,

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

    /// Write this run's entity glossary to `<DIR>/glossary.json`.
    ///
    /// See `ExtractArgs::glossary_out` — same artifact, same reasoning. `scan` already
    /// runs full PII detection, so it populates the same glossary the same way; an
    /// entry is `{category, term, count, mean_confidence}`, never document text, so
    /// offering this here does not violate scan's "no document text" contract.
    #[arg(long, value_name = "DIR")]
    pub glossary_out: Option<PathBuf>,

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
    /// Measured against a 300-document fixed corpus (§9, Phase 2, tracking issue #31):
    /// raising this past 1 did not reach 2x throughput at CPU count, and in some runs
    /// made wall time worse. The audit store's `io_order` lock is not the reason — its
    /// measured wait stayed under 0.2% of wall time at every concurrency level tested.
    /// Do not expect this flag alone to scale throughput; the ceiling on constrained
    /// hardware is elsewhere and is not yet root-caused.
    #[arg(long, value_name = "N")]
    pub concurrency: Option<usize>,
}
