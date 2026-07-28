# Implementation Plan: Interfaces (MCP + REST API + CLI + WASM)

**Spec Reference:** `docs/superpowers/specs/2025-07-24-xberg-pii-ecosystem-design.md`  
**Plan Version:** 1.0  
**Target Crates:** `pii-mcp-server`, `pii-api`, `pii-cli`, `pii-wasm`  
**Priority:** Should Have (v0.2.0)

---

## 📋 Plan Overview

| Task | Description | Est. Hours | Dependencies |
|------|-------------|------------|--------------|
| 1 | `pii-mcp-server` crate (9 tools + 4 resources + 3 prompts) | 8 | Core pipeline |
| 2 | `pii-api` crate (Axum + OpenAPI + Auth) | 8 | Core pipeline |
| 3 | `pii-cli` crate (Clap commands) | 5 | Core pipeline |
| 4 | `pii-wasm` crate (wasm-bindgen) | 6 | Core pipeline |
| 5 | Integration tests + MCP contract tests | 4 | Tasks 1-4 |
| 6 | Distribution configs (Docker, Helm, NPM) | 3 | Tasks 1-4 |

**Total Estimated:** ~33 hours

---

## 🤖 Task 1: `pii-mcp-server` Crate

### Files

```text
crates/pii-mcp-server/
├── Cargo.toml
├── src/
│   ├── main.rs
│   ├── lib.rs
│   ├── tools.rs
│   ├── resources.rs
│   ├── prompts.rs
│   └── pipeline.rs
└── tests/
    └── mcp_contract_tests.rs
```

### Cargo.toml

```toml
[package]
name = "pii-mcp-server"
version = "0.1.0"
edition = "2021"
description = "MCP server for PII detection & redaction"
license = "Apache-2.0"

[dependencies]
rmcp = { version = "0.2", features = ["server", "macros", "transport-io", "transport-streamable-http-server"] }
tokio = { workspace = true, features = ["rt-multi-thread", "macros", "fs", "time"] }
serde = { workspace = true }
serde_json = { workspace = true }
serde_wasm_bindgen = "0.6"
schemars = "0.8"
tracing = { workspace = true }
pii-pipeline = { path = "../pii-pipeline" }
pii-config = { path = "../pii-config" }
pii-compliance = { path = "../pii-compliance" }
pii-review = { path = "../pii-review" }
```

### lib.rs - Server Handler

