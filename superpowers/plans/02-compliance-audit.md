# Implementation Plan: Compliance & Audit Crates

**Spec Reference:** `docs/superpowers/specs/2025-07-24-xberg-pii-ecosystem-design.md`  
**Plan Version:** 1.0  
**Target Crates:** `pii-compliance`, `pii-audit`, `pii-review`  
**Priority:** Should Have (v0.2.0)

---

## 📋 Plan Overview

| Task | Description | Est. Hours | Dependencies |
|------|-------------|------------|--------------|
| 1 | `pii-audit` crate (hash chain + sinks) | 6 | Core pipeline |
| 2 | `pii-compliance` crate (generators) | 8 | Core pipeline |
| 3 | `pii-review` crate (human review queue) | 5 | Core pipeline |
| 4 | Integration tests + compliance validation | 4 | Tasks 1-3 |
| 10 | Documentation + runbooks | 2 | Tasks 1-3 |

**Total Estimated:** ~25 hours

---

## 🛡️ Task 1: `pii-audit` Crate

### Files
```
crates/pii-audit/
├── Cargo.toml
├── src/
│   ├── lib.rs
│   ├── entry.rs
│   ├── chain.rs
│   ├── sink.rs
│   ├── export.rs
│   └── error.rs
└── tests/
    ├── chain_tests.rs
    ├── sink_tests.rs
    └── export_tests.rs
```

### Cargo.toml
```toml
[package]
name = "pii-audit"
version = "0.1.0"
edition = "2021"
description = "Audit logging with integrity chain for PII operations"
license = "Apache-2.0"

[dependencies]
blake3 = { workspace = true }
serde = { workspace = true }
serde_json = { workspace = true }
thiserror = { workspace = true }
tokio = { workspace = true, features = ["fs", "sync"] }
async-trait = "0.1"
chrono = { version = "0.4", features = ["serde"] }
```

### lib.rs
```rust
use blake3;
use serde::{Deserialize, Serialize};
use async_trait::async_trait;
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditEntry {
    pub id: u64,
    pub timestamp: u64,
    pub category: String,
    pub action: RedactionAction,
    pub span_hash: String,
    pub span_length: u32,
    pub confidence: Option<f32>,
    pub source: EntitySource,
    pub pipeline_version: String,
    pub config_hash: String,
    pub chain_hash: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RedactionAction {
    Mask,
    Hash,
    Pseudonymize,
    Remove,
    Custom,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EntitySource {
    Regex,
    Model,
}

impl AuditEntry {
    pub fn new(
        category: String,
        action: RedactionAction,
        span_bytes: &[u8],
        span_length: u32,
        confidence: Option<f32>,
        source: EntitySource,
        config_hash: &str,
        prev_chain_hash: &str,
        seq: u64,
    ) -> Self {
        let span_hash = blake3::hash(span_bytes).to_hex().to_string();
        
        let mut hasher = blake3::Hasher::new();
        hasher.update(prev_chain_hash.as_bytes());
        hasher.update(&Self::bytes_without_chain(
            seq, category, action, span_hash, span_length, 
            confidence, source, env!("CARGO_PKG_VERSION"), config_hash
        ));
        let chain_hash = hasher.finalize().to_hex().to_string();
        
        Self {
            id: seq,
            timestamp: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH).unwrap().as_secs(),
            category,
            action,
            span_hash,
            span_length,
            confidence,
            source,
            pipeline_version: env!("CARGO_PKG_VERSION").into(),
            config_hash: config_hash.into(),
            chain_hash,
        }
    }
    
    fn bytes_without_chain(...) -> Vec<u8> { ... }
}

pub struct AuditChain {
    entries: Vec<AuditEntry>,
    last_chain_hash: String,
    seq: u64,
    config_hash: String,
}

impl AuditChain {
    pub fn new(config_hash: String) -> Self {
        Self {
            entries: Vec::new(),
            last_chain_hash: "0".repeat(64),
            seq: 0,
            config_hash,
        }
    }
    
    pub fn append(&mut self, entry: AuditEntry) -> Result<()> {
        if entry.chain_hash != self.compute_chain_hash(&entry) {
            bail!("Chain hash mismatch at seq {}", entry.id);
        }
        if entry.id != self.seq + 1 {
            bail!("Sequence gap: expected {}, got {}", self.seq + 1, entry.id);
        }
        self.last_chain_hash = entry.chain_hash.clone();
        self.seq = entry.id;
        self.entries.push(entry);
        Ok(())
    }
    
    pub fn verify(&self) -> Result<()> { ... }
}
```

