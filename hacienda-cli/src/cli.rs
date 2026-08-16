//! The clap surface.
//!
//! Scope is Phases 2 and 4 of
//! `superpowers/specs/2026-07-28-hacienda-cli-api-integration-design.md` §9: `extract`,
//! `scan`, `config show`, and `serve`, plus a later, narrower slice closing part of the
//! CLI/API parity gap: `audit verify` (verifying the `--audit-out` export, not the full
//! `GET /v1/audit` surface) and `pii reveal`, plus a `--glossary-out` flag on `extract`
//! and `scan` (glossary state lives only inside a live run's facade, so there is nothing
//! for a standalone `glossary` subcommand to read — see `ExtractArgs::glossary_out`).
//!
//! A further slice closes the rest of the parity gap now that the backing
//! `hacienda-core` functionality is real rather than aspirational: `review` (operates on
//! a durable `FileReviewStore` directory, written to by `extract`/`scan --review-out` or
//! by another process embedding `hacienda-core` directly), `compliance` (DPIA, model
//! card, checklist, and DORA incident reports — pure functions of configuration plus, for
//! DORA, an `--incident` file, so these never need a facade or a store), the rest of
//! `audit` (`list`/`export` against a durable `FileAuditStore` directory — distinct from
//! `verify`, which reads the flat `--audit-out` export), and `completions`. The `xberg`
//! passthrough remains deliberately absent — a separate, larger design question this
//! slice does not settle.

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
    /// Serve the Model Context Protocol (MCP) surface, for Claude Desktop and other
    /// local MCP clients.
    ///
    /// Runs the same `HaciendaFacade` `hacienda serve` would build from this
    /// configuration, over stdio instead of HTTP. Like `pii reveal`, this process is
    /// `Caller::Trusted` — the process boundary is the trust boundary, appropriate for a
    /// locally spawned MCP server, not for a network-reachable one.
    Mcp(McpArgs),
    /// Operations on the human review queue (AI Act Art. 14 human oversight).
    Review {
        #[command(subcommand)]
        command: ReviewCommand,
    },
    /// Generate GDPR/AI-Act/DORA compliance artefacts.
    Compliance {
        #[command(subcommand)]
        command: ComplianceCommand,
    },
    /// Print a shell completion script to stdout.
    Completions(CompletionsArgs),
}

#[derive(Debug, Parser)]
pub struct McpArgs {
    #[command(subcommand)]
    pub command: McpCommand,
}

#[derive(Debug, Subcommand)]
pub enum McpCommand {
    /// Start the MCP server on the stdio transport.
    Serve,
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

    /// Page through a durable `FileAuditStore`'s history.
    ///
    /// Distinct from `verify`: this reads the segmented, per-writer store
    /// `FileAuditStore::open` produces (a `root/{node}/` layout of sealed and open
    /// segments), not the flat `audit.json` array `--audit-out` writes. Nothing in this
    /// CLI creates that layout today — `hacienda serve` still audits in-memory only, and
    /// `extract`/`scan --audit-out` deliberately stay on the flat, single-run export (see
    /// `write_audit_chain`) — so this command reads whatever a `FileAuditStore` another
    /// process (a library embedding `hacienda-core` directly, or a future `serve`
    /// enhancement) already wrote there.
    List(AuditListArgs),

    /// Export a durable `FileAuditStore`'s full history to a file.
    ///
    /// Same store shape as `list`, not `--audit-out`'s flat export. Pages the entire
    /// history via `AuditStore::history` and hands it to the same `hacienda_core::audit`
    /// export functions `extract --audit-out` and the HTTP API's `/v1/audit/export` both
    /// call — one export implementation, three callers.
    Export(AuditExportArgs),
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

/// Shared by `audit list` and `audit export`: which segment directory to open.
#[derive(Debug, Parser)]
pub struct AuditStoreArgs {
    /// Root directory of a durable `FileAuditStore` (as passed to `FileAuditStore::open`).
    #[arg(value_name = "DIR")]
    pub dir: PathBuf,

    /// The writer identity whose segment directory to open, i.e. `<DIR>/<NODE>/`.
    ///
    /// Segments are per-writer (see `AuditStore::history`'s own "this node, not this
    /// deployment" doc) — there is no way to discover a NodeId from the directory alone
    /// without guessing, and guessing wrong would silently open (and start writing an
    /// empty segment into) the wrong writer's history. Required rather than defaulted for
    /// the same reason `--i-accept-unredacted-pii` has no implicit default: naming the
    /// writer is the caller's job, not this command's to infer.
    #[arg(long, value_name = "ID")]
    pub node: String,

    /// The config hash this invocation opens the store under.
    ///
    /// Only load-bearing if the node directory holds an *unsealed* segment from a
    /// previous run — `FileAuditStore::open` replays it and rejects entries minted under
    /// a different config hash (`AuditError::ConfigMismatch`). Defaults to `"default"`,
    /// matching `write_audit_chain`'s own fallback when no `[pii.audit] config_hash` was
    /// configured for the run that wrote the store.
    #[arg(long, value_name = "HASH", default_value = "default")]
    pub config_hash: String,
}

#[derive(Debug, Parser)]
pub struct AuditListArgs {
    #[command(flatten)]
    pub store: AuditStoreArgs,