```rust
use rmcp::{ServerHandler, tool, tool_router, resource, resource_router, prompt, prompt_router, ErrorData as McpError};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Clone)]
pub struct PiiMcpServer {
    pipeline: Arc<pii_pipeline::PiiPipeline>,
    compliance: Arc<pii_compliance::ComplianceGenerator>,
    review_queue: Arc<pii_review::ReviewQueue>,
}

#[tool_router]
impl PiiMcpServer {
    // ─────────────────────────────────────────────────────────────
    // 1. SCAN
    // ─────────────────────────────────────────────────────────────
    #[tool(description = "Scan text for PII using regex + Fastino GLiNER2. Returns entities with confidence, source, position. Zero network.")]
    async fn pii_scan(&self, args: ScanArgs) -> Result<ScanResult, McpError> {
        let config = args.config.unwrap_or_default();
        let entities = self.pipeline.scan(&args.text, &config).await?;
        
        Ok(ScanResult {
            entities: entities.into_iter().map(|e| ScanEntity {
                category: e.category,
                text: args.text[e.start as usize..e.end as usize].to_string(),
                start: e.start,
                end: e.end,
                confidence: e.confidence,
                source: e.source,
            }).collect(),
            metrics: ScanMetrics { regex_ms: 0, model_ms: 0, total_ms: 0 },
        })
    }

    // ─────────────────────────────────────────────────────────────
    // 2. REDACT
    // ─────────────────────────────────────────────────────────────
    #[tool(description = "Detect and redact PII. Modes: mask|hash|pseudonymize|remove|custom. Returns redacted_text + audit_log (NO PII). GDPR Art. 25/32.")]
    async fn pii_redact(&self, args: RedactArgs) -> Result<RedactResult, McpError> {
        let result = self.pipeline.process(&args.text, &args.config).await?;
        
        Ok(RedactResult {
            redacted_text: result.redacted_text,
            entities: if args.config.return_entities {
                Some(result.entities.into_iter().map(|e| ScanEntity {
                    category: e.category,
                    text: args.text[e.start as usize..e.end as usize].to_string(),
                    start: e.start,
                    end: e.end,
                    confidence: e.confidence,
                    source: e.source,
                }).collect())
            } else { None },
            audit_log: if args.config.return_audit_log {
                Some(result.audit_log)
            } else { None },
            metrics: result.metrics,
        })
    }

    // ─────────────────────────────────────────────────────────────
    // 3. EXPLAIN - AI Act Art. 13
    // ─────────────────────────────────────────────────────────────
    #[tool(description = "Explain why an entity was detected/redacted. Returns: rule (regex/model), confidence, threshold used, span context. AI Act Art. 13.")]
    async fn pii_explain(&self, args: ExplainArgs) -> Result<ExplainResult, McpError> {
        let explanation = self.pipeline.explain(&args.text, args.entity_index).await?;
        
        Ok(ExplainResult {
            entity: explanation.entity,
            detection_method: explanation.method,
            confidence: explanation.confidence,
            threshold_used: explanation.threshold,
            matched_pattern: explanation.pattern,
            context_window: explanation.context,
            similar_examples: explanation.examples,
            model_reasoning: explanation.reasoning,
        })
    }

    // ─────────────────────────────────────────────────────────────
    // 4. MODEL_CARD - AI Act Art. 11
    // ─────────────────────────────────────────────────────────────
    #[tool(description = "Return model card for Fastino GLiNER2-Guardrails-PII-Multi. AI Act Art. 11.")]
    async fn pii_model_card(&self, _: ()) -> Result<ModelCard, McpError> {
        Ok(self.compliance.model_card().await?)
    }

    // ─────────────────────────────────────────────────────────────
    // 5. HUMAN_REVIEW - AI Act Art. 14
    // ─────────────────────────────────────────────────────────────
    #[tool(description = "Submit low-confidence entities for human review. AI Act Art. 14.")]
    async fn pii_human_review(&self, args: HumanReviewArgs) -> Result<ReviewQueueItem, McpError> {
        let item = self.review_queue.submit(ReviewRequest {
            entity: args.entity,
            reviewer: args.reviewer,
            priority: args.priority.unwrap_or(Priority::Normal),
            reason: args.reason,
            deadline: args.deadline,
        }).await?;
        Ok(item)
    }

    // ─────────────────────────────────────────────────────────────
    // 6. INCIDENT_REPORT - DORA Art. 11
    // ─────────────────────────────────────────────────────────────
    #[tool(description = "Generate DORA-compliant incident report. DORA Art. 11.")]
    async fn pii_incident_report(&self, args: IncidentArgs) -> Result<IncidentReport, McpError> {
        Ok(self.compliance.dora_report(&args).await?)
    }

    // ─────────────────────────────────────────────────────────────
    // 7. LOAD_TEST - DORA Resilience
    // ─────────────────────────────────────────────────────────────
    #[tool(description = "Run resilience test: latency P99, memory pressure, model corruption. DORA resilience.")]
    async fn pii_load_test(&self, args: LoadTestArgs) -> Result<LoadTestReport, McpError> {
        Ok(self.pipeline.load_test(&args).await?)
    }

    // ─────────────────────────────────────────────────────────────
    // 8. COMPLIANCE_REPORT
    // ─────────────────────────────────────────────────────────────
    #[tool(description = "Generate unified GDPR/DORA/AI Act compliance report.")]
    async fn pii_compliance_report(&self, _: ()) -> Result<ComplianceReport, McpError> {
        Ok(self.compliance.full_report().await?)
    }

    // ─────────────────────────────────────────────────────────────
    // 9. CONFIG_VALIDATE
    // ─────────────────────────────────────────────────────────────
    #[tool(description = "Validate config against GDPR/DORA/AI Act.")]
    async fn pii_config_validate(&self, args: ConfigValidateArgs) -> Result<ConfigValidationResult, McpError> {
        Ok(self.pipeline.validate_config(&args.config).await?)
    }
}
```

### Resources

