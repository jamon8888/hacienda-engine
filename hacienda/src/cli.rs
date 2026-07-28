use clap::{CommandFactory, Parser, Subcommand};
use xberg_cli::{Cli as XbergCli, Commands as XbergCommands};

#[derive(Parser)]
#[command(name = "hacienda", version, about = "xberg + PII/Compliance")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand)]
pub enum Commands {
    // All xberg commands
    #[command(flatten)]
    Xberg(XbergCommands),

    // hacienda-specific
    /// PII detection & redaction
    Pii(PiiCommands),

    /// Compliance reports (DPIA, Model Card, DORA)
    Compliance(ComplianceCommands),

    /// Human review queue
    Review(ReviewCommands),

    /// Audit log management
    Audit(AuditCommands),
}

#[derive(Parser)]
pub enum PiiCommands {
    /// Scan text for PII entities
    Scan { text: String },
    /// Redact PII from text
    Redact { text: String },
    /// Explain a detection
    Explain { text: String, index: usize },
    /// Batch process multiple texts
    Batch { file: String },
    /// Model management
    Model(ModelCommands),
}

#[derive(Parser)]
pub enum ModelCommands {
    /// Download model
    Download { model: String },
    /// List available models
    List,
    /// Show model info
    Info { model: String },
}

#[derive(Parser)]
pub enum ComplianceCommands {
    /// Generate full compliance report
    Report { model: String },
    /// Generate DPIA
    Dpia { model: String },
    /// Generate Model Card
    ModelCard { model: String },
    /// Generate DORA report
    Dora { model: String },
    /// Generate AI Act checklist
    AiAct { model: String },
    /// Generate all checklists
    Checklist { model: String },
}

#[derive(Parser)]
pub enum ReviewCommands {
    /// Submit item for review
    Submit { text: String, category: String },
    /// List review items
    List { status: Option<String> },
    /// Get review item
    Get { id: String },
    /// Make decision
    Decide {
        id: String,
        decision: String,
        reviewer: String,
        comment: String,
    },
    /// Assign reviewer
    Assign { id: String, reviewer: String },
    /// Queue statistics
    Stats,
}

#[derive(Parser)]
pub enum AuditCommands {
    /// Query audit logs
    Query {
        from: Option<String>,
        to: Option<String>,
        category: Option<String>,
    },
    /// Export audit logs
    Export {
        format: String,
        from: Option<String>,
        to: Option<String>,
    },
    /// Verify chain integrity
    Verify,
    /// Statistics
    Stats,
}
