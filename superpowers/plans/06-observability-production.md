# Implementation Plan: Observability & Production Hardening

**Spec Reference:** `docs/superpowers/specs/2025-07-24-xberg-pii-ecosystem-design.md`  
**Plan Version:** 1.0  
**Target:** `pii-observability` crate + production configs  
**Priority:** Should Have (v0.2.0)

---

## 📋 Plan Overview

| Task | Description | Est. Hours | Dependencies |
|------|-------------|------------|--------------|
| 1 | `pii-observability` crate (tracing, metrics, health) | 6 | Core pipeline |
| 2 | Prometheus metrics + alerting rules | 3 | Task 1 |
| 3 | OpenTelemetry integration | 4 | Task 1 |
| 4 | Health checks + readiness | 2 | Task 1 |
| 5 | Chaos testing + resilience | 4 | Core pipeline |
| 6 | Runbooks + tuning guide | 3 | Tasks 1-5 |
| 7 | Production configs (Docker, K8s, systemd) | 3 | Tasks 1-6 |

**Total Estimated:** ~25 hours

---

## 📊 Task 1: `pii-observability` Crate

### Files
```
crates/pii-observability/
├── Cargo.toml
├── src/
│   ├── lib.rs
│   ├── metrics.rs
│   ├── tracing.rs
│   ├── health.rs
│   └── error.rs
└── tests/
    └── observability_tests.rs
```

### Cargo.toml
```toml
[package]
name = "pii-observability"
version = "0.1.0"
edition = "2021"
description = "Observability for PII pipeline: metrics, tracing, health"
license = "Apache-2.0"

[dependencies]
prometheus = { version = "0.13", features = ["process"] }
opentelemetry = { version = "0.23", features = ["rt-tokio", "trace", "metrics"] }
opentelemetry-otlp = { version = "0.18", features = ["tonic"] }
opentelemetry-prometheus = "0.20"
opentelemetry-semantic-conventions = { version = "0.23", features = ["metrics"] }
tracing = { workspace = true }
tracing-subscriber = { version = "0.3", features = ["env-filter", "json", "fmt"] }
tracing-opentelemetry = { version = "0.26", features = ["metrics"] }
prometheus-client = "0.14"
axum = { version = "0.7", features = [] }
tokio = { workspace = true, features = ["rt-multi-thread", "macros", "time"] }
serde = { workspace = true }
thiserror = { workspace = true }
```

### lib.rs - Metrics
```rust
use prometheus::{Counter, CounterVec, Histogram, HistogramVec, Gauge, GaugeVec, IntGauge, register_counter, register_counter_vec, register_histogram, register_histogram_vec, register_gauge, register_gauge_vec, register_int_gauge};
use lazy_static::lazy_static;

lazy_static! {
    // Pipeline stages
    pub static ref PII_PIPELINE_DURATION: HistogramVec = register_histogram_vec!(
        "pii_pipeline_duration_seconds",
        "Pipeline stage latency in seconds",
        &["stage"]  // regex, model, merge, redact, audit
    ).unwrap();

    pub static ref PII_ENTITIES_DETECTED: CounterVec = register_counter_vec!(
        "pii_entities_detected_total",
        "Total entities detected",
        &["category", "source"]  // email/regex, person/model
    ).unwrap();

    pub static ref PII_ENTITIES_REDACTED: CounterVec = register_counter_vec!(
        "pii_entities_redacted_total",
        "Total entities redacted",
        &["category", "action"]  // email/mask, iban/pseudonymize
    ).unwrap();

    // Model inference
    pub static ref PII_MODEL_INFERENCE_DURATION: Histogram = register_histogram!(
        "pii_model_inference_duration_seconds",
        "Model inference latency"
    ).unwrap();

    pub static ref PII_MODEL_INFERENCE_ERRORS: Counter = register_counter!(
        "pii_model_inference_errors_total",
        "Model inference errors"
    ).unwrap();

    // Redaction
    pub static ref PII_REDACTION_OPERATIONS: CounterVec = register_counter_vec!(
        "pii_redaction_operations_total",
        "Redaction operations",
        &["mode"]  // mask, hash, pseudonymize, remove
    ).unwrap();

    pub static ref PII_FPE_OPERATIONS: Counter = register_counter!(
        "pii_fpe_operations_total",
        "FPE encryption operations"
    ).unwrap();

    // Audit
    pub static ref PII_AUDIT_WRITES: Counter = register_counter!(
        "pii_audit_writes_total",
        "Audit log write operations"
    ).unwrap();

    pub static ref PII_AUDIT_WRITE_ERRORS: Counter = register_counter!(
        "pii_audit_write_errors_total",
        "Audit log write errors"
    ).unwrap();

    // System
    pub static ref PII_MEMORY_BYTES: Gauge = register_gauge!(
        "pii_memory_bytes",
        "Current memory usage in bytes"
    ).unwrap();

    pub static ref PII_ACTIVE_REQUESTS: Gauge = register_gauge!(
        "pii_active_requests",
        "Currently processing requests"
    ).unwrap();

    pub static ref PII_QUEUE_DEPTH: Gauge = register_gauge!(
        "pii_queue_depth",
        "Pending requests in queue"
    ).unwrap();
}

/// Record pipeline stage duration
pub fn record_stage_duration(stage: &str, duration: std::time::Duration) {
    PII_PIPELINE_DURATION.with_label_values(&[stage]).observe(duration.as_secs_f64());
}

/// Record entity detection
pub fn record_entity_detected(category: &str, source: &str) {
    PII_ENTITIES_DETECTED.with_label_values(&[category, source]).inc();
}

/// Record redaction
pub fn record_redaction(category: &str, action: &str) {
    PII_ENTITIES_REDACTED.with_label_values(&[category, action]).inc();
}
```