```rust
#[resource_router]
impl PiiMcpServer {
    #[resource(uri = "pii://model-card", name = "Model Card", mime_type = "application/json")]
    async fn resource_model_card(&self) -> Result<ModelCard, McpError> {
        Ok(self.compliance.model_card().await?)
    }

    #[resource(uri = "pii://compliance-template", name = "Compliance Template", mime_type = "text/markdown")]
    async fn resource_compliance_template(&self) -> Result<String, McpError> {
        Ok(include_str!("../templates/compliance-template.md").to_string())
    }

    #[resource(uri = "pii://audit-schema", name = "Audit Log Schema", mime_type = "application/json")]
    async fn resource_audit_schema(&self) -> Result<serde_json::Value, McpError> {
        Ok(schema_for!(AuditEntry))
    }

    #[resource(uri = "pii://risk-assessment", name = "Risk Assessment Template", mime_type = "application/yaml")]
    async fn resource_risk_assessment(&self) -> Result<String, McpError> {
        Ok(include_str!("../templates/risk-assessment.yaml").to_string())
    }

    #[resource(uri = "pii://compliance-checklist", name = "Compliance Checklist", mime_type = "application/json")]
    async fn resource_checklist(&self) -> Result<ComplianceChecklist, McpError> {
        Ok(self.compliance.checklist().await?)
    }

    #[resource(uri = "pii://thresholds", name = "Per-Label Thresholds", mime_type = "text/plain")]
    async fn resource_thresholds(&self) -> Result<String, McpError> {
        Ok(include_str!("../models/thresholds.toml").to_string())
    }

    #[resource(uri = "pii://patterns", name = "Regex Patterns", mime_type = "text/plain")]
    async fn resource_patterns(&self) -> Result<String, McpError> {
        Ok(include_str!("../patterns/patterns.toml").to_string())
    }
}
```

### Prompts

```rust
#[prompt_router]
impl PiiMcpServer {
    #[prompt(name = "pii_legal_document", description = "Analyze legal document for PII with GDPR redaction")]
    async fn prompt_legal_document(&self, args: LegalDocumentArgs) -> Result<Vec<PromptMessage>> {
        Ok(vec![
            PromptMessage::system(r#"
You are a GDPR/DORA/AI Act compliance assistant for legal documents.
Use pii_scan and pii_redact tools. Respect thresholds and regex > model priority.
"#.to_string()),
            PromptMessage::user(format!(r#"
Analyze this legal document:
---
{}
---

Instructions:
1. Full PII scan (42 labels + guardrails)
2. Redaction mode: {mode}
3. Full audit log required
4. Explain entities with confidence < 0.7
5. Final compliance report
"#, args.document, mode = args.redaction_mode.unwrap_or("pseudonymize"))),
        ])
    }

    #[prompt(name = "pii_human_review", description = "Human review guide for low-confidence PII")]
    async fn prompt_human_review(&self, args: HumanReviewPromptArgs) -> Result<Vec<PromptMessage>> {
        Ok(vec![
            PromptMessage::system(r#"
You are a compliance reviewer. Decide: APPROVE | REJECT | MODIFY
Justify your decision.
"#.to_string()),
            PromptMessage::user(format!(r#"
Entity to review:
- Category: {category}
- Text: "{text}"
- Confidence: {confidence}
- Source: {source}
- Context: "...{context}..."

Reason: {reason}
Reviewer: {reviewer}
Deadline: {deadline}

Reply with decision and justification.
"#,
                category = args.entity.category,
                text = args.entity.text,
                confidence = args.entity.confidence,
                source = format!("{:?}", args.entity.source),
                context = args.context,
                reason = args.reason,
                reviewer = args.reviewer,
                deadline = args.deadline.map(|d| d.to_string()).unwrap_or("N/A".into()),
            )),
        ])
    }

    #[prompt(name = "pii_dora_incident", description = "Generate DORA incident report for PII pipeline failure")]
    async fn prompt_dora_incident(&self, args: DoraIncidentPromptArgs) -> Result<Vec<PromptMessage>> {
        Ok(vec![
            PromptMessage::system(r#"
You are a DORA compliance officer. Generate complete incident report.
Format: JSON per DORA Art. 11.
"#.to_string()),
            PromptMessage::user(format!(r#"
Incident:
- Summary: {summary}
- Severity: {severity}
- Systems: {systems}
- Root cause: {root_cause}
- Actions: {actions}
- Lessons: {lessons}

Generate DORA report.
"#,
                summary = args.summary,
                severity = args.severity,
                systems = args.systems.join(", "),
                root_cause = args.root_cause.unwrap_or("Under investigation".into()),
                actions = args.actions.join("; "),
                lessons = args.lessons.unwrap_or("To be documented".into()),
            )),
        ])
    }
}
```