    /// Resume from the `next` cursor a previous `audit list` call printed.
    ///
    /// Opaque — see `AuditCursor`'s own doc. Omit to start from the beginning of this
    /// node's history.
    #[arg(long, value_name = "CURSOR")]
    pub after: Option<hacienda::audit::AuditCursor>,

    /// Maximum entries to return in this page.
    #[arg(long, value_name = "N", default_value_t = 50)]
    pub limit: usize,

    #[arg(long, value_enum, default_value_t = Format::Text)]
    pub format: Format,
}

#[derive(Debug, Parser)]
pub struct AuditExportArgs {
    #[command(flatten)]
    pub store: AuditStoreArgs,

    /// File to write the export to.
    #[arg(long, value_name = "FILE")]
    pub out: PathBuf,

    #[arg(long, value_enum, default_value_t = AuditExportFormat::Json)]
    pub format: AuditExportFormat,
}

/// A local mirror of `hacienda_core::audit::ExportFormat`, matching `Mode`'s precedent for
/// keeping `clap::ValueEnum` out of `hacienda-core` — converted at the call site in
/// `commands.rs`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum AuditExportFormat {
    Json,
    Csv,
    JsonLines,
}

#[derive(Debug, Subcommand)]
pub enum ReviewCommand {
    /// List items in a review queue, optionally filtered by status.
    List(ReviewListArgs),
    /// Show a single review item by id.
    Show(ReviewShowArgs),
    /// Claim a pending item for review, moving it to `in_review`.
    Assign(ReviewAssignArgs),
    /// Record a reviewer's decision on an item.
    Decide(ReviewDecideArgs),
    /// Print queue counts by status.
    Stats(ReviewStatsArgs),
}

/// Shared by every `review` subcommand: which durable queue to open.
#[derive(Debug, Parser)]
pub struct ReviewStoreArgs {
    /// Directory holding the review queue's event log (`<DIR>/review.jsonl`).
    ///
    /// Written by `extract`/`scan --review-out <DIR>` (see `ExtractArgs::review_out`), or
    /// by another process embedding `hacienda-core::review::FileReviewStore` directly. If
    /// `<DIR>/review.jsonl` does not exist yet, it is created empty — matching
    /// `FileReviewStore::open`'s own "create if absent" behaviour — so `review list` on a
    /// fresh directory reports zero items rather than failing.
    #[arg(value_name = "DIR")]
    pub dir: PathBuf,
}

#[derive(Debug, Parser)]
pub struct ReviewListArgs {
    #[command(flatten)]
    pub store: ReviewStoreArgs,

    /// Restrict to items in this status. Omit to list every item regardless of status.
    #[arg(long, value_enum)]
    pub status: Option<ReviewStatusArg>,

    #[arg(long, value_enum, default_value_t = Format::Text)]
    pub format: Format,
}

#[derive(Debug, Parser)]
pub struct ReviewShowArgs {
    #[command(flatten)]
    pub store: ReviewStoreArgs,

    /// The review item's id, as printed by `review list`.
    #[arg(value_name = "ID")]
    pub id: String,

    #[arg(long, value_enum, default_value_t = Format::Text)]
    pub format: Format,
}

#[derive(Debug, Parser)]
pub struct ReviewAssignArgs {
    #[command(flatten)]
    pub store: ReviewStoreArgs,

    /// The review item's id, as printed by `review list`.
    #[arg(value_name = "ID")]
    pub id: String,

    /// Identifies the reviewer claiming this item. Recorded verbatim, not authenticated —
    /// the CLI is in-process and trusted (the same `Caller::Trusted` precedent `pii
    /// reveal` documents: the process boundary is the trust boundary), so there is no
    /// caller identity to derive this from.
    #[arg(long)]
    pub reviewer: String,

    #[arg(long, value_enum, default_value_t = Format::Text)]
    pub format: Format,
}

#[derive(Debug, Parser)]
pub struct ReviewDecideArgs {
    #[command(flatten)]
    pub store: ReviewStoreArgs,

    /// The review item's id, as printed by `review list`.
    #[arg(value_name = "ID")]
    pub id: String,

    #[arg(long, value_enum)]
    pub decision: ReviewDecisionArg,

    /// Identifies the reviewer making this decision. See `ReviewAssignArgs::reviewer` for
    /// why this is an unauthenticated free-text field in the CLI.
    #[arg(long)]
    pub reviewer: String,

    /// Free-text rationale, stored alongside the decision.
    #[arg(long, default_value = "")]
    pub comment: String,

    #[arg(long, value_enum, default_value_t = Format::Text)]
    pub format: Format,
}

