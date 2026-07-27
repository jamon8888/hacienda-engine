# hacienda Configuration Examples

## Basic Configuration (hacienda.toml)

```toml
[extraction]
output_format = "json"

[pii]
enabled = true
redaction_profile = "GDPR"
model = { enabled = true, model_id = "fastino/GLiNER2-Guardrails-PII-Multi" }

[compliance]
enabled = true
model_name = "hacienda-pii-v1"
enabled_reports = ["DPIA", "ModelCard", "DORA", "AIAct", "Checklist"]

[audit]
enabled = true
log_path = "audit.log"
format = "jsonl"

[review]
enabled = true
auto_assign = false
deadline_hours = 24

[glossary]
enabled = true
link_style = "Markdown"
min_confidence = 0.5
```

## Profiles

### Default

Basic PII detection with standard redaction modes.

### PCI-DSS

```toml
[pii]
redaction_profile = "PCI"
```

Detects: Credit card numbers, PANs, CVV, expiration dates

### HIPAA

```toml
[pii]
redaction_profile = "HIPAA"
```

Detects: SSN, MRN, PHI identifiers, dates, phone numbers

### GDPR

```toml
[pii]
redaction_profile = "GDPR"
```

Detects: Email, phone, IP, names, addresses, personal identifiers

### Custom

```toml
[pii]
redaction_profile = "Custom"
custom_patterns = [
    r"INVOICE-\d{6}",
    r"PO-\d{4}-\d{3}"
]
custom_terms = [
    "CONFIDENTIAL",
    "PROPRIETARY"
]
```

## Production Configuration

```toml
[server]
bind_address = "0.0.0.0:8080"
workers = 4
request_body_limit = "50mb"
timeout = 30

[security]
jwt_secret = "${HACIENDA_JWT_SECRET}"
cors_origins = ["https://app.example.com"]
rate_limit_rpm = 100

[audit]
enabled = true
log_path = "/var/log/hacienda/audit.log"
max_size_mb = 100
max_files = 30
rotation = "daily"
hash_algorithm = "blake3"

[models]
cache_dir = "/var/lib/hacienda/models"
auto_download = true

[observability]
metrics_port = 9090
log_level = "info"
log_format = "json"
tracing_enabled = true
```

## Environment Variables

| Variable              | Description                 | Default             |
| --------------------- | --------------------------- | ------------------- |
| `HACIENDA_CONFIG`     | Path to config file         | `./hacienda.toml`   |
| `HACIENDA_JWT_SECRET` | JWT signing secret          | Required            |
| `HACIENDA_FPE_KEY`    | FPE encryption key (base64) | Optional            |
| `RUST_LOG`            | Log level                   | `info`              |
| `HACIENDA_MODELS_DIR` | Model cache directory       | `~/.cache/hacienda` |

## CLI Usage

```bash
# Scan text
hacienda pii scan "Contact john@example.com"

# Redact file
hacienda pii redact input.txt -o redacted.txt

# Batch process
hacienda pii batch input/ -o output/

# Generate compliance report
hacienda compliance report --model hacienda-pii-v1

# Manage review queue
hacienda review list --status pending
hacienda review decide <id> --decision approve --reviewer alice

# Audit operations
hacienda audit query --from 2024-01-01 --to 2024-01-31
hacienda audit export --format csv --from 2024-01-01
hacienda audit verify
```

## Programmatic Usage

### Rust

```rust
use hacienda::{HaciendaFacade, HaciendaFacadeConfig, ExtractInput};

let facade = HaciendaFacade::new(HaciendaFacadeConfig {
    extraction: Default::default(),
    pii: Some(Default::default()),
    compliance: Some(Default::default()),
    audit: Some(Default::default()),
    review: Some(Default::default()),
    glossary: Some(Default::default()),
})?;

let result = facade.process(ExtractInput::from_uri("document.pdf")).await?;
```

### Python