### main.rs

```rust
use rmcp::{transport::stdio, ServerHandler, ServiceExt};
use tracing_subscriber::{EnvFilter, fmt};

mod tools;
mod resources;
mod prompts;
mod pipeline;

use crate::{tools::PiiMcpServer, pipeline::PiiPipeline};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .with_writer(std::io::stderr)
        .with_ansi(false)
        .init();

    let config = pii_config::PipelineConfig::load(None, Default::default())?;
    let pipeline = Arc::new(PiiPipeline::new(config).await?);
    let compliance = Arc::new(pii_compliance::ComplianceGenerator::new());
    let review_queue = Arc::new(pii_review::ReviewQueue::new());

    let server = PiiMcpServer::new(pipeline, compliance, review_queue).await?;
    let service = server.into_server();
    let (ctxt, server) = server.bind(stdio()).await?;
    
    tracing::info!("xberg-pii-mcp server started on stdio");
    server.await?;
    Ok(())
}
```

---

## 🌐 Task 2: `pii-api` Crate (REST API)

### Files

```text
crates/pii-api/
├── Cargo.toml
├── src/
│   ├── main.rs
│   ├── lib.rs
│   ├── config.rs
│   ├── auth.rs
│   ├── rate_limit.rs
│   ├── routes/
│   │   ├── mod.rs
│   │   ├── pii.rs
│   │   ├── compliance.rs
│   │   ├── review.rs
│   │   ├── audit.rs
│   │   ├── admin.rs
│   │   └── models.rs
│   ├── openapi.rs
│   └── state.rs
└── tests/
    └── api_tests.rs
```

### Cargo.toml

```toml
[package]
name = "pii-api"
version = "0.1.0"
edition = "2021"
description = "REST API for PII detection & redaction"
license = "Apache-2.0"

[dependencies]
axum = { version = "0.7", features = ["json", "cors", "trace"] }
tower = { version = "0.4", features = ["timeout", "limit"] }
tower-http = { version = "0.5", features = ["cors", "trace", "compression"] }
tokio = { workspace = true, features = ["rt-multi-thread", "macros", "fs", "time", "sync"] }
serde = { workspace = true }
serde_json = { workspace = true }
schemars = "0.8"
utoipa = { version = "5", features = ["axum"] }
tracing = { workspace = true }
tracing-subscriber = { version = "0.3", features = ["env-filter", "json"] }
prometheus = "0.13"
jsonwebtoken = "9"
governor = { version = "0.6", features = ["std"] }
pii-pipeline = { path = "../pii-pipeline" }
pii-config = { path = "../pii-config" }
pii-compliance = { path = "../pii-compliance" }
pii-review = { path = "../pii-review" }
pii-audit = { path = "../pii-audit" }
```

### routes/pii.rs

```rust
use axum::{Json, extract::State, response::IntoResponse};
use serde::{Deserialize, Serialize};
use utoipa::JsonSchema;
use crate::state::ApiState;

#[derive(Deserialize, JsonSchema)]
pub struct ScanRequest {
    pub text: String,
    #[serde(default)]
    pub config: Option<ScanConfig>,
}

#[derive(Serialize, JsonSchema)]
pub struct ScanResponse {
    pub entities: Vec<EntityResponse>,
    pub metrics: ScanMetrics,
}

#[utoipa::path(
    post,
    path = "/api/v1/pii/scan",
    request_body = ScanRequest,
    responses(
        (status = 200, description = "Scan successful", body = ScanResponse),
        (status = 400, description = "Invalid request"),
        (status = 401, description = "Unauthorized"),
        (status = 429, description = "Rate limited")
    ),
    tag = "pii"
)]
pub async fn scan_handler(
    State(state): State<Arc<ApiState>>,
    Json(req): Json<ScanRequest>,
) -> impl IntoResponse {
    let config = req.config.unwrap_or_default();
    let entities = state.pipeline.scan(&req.text, &config).await?;
    
    Json(ScanResponse {
        entities: entities.into_iter().map(|e| EntityResponse {
            category: e.category,
            text: req.text[e.start as usize..e.end as usize].to_string(),
            start: e.start,
            end: e.end,
            confidence: e.confidence,
            source: e.source,
        }).collect(),
        metrics: ScanMetrics { regex_ms: 0, model_ms: 0, total_ms: 0 },
    })
}
```