### tracing.rs - OpenTelemetry
```rust
use opentelemetry::{global, KeyValue};
use opentelemetry_otlp::WithExportConfig;
use opentelemetry_semantic_conventions as semconv;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

pub fn init_tracing(service_name: &str, otlp_endpoint: Option<String>) -> Result<()> {
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info,pii_pipeline=debug,pii_fastino=debug"));

    let fmt_layer = tracing_subscriber::fmt::layer()
        .json()
        .with_current_span(true)
        .with_span_list(true);

    let otlp_layer = if let Some(endpoint) = otlp_endpoint {
        let tracer = opentelemetry_otlp::new_pipeline()
            .tracing()
            .with_exporter(opentelemetry_otlp::new_exporter()
                .tonic()
                .with_endpoint(endpoint))
            .with_trace_config(
                opentelemetry_sdk::trace::config().with_resource(
                    opentelemetry_sdk::Resource::new(vec![
                        KeyValue::new(semconv::resource::SERVICE_NAME, "xberg-pii"),
                        KeyValue::new(semconv::resource::SERVICE_VERSION, env!("CARGO_PKG_VERSION")),
                    ])
                )
            )
            .install_batch(opentelemetry_sdk::runtime::Tokio)?
            .into();
        Some(tracer)
    } else { None };

    tracing_subscriber::registry()
        .with(filter)
        .with(fmt_layer)
        .with(otlp_layer)
        .init();

    Ok(())
}
```

### health.rs - Health Checks
```rust
use axum::{Json, Router, routing::get};
use serde::Serialize;
use std::sync::Arc;
use std::time::Instant;

#[derive(Serialize)]
pub struct HealthResponse {
    status: String,
    version: String,
    uptime_seconds: u64,
    checks: Vec<HealthCheck>,
}

#[derive(Serialize)]
pub struct HealthCheck {
    name: String,
    status: String,
    latency_ms: u64,
    message: Option<String>,
}

pub struct HealthChecker {
    start_time: Instant,
    pipeline: Arc<pii_pipeline::PiiPipeline>,
}

impl HealthChecker {
    pub fn new(pipeline: Arc<pii_pipeline::PiiPipeline>) -> Self {
        Self { start_time: Instant::now(), pipeline }
    }

    pub async fn check(&self) -> HealthResponse {
        let mut checks = Vec::new();

        // Liveness - always healthy if process alive
        checks.push(HealthCheck {
            name: "liveness".into(),
            status: "healthy".into(),
            latency_ms: 0,
            message: None,
        });

        // Readiness - model loaded, memory OK
        let start = Instant::now();
        let model_ok = self.pipeline.is_model_loaded();
        let memory_ok = self.check_memory();
        let latency = start.elapsed().as_millis() as u64;

        checks.push(HealthCheck {
            name: "readiness".into(),
            status: if model_ok && memory_ok { "healthy" } else { "degraded" }.into(),
            latency_ms: latency,
            message: if model_ok && memory_ok { None } else {
                Some(format!("model_loaded: {}, memory_ok: {}", model_ok, memory_ok))
            },
        });

        // Model inference test
        let start = Instant::now();
        let test_result = self.pipeline.process("test", &Default::default()).await;
        let inference_latency = start.elapsed().as_millis() as u64;

        checks.push(HealthCheck {
            name: "model_inference".into(),
            status: if test_result.is_ok() { "healthy" } else { "unhealthy" }.into(),
            latency_ms: inference_latency,
            message: test_result.err().map(|e| e.to_string()),
        });

        Json(HealthResponse {
            status: if checks.iter().all(|c| c.status == "healthy") { "healthy" } else { "degraded" }.into(),
            version: env!("CARGO_PKG_VERSION").into(),
            uptime_seconds: self.start_time.elapsed().as_secs(),
            checks,
        })
    }

    fn check_memory(&self) -> bool {
        // Check if memory < 80% of limit
        let usage = get_memory_usage_bytes();
        let limit = get_memory_limit_bytes();
        usage < (limit as f64 * 0.8) as u64
    }
}

pub fn health_routes(state: Arc<HealthChecker>) -> Router {
    Router::new()
        .route("/health", get(move || async move { Json(state.check().await) }))
        .route("/ready", get(move || async move { Json(state.check().await) }))
        .route("/live", get(|| async { "OK" }))
}
```