```python
import hacienda

facade = hacienda.HaciendaFacade()
result = facade.process(hacienda.ExtractInput.from_uri("contract.pdf"))
print(result.pii.redacted_text)
```

### REST API

```bash
curl -X POST http://localhost:8080/v1/pii/scan \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer <token>" \
  -d '{"text": "Contact john@example.com"}'

# Response
{
  "entities": [
    {"category": "Email", "start": 8, "end": 24, "confidence": 0.99}
  ],
  "redacted": "Contact [EMAIL:john@example.com]"
}
```

## Docker Compose

```yaml
version: "3.8"

services:
  hacienda:
    image: ghcr.io/jamon8888/hacienda:latest
    ports:
      - "8080:8080"
    environment:
      - HACIENDA_JWT_SECRET=${JWT_SECRET}
      - RUST_LOG=info
    volumes:
      - ./config:/app/config:ro
      - ./data:/app/data
    healthcheck:
      test: ["CMD", "hacienda", "health-check"]
      interval: 30s
      timeout: 5s
      retries: 3
    deploy:
      resources:
        limits:
          memory: 4G
          cpus: "2"
        reservations:
          memory: 2G
          cpus: "1"
```

## Kubernetes Deployment

```yaml
apiVersion: apps/v1
kind: Deployment
metadata:
  name: hacienda
spec:
  replicas: 3
  selector:
    matchLabels:
      app: hacienda
  template:
    metadata:
      labels:
        app: hacienda
    spec:
      containers:
        - name: hacienda
          image: ghcr.io/jamon8888/hacienda:latest
          ports:
            - containerPort: 8080
            - containerPort: 9090
          env:
            - name: HACIENDA_JWT_SECRET
              valueFrom:
                secretKeyRef:
                  name: hacienda-secrets
                  key: jwt-secret
            - name: HACIENDA_MODELS_DIR
              value: /var/lib/hacienda/models
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
          volumeMounts:
            - name: config
              mountPath: /app/config
            - name: models
              mountPath: /var/lib/hacienda/models
            - name: audit-logs
              mountPath: /var/log/hacienda
      volumes:
        - name: config
          configMap:
            name: hacienda-config
        - name: models
          emptyDir: {}
        - name: audit-logs
          emptyDir: {}
---
apiVersion: v1
kind: Service
metadata:
  name: hacienda
spec:
  selector:
    app: hacienda
  ports:
    - name: http
      port: 8080
      targetPort: 8080
    - name: metrics
      port: 9090
      targetPort: 9090
    - name: health
      port: 8081
      targetPort: 8081
```

## Model Management

```bash
# List available models
hacienda model list

# Download specific model
hacienda model download fastino/GLiNER2-Guardrails-PII-Multi

# Show model info
hacienda model info fastino/GLiNER2-Guardrails-PII-Multi

# Pin model version
hacienda model pin fastino/GLiNER2-Guardrails-PII-Multi@v1.2.3
```

## Monitoring

### Prometheus Metrics

```yaml
# prometheus.yml
scrape_configs:
  - job_name: "hacienda"
    static_configs:
      - targets: ["hacienda:9090"]
```

### Key Metrics

| Metric                                 | Type      | Description               |
| -------------------------------------- | --------- | ------------------------- |
| `pii_pipeline_duration_seconds`        | Histogram | Pipeline stage latencies  |
| `pii_entities_detected_total`          | Counter   | Entities by category      |
| `pii_entities_redacted_total`          | Counter   | Redacted entities by mode |
| `compliance_reports_generated_total`   | Counter   | Reports by type           |
| `audit_writes_total`                   | Counter   | Audit log writes          |
| `review_queue_depth`                   | Gauge     | Pending review items      |
| `pii_model_inference_duration_seconds` | Histogram | ML model latency          |

### Grafana Dashboard

Import dashboard from `monitoring/grafana/hacienda-dashboard.json`