### routes/compliance.rs

```rust
#[utoipa::path(
    get,
    path = "/api/v1/compliance/report",
    params(
        ("format" = Option<String>, Query, description = "html|json|pdf")
    ),
    responses(
        (status = 200, description = "Compliance report", body = ComplianceReport),
    ),
    tag = "compliance"
)]
pub async fn compliance_report_handler(
    State(state): State<Arc<ApiState>>,
    Query(params): Query<ComplianceReportParams>,
) -> impl IntoResponse {
    let report = state.compliance.full_report().await?;
    
    match params.format.as_deref() {
        Some("html") => Html(report.to_html()),
        Some("pdf") => Pdf(report.to_pdf()),
        _ => Json(report),
    }
}

#[utoipa::path(
    get,
    path = "/api/v1/compliance/model-card",
    responses((status = 200, body = ModelCard)),
    tag = "compliance"
)]
pub async fn model_card_handler(State(state): State<Arc<ApiState>>) -> impl IntoResponse {
    Json(state.compliance.model_card().await?)
}
```

### Auth + Rate Limiting

```rust
// auth.rs
use axum::{extract::FromRequestParts, http::StatusCode, response::Response};
use jsonwebtoken::{decode, Validation, Algorithm};

#[derive(Clone)]
pub struct AuthConfig {
    pub jwt_secret: Arc<[u8]>,
    pub jwt_issuer: String,
    pub jwt_audience: String,
    pub api_keys: HashMap<String, ApiKeyInfo>,
}

#[derive(Debug, Clone)]
pub struct Claims {
    pub sub: String,
    pub roles: Vec<Role>,
    pub exp: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Role { Admin, Compliance, Auditor, Reviewer, ApiUser }

pub async fn auth_middleware(
    State(config): State<Arc<AuthConfig>>,
    mut req: Request,
    next: Next,
) -> Result<Response, StatusCode> {
    // 1. Try JWT Bearer
    if let Some(token) = extract_bearer_token(&req) {
        if let Ok(claims) = decode_jwt(&token, &config) {
            req.extensions_mut().insert(claims);
            return Ok(next.run(req).await);
        }
    }
    // 2. Try API Key
    if let Some(api_key) = req.headers().get("X-API-Key") {
        if let Some(key_info) = config.api_keys.get(api_key.to_str().unwrap_or("")) {
            if !key_info.is_expired() {
                req.extensions_mut().insert(Claims {
                    sub: key_info.subject.clone(),
                    roles: key_info.roles.clone(),
                    exp: u64::MAX,
                    iat: 0,
                });
                return Ok(next.run(req).await);
            }
        }
    }
    Err(StatusCode::UNAUTHORIZED)
}

pub fn require_role(role: Role) -> impl Fn(Request, Next) -> Result<Response, StatusCode> + Clone {
    move |mut req: Request, next: Next| {
        let claims = req.extensions().get::<Claims>().ok_or(StatusCode::UNAUTHORIZED)?;
        if !claims.roles.contains(&role) && !claims.roles.contains(&Role::Admin) {
            return Err(StatusCode::FORBIDDEN);
        }
        Ok(next.run(req).await)
    }
}
```

### Rate Limiting

```rust
// rate_limit.rs
use governor::{Quota, RateLimiter};
use std::sync::Arc;

pub fn rate_limit_layer(config: RateLimitConfig) -> impl Layer<Router> + Clone {
    let limiter = Arc::new(RateLimiter::direct(Quota::per_minute(config.rpm)));
    
    tower::ServiceBuilder::new()
        .layer(tower::limit::RateLimitLayer::new(limiter))
        .layer(map_response(|mut resp| {
            resp.headers_mut().insert(
                "X-RateLimit-Limit",
                config.rpm.to_string().parse().unwrap(),
            );
            resp
        }))
}
```