---

## 📈 Task 2: Prometheus Metrics + Alerting Rules

### metrics.rs (additional)
```rust
// Additional metrics for alerting
lazy_static! {
    // SLO metrics
    pub static ref PII_SLO_LATENCY_P99: Histogram = register_histogram!(
        "pii_slo_latency_p99_seconds",
        "P99 latency SLO"
    ).unwrap();

    pub static ref PII_SLO_AVAILABILITY: Gauge = register_gauge!(
        "pii_slo_availability_ratio",
        "Availability ratio (successful requests / total)"
    ).unwrap();

    pub static ref PII_SLO_ERROR_RATE: Gauge = register_gauge!(
        "pii_slo_error_rate_ratio",
        "Error rate ratio"
    ).unwrap();

    // Resource utilization
    pub static ref PII_CPU_USAGE: Gauge = register_gauge!(
        "pii_cpu_usage_ratio",
        "CPU usage ratio"
    ).unwrap();

    pub static ref PII_DISK_USAGE: Gauge = register_gauge!(
        "pii_disk_usage_bytes",
        "Disk usage for audit logs"
    ).unwrap();

    // Model-specific
    pub static ref PII_MODEL_LOADED: Gauge = register_gauge!(
        "pii_model_loaded",
        "Whether model is loaded (1) or not (0)"
    ).unwrap();

    pub static ref PII_MODEL_LOAD_DURATION: Histogram = register_histogram!(
        "pii_model_load_duration_seconds",
        "Model load time"
    ).unwrap();
}
```

### alerting-rules.yml
```yaml
groups:
- name: xberg-pii
  interval: 30s
  rules:
  # Latency
  - alert: PiiPipelineLatencyHigh
    expr: histogram_quantile(0.99, rate(pii_pipeline_duration_seconds_bucket[5m])) > 0.5
    for: 5m
    labels:
      severity: warning
    annotations:
      summary: "PII pipeline P99 latency > 500ms"
      description: "Stage {{ $labels.stage }} P99 is {{ $value }}s"

  - alert: PiiModelInferenceLatencyHigh
    expr: histogram_quantile(0.99, rate(pii_model_inference_duration_seconds_bucket[5m])) > 2.0
    for: 5m
    labels:
      severity: critical
    annotations:
      summary: "Model inference P99 > 2s"

  # Availability
  - alert: PiiAvailabilityLow
    expr: pii_slo_availability_ratio < 0.999
    for: 5m
    labels:
      severity: critical
    annotations:
      summary: "PII availability below 99.9%"

  # Error rate
  - alert: PiiErrorRateHigh
    expr: pii_slo_error_rate_ratio > 0.01
    for: 5m
    labels:
      severity: warning
    annotations:
      summary: "PII error rate > 1%"

  # Memory
  - alert: PiiMemoryHigh
    expr: pii_memory_bytes / pii_memory_limit_bytes > 0.85
    for: 5m
    labels:
      severity: warning
    annotations:
      summary: "PII memory usage > 85%"

  # Audit log
  - alert: PiiAuditLogWriteFailures
    expr: rate(pii_audit_write_errors_total[5m]) > 0
    for: 1m
    labels:
      severity: critical
    annotations:
      summary: "Audit log write failures detected"

  # Model
  - alert: PiiModelNotLoaded
    expr: pii_model_loaded == 0
    for: 1m
    labels:
      severity: critical
    annotations:
      summary: "PII model not loaded"

  - alert: PiiModelLoadSlow
    expr: pii_model_load_duration_seconds > 30
    for: 5m
    labels:
      severity: warning
    annotations:
      summary: "Model load time > 30s"

  # Disk
  - alert: PiiDiskSpaceLow
    expr: pii_disk_usage_bytes / pii_disk_limit_bytes > 0.9
    for: 15m
    labels:
      severity: warning
    annotations:
      summary: "Disk space for audit logs > 90%"

  # Queue
  - alert: PiiQueueBacklog
    expr: pii_queue_depth > 100
    for: 10m
    labels:
      severity: warning
    annotations:
      summary: "PII request queue backlog > 100"
```