### Sink Trait
```rust
#[async_trait]
pub trait AuditSink: Send + Sync {
    async fn write(&mut self, entry: &AuditEntry) -> Result<()>;
    async fn flush(&mut self) -> Result<()>;
    async fn rotate(&mut self) -> Result<()>;
}

pub struct FileSink {
    writer: BufWriter<File>,
    path: PathBuf,
    current_size: u64,
    max_size: u64,
    max_files: usize,
    chain: AuditChain,
    signing_key: Option<[u8; 32]>,
}
```

### Export Formats
```rust
pub enum ExportFormat {
    JsonLines,
    Json,
    Csv,
    Parquet,
    DoraJson,
}

pub async fn export_audit(
    sink: &FileSink,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
    format: ExportFormat,
    filter: ExportFilter,
) -> Result<Vec<u8>> { ... }
```

---

## ⚖️ Task 2: `pii-compliance` Crate

### DPIA Generator
```rust
// pii-compliance/src/dpia.rs

pub struct DpiaGenerator {
    pipeline_config: PipelineConfig,
    model_card: ModelCard,
    risk_assessment: RiskAssessment,
}

impl DpiaGenerator {
    pub fn generate(&self) -> DpiaDocument {
        DpiaDocument {
            processing_description: self.describe_processing(),
            necessity_proportionality: self.assess_necessity(),
            risks: self.identify_risks(),
            mitigation_measures: self.list_mitigations(),
            dpo_opinion: self.dpo_opinion_template(),
            annexes: vec![
                Annex::ModelCard(self.model_card.clone()),
                Annex::RiskAssessment(self.risk_assessment.clone()),
                Annex::DataFlowDiagram(self.generate_data_flow()),
                Annex::SecurityMeasures(self.list_security_measures()),
            ],
        }
    }
    
    fn identify_risks(&self) -> Vec<Risk> {
        vec![
            Risk {
                id: "R-001",
                description: "Faux positifs sur noms propres → redaction excessive",
                likelihood: Likelihood::Medium,
                severity: Severity::High,
                mitigation: "Seuils par label + human review queue pour confidence < 0.6",
                residual_risk: ResidualRisk::Low,
            },
            Risk {
                id: "R-002",
                description: "Faux négatifs sur PII complexes (adresses étrangères)",
                likelihood: Likelihood::High,
                severity: Severity::Critical,
                mitigation: "Regex étendus + seuils bas par label + human review",
                residual_risk: ResidualRisk::Medium,
            },
            Risk {
                id: "R-003",
                description: "Fuite PII dans logs d'audit",
                likelihood: Likelihood::Low,
                severity: Severity::Critical,
                mitigation: "Audit log = span_hash (BLAKE3) uniquement, ZERO PII, tests unitaires",
                residual_risk: ResidualRisk::Negligible,
            },
            Risk {
                id: "R-004",
                description: "Biais modèle sur noms non-occidentaux",
                likelihood: Likelihood::Medium,
                severity: Severity::High,
                mitigation: "Modèle multilingue Fastino + seuils adaptatifs + human review",
                residual_risk: ResidualRisk::Medium,
            },
        ]
    }
}
```

### Model Card Generator (AI Act Art. 11)
```rust
pub fn generate_model_card(config: &PipelineConfig) -> ModelCard {
    ModelCard {
        model_details: ModelDetails {
            name: "Fastino GLiNER2-Guardrails-PII-Multi".into(),
            version: "1.0.0".into(),
            architecture: "GLiNER2 (DeBERTa-v2 encoder, 300M params)".into(),
            framework: "PyTorch → Safetensors → Candle".into(),
            license: "Apache-2.0".into(),
            repository: "https://github.com/fastino-ai/GLiNER2".into(),
            paper: "arXiv:2605.09973 (GLiNER2-PII), arXiv:2605.07982 (GLiGuard)".into(),
        },
        training_data: TrainingData {
            datasets: vec![
                "GLiGuard (WildGuardTrain + synthetic harm/jailbreak)".into(),
                "GLiNER2-PII (4,910 synthetic multilingual texts, 42 PII types)".into(),
            ],
            languages: vec!["en", "fr", "es", "de", "it", "pt", "nl"],
            synthetic: true,
            pii_categories: 42,
            privacy_risk: "Synthetic data only — no real PII used in training".into(),
        },
        evaluation: Evaluation {
            benchmark: "SPY (Synthetic PII Yesterday)".into(),
            metrics: hashmap! {
                "legal_domain_f1" => 0.475,
                "medical_domain_f1" => 0.467,
                "avg_f1" => 0.471,
                "vs_openai_privacy_filter" => "5x label coverage, better F1",
            },
            limitations: vec![
                "MAX_WIDTH=8 tokens — entités longues non détectées".into(),
                "Performance inférieure sur langues bas-resource".into(),
                "Faux positifs sur patterns similaires (ex: numéros de commande vs IBAN)".into(),
            ],
        },
        bias_fairness: BiasFairness {
            known_biases: vec![
                "Meilleure performance sur noms occidentaux".into(),
                "Adresses non-standard moins bien détectées".into(),
            ],
            mitigation: "Human review queue pour confidence < 0.6, seuils adaptatifs par label".into(),
        },
        deployment: Deployment {
            hardware: "CPU (Intel/AMD ARM64), 4GB RAM minimum".into(),
            software: "Rust 1.75+, Candle 0.11, WASM 32-bit".into(),
            latency_p99_ms: 500,
            memory_mb: 2048,
            fallback: "Regex-only mode if model unavailable".into(),
        },
        governance: Governance {
            risk_classification: "High-risk (Art. 6 AI Act) — used for financial document processing".into(),
            human_oversight: "Human review queue for confidence < 0.6, CLI/API for review decisions".into(),
            transparency: "MCP tool `pii_explain`, tool `pii_model_card`, audit logs".into(),
            monitoring: "Prometheus metrics, audit log integrity verification, drift detection".into(),
            incident_response: "DORA-compliant incident reporting via `pii_incident_report`".into(),
        },
    }
}
```