### OpenAPI

```rust
// openapi.rs
use utoipa::OpenApi;

#[derive(OpenApi)]
#[openapi(
    paths(
        // PII
        scan_handler, redact_handler, explain_handler,
        batch_scan_handler, batch_redact_handler,
        // Compliance
        compliance_report_handler, model_card_handler,
        // Review
        list_reviews, create_review, submit_decision,
        // Audit
        query_logs, export_logs, audit_stats,
        // Admin
        get_config, model_status, download_model, benchmark_model,
        rotate_fpe_key, reload_config,
        // Public
        health_check, metrics, openapi_json,
    ),
    components(schemas(
        ScanRequest, ScanConfig, RedactRequest, RedactConfig,
        BatchRedactRequest, ScanEntity, RedactResponse,
        PipelineMetrics, ExplainResult, ModelCard,
        ReviewQueueItem, ReviewDecision, Priority,
        AuditEntry, RedactionAction, EntitySource,
        ComplianceReport, ModelCard, RiskAssessment,
        ConfigValidationResult, ModelStatus, BenchmarkResult,
    )),
    info(
        title = "xberg-pii API",
        version = "1.0.0",
        description = "GDPR/DORA/AI Act compliant PII detection & redaction API",
        contact = Contact { name = "xberg-pii team", email = Some("pii@company.com") },
        license = License { name = "Apache-2.0" }
    ),
    servers(
        (url = "http://localhost:8080/api/v1", description = "Local"),
        (url = "https://api.company.com/api/v1", description = "Production"),
    )
)]
pub struct ApiDoc;

pub fn openapi_json() -> String {
    serde_json::to_string_pretty(&ApiDoc::openapi()).unwrap()
}
```

---

## 🖥️ Task 3: `pii-cli` Crate

### Cargo.toml

```toml
[package]
name = "pii-cli"
version = "0.1.0"
edition = "2021"
description = "CLI for PII detection & redaction"
license = "Apache-2.0"

[[bin]]
name = "pii-cli"
path = "src/main.rs"

[dependencies]
clap = { version = "4", features = ["derive", "env"] }
tokio = { workspace = true, features = ["rt-multi-thread", "macros", "fs", "time"] }
serde = { workspace = true }
serde_json = { workspace = true }
anyhow = { workspace = true }
thiserror = { workspace = true }
pii-pipeline = { path = "../pii-pipeline" }
pii-config = { path = "../pii-config" }
pii-compliance = { path = "../pii-compliance" }
pii-audit = { path = "../pii-audit" }
pii-review = { path = "../pii-review" }
self_update = { version = "0.30", features = ["signatures"] }
```

### main.rs

```rust
use clap::{Parser, Subcommand};
use pii_pipeline::{PiiPipeline, PipelineConfig};

#[derive(Parser)]
#[command(name = "pii-cli", version, about = "PII detection & redaction CLI")]
struct Cli {
    #[arg(long, global = true)]
    config: Option<PathBuf>,
    
    #[arg(long, global = true)]
    log_level: Option<String>,
    
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Scan text for PII
    Scan {
        text: String,
        #[arg(long)]
        threshold: Option<f32>,
        #[arg(long)]
        categories: Option<Vec<String>>,
    },
    /// Redact PII from text/file
    Redact {
        #[arg(short, long)]
        input: Option<PathBuf>,
        #[arg(long)]
        text: Option<String>,
        #[arg(long, default_value = "pseudonymize")]
        mode: RedactionMode,
        #[arg(long)]
        fpe_key: Option<String>,
        #[arg(long)]
        output: Option<PathBuf>,
        #[arg(long)]
        audit_log: Option<PathBuf>,
    },
    /// Batch process multiple files
    Batch {
        paths: Vec<PathBuf>,
        #[arg(long)]
        input: Option<PathBuf>,
        #[arg(long)]
        format: Option<String>,
        #[arg(long)]
        parallel: Option<usize>,
        #[arg(long)]
        output_dir: Option<PathBuf>,
    },
    /// Explain detection for an entity
    Explain {
        text: String,
        entity_index: usize,
    },
    /// Model management
    #[command(subcommand)]
    Model(ModelCommands),
    /// Compliance reports
    #[command(subcommand)]
    Compliance(ComplianceCommands),
    /// Human review queue
    #[command(subcommand)]
    Review(ReviewCommands),
    /// Audit log queries
    #[command(subcommand)]
    Audit(AuditCommands),
    /// Keys management
    #[command(subcommand)]
    Keys(KeysCommands),
    /// Server modes
    #[command(subcommand)]
    Serve(ServeCommands),
    /// Config
    #[command(subcommand)]
    Config(ConfigCommands),
    /// Self-update
    SelfUpdate,
    /// Doctor - health check
    Doctor,
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    // ... dispatch
}
```