---

## 🔧 Task 3: OpenTelemetry Integration

### Config
```toml
# config/production.toml
[observability]
tracing_enabled = true
otlp_endpoint = "http://otel-collector:4317"
metrics_enabled = true
metrics_port = 9090
health_port = 8081
log_level = "info"
log_format = "json"
```

### Integration in main.rs
```rust
// In pii-api/src/main.rs
fn main() -> Result<()> {
    pii_observability::init_tracing("xberg-pii-api", Some("http://otel-collector:4317".into()))?;
    
    let health_checker = Arc::new(HealthChecker::new(pipeline.clone()));
    
    let app = Router::new()
        .merge(pii_routes(state.clone()))
        .merge(compliance_routes(state.clone()))
        .merge(health_routes(health_checker.clone()))
        .route("/metrics", get(metrics_handler))
        .layer(
            ServiceBuilder::new()
                .layer(TraceLayer::new_for_http())
                .layer(CorsLayer::permissive())
                .layer(rate_limit_layer(config.rate_limit.clone()))
                .layer(auth_middleware(config.auth.clone()))
        );
}
```

---

## 🧪 Task 4: Chaos Testing + Resilience

### chaos.rs
```rust
// pii-pipeline/src/chaos.rs
use rand::Rng;

pub struct ChaosConfig {
    pub oom_probability: f64,
    pub latency_injection_ms: Option<u64>,
    pub model_corruption_probability: f64,
    pub network_failure_probability: f64,
}

impl Default for ChaosConfig {
    fn default() -> Self {
        Self {
            oom_probability: 0.0,
            latency_injection_ms: None,
            model_corruption_probability: 0.0,
            network_failure_probability: 0.0,
        }
    }
}

pub struct ChaosPipeline {
    inner: PiiPipeline,
    chaos: ChaosConfig,
}

impl ChaosPipeline {
    pub fn new(pipeline: PiiPipeline, chaos: ChaosConfig) -> Self {
        Self { inner: pipeline, chaos }
    }

    pub async fn process(&self, text: &str, config: &PipelineConfig) -> Result<PipelineResult> {
        // Inject latency
        if let Some(ms) = self.chaos.latency_injection_ms {
            tokio::time::sleep(Duration::from_millis(ms)).await;
        }

        // Inject OOM
        if rand::thread_rng().gen::<f64>() < self.chaos.oom_probability {
            // Allocate large buffer to trigger OOM
            let _big = vec![0u8; 500_000_000]; // 500MB
        }

        // Inject model corruption
        if rand::thread_rng().gen::<f64>() < self.chaos.model_corruption_probability {
            // Return corrupted results
            return Ok(PipelineResult {
                redacted_text: "CHAOS_CORRUPTED".into(),
                entities: vec![],
                audit_log: vec![],
                metrics: PipelineMetrics::default(),
            });
        }

        // Inject network failure
        if rand::thread_rng().gen::<f64>() < self.chaos.network_failure_probability {
            return Err(anyhow::anyhow!("CHAOS: Network failure injected"));
        }

        self.inner.process(text, config).await
    }
}
```

### Load Test
```bash
# Cargo command
cargo bench --package pii-pipeline -- chaos --duration 300 --concurrency 10 --chaos oom,latency,corruption,network

# Expected: Pipeline degrades gracefully, circuit breakers work, no panics
```

---

## 📋 Task 5: Runbooks

### docs/runbooks/model-oom.md
```markdown
# Model Inference OOM

## Symptoms
- Pipeline latency spikes > 5s
- Memory usage > 3.5GB
- Container killed (OOMKilled)

## Diagnosis
1. Check `pii_model_inference_duration_seconds` histogram
2. Check `pii_memory_bytes` gauge
3. Verify model dtype (should be F16)

## Resolution
1. Reduce batch size to 1
2. Switch to F16 if not already
3. Enable streaming for large inputs
4. Increase container memory limit

## Prevention
- Set memory limit: `--memory=4g`
- Monitor: `pii_model_inference_duration_seconds_bucket{le="0.5"}`
- Alert: P99 > 500ms
```