#[derive(Debug, Parser)]
pub struct ReviewStatsArgs {
    #[command(flatten)]
    pub store: ReviewStoreArgs,

    #[arg(long, value_enum, default_value_t = Format::Text)]
    pub format: Format,
}

/// A local mirror of `hacienda_core::review::ReviewStatus`. `ReviewStatus` already
/// implements `FromStr`, but its `Err` is a plain `String`, which does not satisfy
/// clap's blanket `value_parser` inference (`Into<Box<dyn Error + Send + Sync>>`) — and a
/// local `ValueEnum` gets `--help` to enumerate the four choices for free, which a bare
/// `FromStr` parser would not. Same precedent as `Mode`/`AuditExportFormat`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum ReviewStatusArg {
    Pending,
    InReview,
    Approved,
    Rejected,
    Modified,
}

/// A local mirror of `hacienda_core::review::ReviewDecision`. See `ReviewStatusArg` for
/// why this is not just `#[arg(value_parser = ...)]` over the core type's `FromStr`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum ReviewDecisionArg {
    Approve,
    Reject,
    Modify,
}

#[derive(Debug, Subcommand)]
pub enum ComplianceCommand {
    /// Generate a Data Protection Impact Assessment (GDPR Art. 35).
    Dpia(ComplianceFormatArgs),
    /// Generate a model card (AI Act Art. 11 technical documentation).
    ModelCard(ComplianceFormatArgs),
    /// Generate the GDPR/AI-Act/DORA article-to-control checklist.
    Checklist(ComplianceFormatArgs),
    /// Generate a DORA major-incident report from a PII incident description.
    Dora(ComplianceDoraArgs),
    /// Generate the full compliance pack (every report named in `[compliance]
    /// enabled_reports`).
    Report(ComplianceReportArgs),
}

#[derive(Debug, Parser)]
pub struct ComplianceFormatArgs {
    #[arg(long, value_enum, default_value_t = Format::Text)]
    pub format: Format,
}

#[derive(Debug, Parser)]
pub struct ComplianceDoraArgs {
    /// Path to a JSON-encoded `PiiIncident` (`summary`, `timeline`, `root_cause`,
    /// `detected_at`, `contained_at`, `resolved_at`, `actions_taken`,
    /// `lessons_learned`).
    ///
    /// A DORA report describes a specific event; there is no live incident data in a
    /// one-shot CLI run to draw one from, so it is read from a file rather than
    /// synthesised. This is not optional the way it is on `compliance report`: an
    /// operator who ran `compliance dora` asked for exactly this report, so a missing
    /// incident is refused rather than silently producing nothing.
    #[arg(long, value_name = "FILE")]
    pub incident: PathBuf,

    #[arg(long, value_enum, default_value_t = Format::Text)]
    pub format: Format,
}

#[derive(Debug, Parser)]
pub struct ComplianceReportArgs {
    /// Same `PiiIncident` JSON shape as `compliance dora --incident`. Optional here:
    /// `ComplianceGenerator::report` already omits the DORA section when no incident is
    /// given (rather than erroring), so this command exposes that behaviour as-is instead
    /// of forcing every report to carry incident data it does not have.
    #[arg(long, value_name = "FILE")]
    pub incident: Option<PathBuf>,

    #[arg(long, value_enum, default_value_t = Format::Text)]
    pub format: Format,
}

#[derive(Debug, Parser)]
pub struct CompletionsArgs {
    /// The shell to generate a completion script for.
    #[arg(value_enum)]
    pub shell: clap_complete::Shell,
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

    /// Materialise this run's review submissions into a durable queue at
    /// `<DIR>/review.jsonl`, readable and actionable afterwards via `hacienda review
    /// list|show|assign|decide|stats <DIR>`.
    ///
    /// Unlike `--audit-out` and `--glossary-out`, which overwrite their target on every
    /// run, this *accumulates*: `FileReviewStore::open` replays whatever this directory
    /// already holds before this run's submissions are appended, so repeated
    /// `extract --review-out` runs build one durable reviewer inbox rather than each
    /// silently discarding the last run's queue. Materialises a `[review]` config
    /// section (`ReviewConfig::default`) if none was loaded, mirroring
    /// `--glossary-out`'s materialisation of `[glossary]` — see
    /// `ExtractArgs::glossary_out`. Refused when combined with `--no-redact`, which skips
    /// the PII stage this flag depends on — see `--audit-out`'s identical guard.
    #[arg(long, value_name = "DIR")]
    pub review_out: Option<PathBuf>,

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

    /// Materialise this run's review submissions into a durable queue at
    /// `<DIR>/review.jsonl`. See `ExtractArgs::review_out` — same artefact, same
    /// accumulate-across-runs behaviour. `scan` already runs full PII detection, so it
    /// populates the same queue the same way; there is no `--no-redact` on `scan` to
    /// conflict with it.
    #[arg(long, value_name = "DIR")]
    pub review_out: Option<PathBuf>,

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