---

## 🌐 Task 4: `pii-wasm` Crate

### Cargo.toml

```toml
[package]
name = "pii-wasm"
version = "0.1.0"
edition = "2021"
description = "WASM module for PII detection (Browser + Node.js + WASI)"
license = "Apache-2.0"

[lib]
crate-type = ["cdylib", "rlib"]

[dependencies]
wasm-bindgen = { version = "0.2", features = ["serde-serialize"] }
serde = { workspace = true }
serde_wasm_bindgen = "0.6"
serde_json = { workspace = true }
thiserror = { workspace = true }
pii-pipeline = { path = "../pii-pipeline", default-features = false, features = ["wasm"] }
pii-fastino = { path = "../pii-fastino", features = ["wasm"] }

[target.'cfg(target_arch = "wasm32")'.dependencies]
getrandom = { version = "0.4", features = ["wasm_js"] }
console_error_panic_hook = "0.1"

[features]
default = ["console_error_panic_hook"]
web = ["wasm-bindgen", "web-sys", "js-sys"]
wasi = ["wasm-bindgen", "wasm-bindgen-wasm"]

[profile.release]
opt-level = "s"
lto = true
codegen-units = 1
panic = "abort"
strip = true
```

### lib.rs

```rust
use wasm_bindgen::prelude::*;
use serde::{Deserialize, Serialize};
use pii_pipeline::{Pipeline, PipelineConfig, PipelineResult, Entity, EntitySource};
use pii_fastino::FastinoBackend;

#[wasm_bindgen]
pub struct NerEngine {
    pipeline: Pipeline,
    model_loaded: bool,
}

#[wasm_bindgen]
impl NerEngine {
    #[wasm_bindgen(constructor)]
    pub fn new(config: Option<WasmConfig>) -> Result<NerEngine, WasmError> {
        let config = config.unwrap_or_default().into_pipeline_config()?;
        let pipeline = Pipeline::new(config).map_err(|e| WasmError::Pipeline(e.to_string()))?;
        
        Ok(Self {
            pipeline,
            model_loaded: false,
        })
    }

    #[wasm_bindgen]
    pub async fn load_model(
        &mut self,
        safetensors: Vec<u8>,
        tokenizer: Vec<u8>,
        config: Vec<u8>,
    ) -> Result<(), WasmError> {
        let backend = FastinoBackend::from_bytes(&safetensors, &tokenizer, &config)
            .await.map_err(|e| WasmError::Model(e.to_string()))?;
        
        self.pipeline.set_backend(backend)
            .map_err(|e| WasmError::Pipeline(e.to_string()))?;
        
        self.model_loaded = true;
        Ok(())
    }

    #[wasm_bindgen]
    pub async fn load_lora(
        &mut self,
        adapter_config: Vec<u8>,
        adapter_weights: Vec<u8>,
    ) -> Result<(), WasmError> {
        if !self.model_loaded {
            return Err(WasmError::Model("Base model not loaded".into()));
        }
        self.pipeline.load_lora_bytes(&adapter_config, &adapter_weights)
            .await
            .map_err(|e| WasmError::Model(e.to_string()))
    }

    #[wasm_bindgen]
    pub fn unload_lora(&mut self) -> Result<(), WasmError> {
        self.pipeline.unload_lora().map_err(|e| WasmError::Pipeline(e.to_string()))
    }

    #[wasm_bindgen]
    pub fn scan(&self, text: &str, config: Option<ScanConfig>) -> Result<ScanResult, WasmError> {
        let config = config.unwrap_or_default().into_pipeline_config()?;
        let result = self.pipeline.scan(text, &config)
            .map_err(|e| WasmError::Pipeline(e.to_string()))?;
        
        Ok(ScanResult::from_pipeline_result(result))
    }

    #[wasm_bindgen]
    pub fn redact(&self, text: &str, config: Option<RedactConfig>) -> Result<RedactResult, WasmError> {
        let config = config.unwrap_or_default().into_pipeline_config()?;
        let result = self.pipeline.process(text, &config)
            .map_err(|e| WasmError::Pipeline(e.to_string()))?;
        
        Ok(RedactResult::from_pipeline_result(result))
    }

    #[wasm_bindgen]
    pub fn create_streaming(&self, config: Option<StreamingConfig>) -> Result<StreamingProcessor, WasmError> {
        let config = config.unwrap_or_default().into_pipeline_config()?;
        Ok(StreamingProcessor::new(self.pipeline.clone(), config))
    }

    #[wasm_bindgen(getter)]
    pub fn model_loaded(&self) -> bool {
        self.model_loaded
    }
}
```