### docs/runbooks/audit-log-corruption.md
```markdown
# Audit Log Corruption

## Symptoms
- `pii_audit_log_verify` fails
- Hash chain verification fails

## Diagnosis
1. Run `pii-audit verify --from 2024-01-01 --to 2024-01-31`
2. Check disk space
3. Check for concurrent writers

## Resolution
1. Rotate log file
2. Restore from backup
3. Investigate concurrent access

## Prevention
- Single writer per log file
- Rotation every 100MB
- Signed rotation markers
```

### docs/runbooks/fpe-key-compromise.md
```markdown
# FPE Key Compromise

## Immediate Actions
1. Rotate FPE key immediately: `pii-cli keys rotate`
2. Re-encrypt all pseudonymized data
3. Audit access logs for key exposure

## Rotation Procedure
1. Generate new key: `openssl rand -base64 32`
2. Set env var: `export XBERG_FPE_KEY=new_key`
3. Restart services
4. Re-process affected documents
```

---

## 📋 Task 6: Production Configs

### systemd service
```ini
# /etc/systemd/system/xberg-pii.service
[Unit]
Description=xberg-pii PII Detection API
After=network.target

[Service]
Type=simple
User=xberg
Group=xberg
WorkingDirectory=/opt/xberg-pii
ExecStart=/opt/xberg-pii/bin/pii-api serve http --config /etc/xberg-pii/production.toml
Restart=on-failure
RestartSec=5
LimitNOFILE=65536
LimitMEMLOCK=infinity
AmbientCapabilities=CAP_NET_BIND_SERVICE

# Security
NoNewPrivileges=true
PrivateTmp=true
ProtectSystem=strict
ProtectHome=true
ReadWritePaths=/var/lib/xberg-pii /var/log/xberg-pii
CapabilityBoundingSet=CAP_NET_BIND_SERVICE

# Resources
MemoryLimit=4G
CPUQuota=200%

[Install]
WantedBy=multi-user.target
```

### Kubernetes Deployment
```yaml
# k8s/deployment.yaml
apiVersion: apps/v1
kind: Deployment
metadata:
  name: xberg-pii
spec:
  replicas: 3
  selector:
    matchLabels:
      app: xberg-pii
  template:
    metadata:
      labels:
        app: xberg-pii
    spec:
      containers:
      - name: api
        image: ghcr.io/your-org/xberg-pii:latest
        ports:
        - containerPort: 8080
        - containerPort: 9090
        env:
        - name: XBERG_FPE_KEY
          valueFrom:
            secretKeyRef:
              name: xberg-pii-secrets
              key: fpe-key
        resources:
          requests:
            memory: "2Gi"
            cpu: "1000m"
          limits:
            memory: "4Gi"
            cpu: "2000m"
        livenessProbe:
          httpGet:
            path: /live
            port: 8081
          initialDelaySeconds: 10
          periodSeconds: 10
        readinessProbe:
          httpGet:
            path: /ready
            port: 8081
          initialDelaySeconds: 5
          periodSeconds: 5
        resources:
          limits:
            memory: "4Gi"
            cpu: "2000m"
      securityContext:
        runAsNonRoot: true
        runAsUser: 1000
        fsGroup: 1000
```

---

## ✅ Acceptance Criteria (v0.2.0)

| Criterion | Target |
|---|---|
| `cargo test --package pii-observability` | ✅ PASS |
| Prometheus metrics exposed on :9090/metrics | ✅ |
| OpenTelemetry traces to OTLP endpoint | ✅ |
| `/health` + `/ready` endpoints work | ✅ |
| P99 latency < 500ms under load | ✅ |
| Memory < 2GB steady state | ✅ |
| Chaos tests pass (no panics) | ✅ |
| Runbooks executable by on-call | ✅ |
| Prometheus alerts firing correctly | ✅ |
| Grafana dashboards importable | ✅ |

---

## 📅 Timeline

| Week | Focus |
|---|---|
| 1 | pii-observability crate + metrics |
| 2 | OpenTelemetry + Health checks |
| 3 | Chaos testing + Alerting rules |
| 4 | Runbooks + Production configs + Testing |

---

**Plan Status:** Ready for execution  
**Next Plan:** `06-xberg-integration.md` (if needed) or move to execution phase