### DORA Incident Reporter
```rust
pub struct DoraIncidentReporter;

impl DoraIncidentReporter {
    pub fn generate_report(&self, incident: &PiiIncident) -> DoraReport {
        DoraReport {
            reference: format!("DORA-PIL-{}", uuid::Uuid::new_v4()),
            timestamp: Utc::now(),
            entity: self.entity_info(),
            classification: IncidentClassification {
                severity: self.classify_severity(incident),
                category: IncidentCategory::IctSystemFailure,
                impact: self.assess_impact(incident),
            },
            description: IncidentDescription {
                summary: incident.summary.clone(),
                timeline: incident.timeline.clone(),
                root_cause: incident.root_cause.clone(),
                affected_systems: vec!["xberg-pii-pipeline".into()],
                affected_data: self.classify_affected_data(incident),
            },
            response: IncidentResponse {
                detection_time: incident.detected_at,
                containment_time: incident.contained_at,
                recovery_time: incident.resolved_at,
                actions_taken: incident.actions_taken.clone(),
                lessons_learned: incident.lessons_learned.clone(),
            },
            communication: CommunicationLog {
                internal: incident.internal_comms.clone(),
                regulatory: vec![RegulatoryNotification {
                    authority: "ACPR/AMF".into(),
                    sent_at: incident.regulatory_notified_at,
                    reference: incident.regulatory_ref.clone(),
                }],
                customers: incident.customer_notifications.clone(),
            },
        }
    }
}
```

---

## 👥 Task 3: `pii-review` Crate

### Review Queue
```rust
// pii-review/src/lib.rs

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReviewQueueItem {
    pub id: String,
    pub entity: ScanEntity,
    pub status: ReviewStatus,
    pub priority: Priority,
    pub assigned_reviewer: Option<String>,
    pub created_at: DateTime<Utc>,
    pub deadline: Option<DateTime<Utc>>,
    pub decision: Option<ReviewDecision>,
    pub decided_by: Option<String>,
    pub decided_at: Option<DateTime<Utc>>,
    pub comment: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewStatus {
    Pending,
    InReview,
    Approved,
    Rejected,
    Modified,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Priority {
    Low,
    Normal,
    High,
    Critical,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewDecision {
    Approve,
    Reject,
    Modify,
}

pub struct ReviewQueue {
    storage: Arc<dyn ReviewStorage>,
    notifier: Arc<dyn Notifier>,
}

impl ReviewQueue {
    pub async fn submit(&self, request: ReviewRequest) -> Result<ReviewQueueItem> { ... }
    pub async fn decide(&self, id: &str, decision: ReviewDecision, reviewer: &str, comment: String) -> Result<ReviewQueueItem> { ... }
    pub async fn list(&self, filter: ReviewFilter) -> Result<Vec<ReviewQueueItem>> { ... }
    pub async fn stats(&self) -> Result<QueueStats> { ... }
}
```

---

## 🧪 Task 4: Tests + Compliance Validation

