# Phase 3 Implementation: Observability (Weeks 7-8)

> **Goal**: Full visibility into system health and performance
> **Duration**: 2 weeks (10 working days)
> **Team**: 1 Platform Engineer + 1 Backend Engineer
> **Prerequisites**: Phase 1 complete

---

## Week 7: Distributed Tracing & Structured Logging

### Day 31-32: OpenTelemetry Integration

| Task | Owner | Acceptance Criteria |
|------|-------|---------------------|
| Add opentelemetry crate to hacienda-api | Backend | Cargo.toml updated |
| Configure OTLP exporter (Jaeger/Tempo) | Platform | Traces visible in backend |
| Add trace context propagation | Backend | W3C TraceContext headers |
| Instrument HTTP handlers | Backend | All routes have span |
| Instrument DB queries | Backend | sqlx spans with query text |
| Instrument Redis operations | Backend | Redis spans |
| Instrument external calls (S3, xberg) | Backend | Client spans |
| Set sampling: 100% errors, 10% success | Platform | Cost-controlled |

#### OTel Setup

```rust
// hacienda-api/src/tracing.rs
use opentelemetry::{global, KeyValue};
use opentelemetry_otlp::WithExportConfig;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

pub fn init_tracing() -> Result<()> {
    let tracer = opentelemetry_otlp::new_pipeline()
        .tracing()
        .with_exporter(opentelemetry_otlp::new_exporter()
            .tonic()
            .with_endpoint("http://tempo:4317"))
        .with_trace_config(
            opentelemetry::sdk::trace::config()
                .with_resource(Resource::new(vec![
                    KeyValue::new("service.name", "hacienda-api"),
                    KeyValue::new("deployment.environment", env::var("ENV").unwrap_or("dev".into())),
                ]))
                .with_sampler(Sampler::TraceIdRatioBased(0.1)) // 10% + errors
        )
        .install_batch(opentelemetry::runtime::Tokio)?;

    let otel_layer = tracing_opentelemetry::layer().with_tracer(tracer);
    let fmt_layer = tracing_subscriber::fmt::layer()
        .json()
        .with_current_span(true)
        .with_span_list(true);

    tracing_subscriber::registry()
        .with(tracing_subscriber::EnvFilter::from_default_env())
        .with(otel_layer)
        .with(fmt_layer)
        .init();

    Ok(())
}
```

### Day 33-34: Structured JSON Logging

| Task | Owner | Acceptance Criteria |
|------|-------|---------------------|
| Configure tracing-subscriber JSON format | Backend | All logs JSON with fields |
| Add trace_id, span_id, tenant_id fields | Backend | Auto-included via OTel |
| Add structured error logging | Backend | error.chain() serialized |
| Configure log levels per environment | Platform | dev=debug, staging=info, prod=info |
| Add Loki log aggregation | Platform | Logs queryable in Grafana |

#### JSON Log Format

```json
{
  "timestamp": "2026-08-25T10:30:00.123Z",
  "level": "INFO",
  "message": "Document processed",
  "trace_id": "abc123",
  "span_id": "def456",
  "tenant_id": "tenant_xyz",
  "fields": {
    "document_id": "doc_789",
    "processing_time_ms": 1250,
    "pii_detected": 5
  }
}
```

### Day 35: SLO/SLI Dashboard Creation

| Task | Owner | Acceptance Criteria |
|------|-------|---------------------|
| Create Prometheus recording rules | Platform | SLI metrics pre-computed |
| Build Grafana dashboard (overview) | Platform | CPU, memory, latency, errors |
| Build Grafana dashboard (business) | Platform | Docs processed, PII recall, quota |
| Build Grafana dashboard (audit) | Platform | Chain health, segment lag |
| Build Grafana dashboard (RAG) | Platform | Query latency, index size |
| Provision dashboards via ConfigMap | Platform | Auto-loaded on Grafana start |

---

## Week 8: Alerting, Health Probes, Chaos

### Day 36: Alerting Rules

| Task | Owner | Acceptance Criteria |
|------|-------|---------------------|
| Define PrometheusRule for all SLIs | Platform | deploy/monitoring/prometheus-rules.yaml |
| Configure Alertmanager routes | Platform | Severity -> PagerDuty/Slack/Email |
| Add inhibition rules | Platform | Suppress redundant alerts |
| Add runbook URLs to annotations | Platform | Each alert links to runbook |
| Test alert firing & resolution | Platform | Alert test passes |

#### Key Alerts

| Alert | Expression | Severity | Runbook |
|-------|------------|----------|---------|
| HaciendaAPIDown | up==0 | critical | api-down.md |
| HighLatency | histogram_quantile(0.95, ...) > 1s | warning | capacity-exhaustion.md |
| HighErrorRate | rate(5xx) > 1% | critical | api-down.md |
| PostgresReplicationLag | pg_replication_lag > 30s | warning | db-failover.md |
| AuditChainVerifyFailed | increase(failures) > 0 | critical | audit-chain-corruption.md |
| PseudonymKeyAnomaly | vault reads > 10/5m | critical | pseudonym-key-compromise.md |
| HPAAtMax | desired == max_replicas | warning | capacity-exhaustion.md |
| JobQueueLag | hacienda_job_queue_lag > 300s | warning | dependency-outage.md |

### Day 37: Health/Readiness Probes

| Task | Owner | Acceptance Criteria |
|------|-------|---------------------|
| Implement /health (liveness) | Backend | Returns 200 if process alive |
| Implement /ready (readiness) | Backend | Checks PG, Redis, S3 |
| Configure K8s probes | Platform | livenessProbe, readinessProbe, startupProbe |
| Test pod removal on dependency failure | Platform | Traffic shifts to healthy pods |

### Day 38: Chaos Engineering Baseline

| Task | Owner | Acceptance Criteria |
|------|-------|---------------------|
| Install Litmus/Chaos Mesh | Platform | CRDs available |
| Create pod kill experiment | Platform | Random pod termination |
| Create network partition experiment | Platform | Pod-to-PG latency/partition |
| Create DB failover experiment | Platform | Primary down, verify failover |
| Run experiments in staging | Platform | All pass, document results |
| Document findings & improvements | Platform | Update runbooks, HPA tuning |

### Day 39: Capacity Planning Model

| Task | Owner | Acceptance Criteria |
|------|-------|---------------------|
| Document requests/pod baseline | Platform | From load test data |
| Document DB connection scaling | Platform | Pool size vs latency |
| Document RAG index growth | Platform | Vectors per GB |
| Document audit chain growth | Platform | Entries per GB per month |
| Create scaling trigger doc | Platform | When to add nodes, increase PG |

### Day 40: Validation

```bash
# 1. Verify traces in Jaeger/Tempo
# 2. Verify logs in Loki/Grafana
# 3. Verify dashboards show data
# 4. Fire test alerts
# 4. Run chaos experiments
# 5. All gate criteria pass
✅ Distributed tracing working (100% errors sampled)
✅ JSON structured logging with trace_id/tenant_id
✅ SLO dashboards operational
✅ Alerting rules firing correctly
✅ Health/readiness probes working
✅ Chaos experiments documented
✅ Capacity planning model documented