---

## 🧪 Task 5: Tests + Distribution

### MCP Contract Tests

```rust
// tests/mcp_contract_tests.rs
use rmcp::{transport::stdio, ServerHandler};

#[tokio::test]
async fn mcp_scan_tool() {
    let server = create_test_server().await;
    let result = server.call_tool("pii_scan", json!({
        "text": "Contact: alice@example.com",
        "config": {}
    })).await.unwrap();
    
    let entities = result.get("entities").unwrap().as_array().unwrap();
    assert_eq!(entities.len(), 1);
    assert_eq!(entities[0]["category"], "email");
}

#[tokio::test]
async fn mcp_redact_tool() {
    let server = create_test_server().await;
    let result = server.call_tool("pii_redact", json!({
        "text": "Contact: alice@example.com",
        "config": { "redaction_mode": "pseudonymize" }
    })).await.unwrap();
    
    assert!(result.get("redacted_text").unwrap().as_str().unwrap().contains("[EMAIL:"));
    assert!(result.get("audit_log").is_some());
}
```

### Distribution Configs

#### Docker Multi-arch

```dockerfile
# Dockerfile (already shown in plan 01)
```

#### Helm Chart

```yaml
# helm/xberg-pii/values.yaml
replicaCount: 3
image:
  repository: ghcr.io/your-org/xberg-pii
  tag: latest
resources:
  limits:
    cpu: 2000m
    memory: 4Gi
config:
  production: |
    [pii_pipeline]
    model_threshold_default = 0.5
    [pii_pipeline.model]
    dtype = "f16"
```

#### NPM Package

```json
{
  "name": "@xberg/pii-wasm",
  "version": "0.1.0",
  "main": "pii_wasm.js",
  "module": "pii_wasm.js",
  "types": "pii_wasm.d.ts",
  "sideEffects": false,
  "files": ["pii_wasm.js", "pii_wasm.d.ts", "pii_wasm_bg.wasm"]
}
```

---

## 📅 Timeline

| Week | Focus |
|------|-------|
| 1 | pii-mcp-server (tools + resources + prompts) |
| 2 | pii-api (Axum + OpenAPI + Auth) |
| 3 | pii-cli (all commands) |
| 4 | pii-wasm (browser + node + wasi) |
| 5 | Integration tests + distribution |

---

## ✅ Acceptance Criteria (v0.2.0)

| Criterion | Target |
|---|---|
| MCP server connects to Claude Desktop | ✅ |
| REST API OpenAPI spec valid | ✅ |
| All CLI commands work | ✅ |
| WASM loads in browser + Node.js | ✅ |
| MCP contract tests pass | ✅ |
| API auth + rate limiting works | ✅ |
| Docker multi-arch builds | ✅ |
| Helm chart deploys | ✅ |
| NPM package publishes | ✅ |

---

**Plan Status:** Ready for execution  
**Next Plan:** `04-facade-integration.md` (xberg-facade + xberg integration)