### Integration Tests
```rust
// tests/compliance_tests.rs

#[test]
fn audit_log_contains_zero_pii() {
    let result = redact_pii(
        "Email: secret@company.com, Tel: +33 6 12 34 56 78",
        RedactionMode::Mask
    );
    let audit_json = serde_json::to_string(&result.audit_log).unwrap();
    
    assert!(!audit_json.contains("secret"));
    assert!(!audit_json.contains("company.com"));
    assert!(!audit_json.contains("06 12 34 56 78"));
    assert!(audit_json.contains("span_hash"));
    assert!(audit_json.contains("category"));
}

#[test]
fn deterministic_output_same_input() {
    let text = "Contact: bob@test.fr";
    let r1 = redact_pii(text, RedactionMode::Hash);
    let r2 = redact_pii(text, RedactionMode::Hash);
    assert_eq!(r1.text, r2.text);
    assert_eq!(r1.audit_log, r2.audit_log);
}

#[test]
fn fpe_preserves_iban_format_and_checksum() {
    let iban = "FR76 3000 6000 0112 3456 7890 189";
    let fpe_key = [0u8; 32];
    let encrypted = fpe_encrypt(iban, &fpe_key, true);
    
    assert_eq!(encrypted.len(), iban.len());
    assert_eq!(encrypted.matches(' ').count(), iban.matches(' ').count());
    assert!(iban_checksum_valid(&encrypted));
}

#[test]
fn audit_log_integrity_chain() {
    let entries = vec![
        AuditEntry { ... },
        AuditEntry { ... },
    ];
    let chain = AuditChain::new(entries);
    assert!(chain.verify().is_ok());
    
    let mut tampered = entries.clone();
    tampered[0].span_hash = "tampered".into();
    let bad_chain = AuditChain::new(tampered);
    assert!(bad_chain.verify().is_err());
}
```

### Compliance Tests
```rust
#[test]
fn model_card_contains_all_required_sections() {
    let card = generate_model_card(&default_config());
    
    assert!(card.model_details.name.contains("GLiNER2"));
    assert!(!card.training_data.datasets.is_empty());
    assert!(card.evaluation.metrics.contains_key("avg_f1"));
    assert!(!card.bias_fairness.known_biases.is_empty());
    assert!(!card.deployment.limitations.is_empty());
    assert!(card.governance.risk_classification.contains("High-risk"));
    assert!(card.governance.human_oversight.contains("review queue"));
}

#[test]
fn risk_assessment_covers_all_ai_act_annex_iii() {
    let assessment = generate_risk_assessment(&default_config());
    
    assert!(assessment.covers_category("biometric"));
    assert!(assessment.covers_category("critical_infrastructure"));
    assert!(assessment.covers_category("employment"));
    assert!(assessment.covers_category("credit_scoring"));
    assert!(assessment.covers_category("law_enforcement"));
}

#[test]
fn incident_report_contains_all_dora_fields() {
    let incident = PiiIncident { ... };
    let report = DoraIncidentReporter::new().generate_report(&incident);
    
    assert!(!report.reference.is_empty());
    assert!(matches!(report.classification.category, IncidentCategory::IctSystemFailure));
    assert!(!report.description.timeline.is_empty());
    assert!(!report.response.actions_taken.is_empty());
    assert!(!report.communication.regulatory.is_empty());
}
```

---

## 📝 Task 5: Documentation + Runbooks

### Runbooks
```markdown
# docs/runbooks/model-oom.md

## Model Inference OOM

### Symptoms
- Pipeline latency spikes > 5s
- Memory usage > 3.5GB
- Container killed (OOMKilled)

### Diagnosis
1. Check `pii_model_inference_duration_seconds` histogram
2. Check memory usage via `pii_memory_bytes` gauge
3. Verify model dtype (should be F16)

### Resolution
1. Reduce batch size to 1
2. Switch to F16 if not already
3. Enable streaming for large inputs
4. Increase container memory limit

### Prevention
- Set memory limit in container: `--memory=4g`
- Monitor `pii_model_inference_duration_seconds_bucket{le="0.5"}`
- Alert if P99 > 500ms

---
```

---

## 📦 Delivery Checklist

- [ ] `pii-audit` crate publishes to crates.io
- [ ] `pii-compliance` crate publishes to crates.io
- [ ] `pii-review` crate publishes to crates.io
- [ ] All tests pass: `cargo test --package pii-audit --package pii-compliance --package pii-review`
- [ ] Compliance tests pass in CI
- [ ] ADR documents committed
- [ ] Runbooks committed
- [ ] ADR-007: Audit chain design
- [ ] ADR-008: Compliance generator architecture

---

## 📅 Timeline

| Week | Focus |
|---|---|
| 1 | pii-audit (chain + sinks) |
| 2 | pii-compliance (DPIA, Model Card, DORA) |
| 3 | pii-review (queue + notifications) |
| 4 | Tests + integration + docs |

---

**Next Plan:** `03-interfaces.md` (MCP Server + REST API + CLI + WASM)