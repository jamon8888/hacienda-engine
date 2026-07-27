# Implementation Plan: Distribution, Packaging & Release Automation

**Spec Reference:** `docs/superpowers/specs/2025-07-24-xberg-pii-ecosystem-design.md`  
**Plan Version:** 1.0  
**Priority:** Should Have (v0.2.0)

---

## 📋 Plan Overview

| Task | Description | Est. Hours | Dependencies |
|------|-------------|------------|--------------|
| 1 | cargo-dist configuration | 2 | All crates |
| 2 | Docker multi-arch build + distroless | 3 | All binaries |
| 3 | Helm chart + OCI registry | 2 | Docker images |
| 4 | NPM package (wasm-pack) | 2 | pii-wasm |
| 5 | SBOM + Provenance + Cosign signing | 3 | All artifacts |
| 6 | Helm chart + cosign signing | 2 | Helm chart |
| 7 | Auto-update (self_update) | 2 | CLI/MCP binaries |
| 8 | Release automation + checklist | 3 | All above |

**Total Estimated:** ~20 hours

---

## 📦 Task 1: cargo-dist Configuration

### Files
```
.cargo-dist/config.toml
.github/workflows/release.yml
```

### .cargo-dist/config.toml
```toml
[workspace]
include = ["pii-cli", "pii-mcp-server", "pii-api"]

[dist]
targets = [
  "x86_64-unknown-linux-gnu",
  "aarch64-unknown-linux-gnu", 
  "x86_64-apple-darwin",
  "aarch64-apple-darwin",
  "x86_64-pc-windows-msvc"
]

artifact-formats = ["tar.gz", "tar.zst", "zip"]
installers = ["sh", "msi", "dmg"]

ci = "github"
update-registry = "github"
announcement = "github-release"

[profile.release]
opt-level = 3
lto = "fat"
codegen-units = 1
strip = true
panic = "abort"

[metadata]
description = "GDPR/DORA/AI Act compliant PII detection & redaction"
license = "Apache-2.0"
repository = "https://github.com/your-org/xberg-pii"
homepage = "https://github.com/your-org/xberg-pii"
documentation = "https://docs.your-org.com/xberg-pii"
```

### GitHub Actions Workflow
```yaml
# .github/workflows/release.yml
name: Release

on:
  push:
    tags: ['v*']
  workflow_dispatch:

permissions:
  contents: write
  packages: write
  id-token: write
  attestations: write

jobs:
  test:
    runs-on: ubuntu-latest
    strategy:
      matrix:
        rust: [stable, beta]
        target: [x86_64-unknown-linux-gnu, aarch64-unknown-linux-gnu]
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
        with:
          toolchain: ${{ matrix.rust }}
          target: ${{ matrix.target }}
      - uses: Swatinem/rust-cache@v2
      - run: cargo test --workspace --target ${{ matrix.target }}

  build:
    needs: test
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: axodotdev/cargo-dist-action@v1
        with:
          args: build --artifacts=global --targets=x86_64-unknown-linux-gnu,aarch64-unknown-linux-gnu,x86_64-apple-darwin,aarch64-apple-darwin,x86_64-pc-windows-msvc
      - uses: actions/upload-artifact@v4
        with:
          name: dist-artifacts
          path: target/artifacts/

  wasm:
    needs: test
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
        with:
          target: wasm32-unknown-unknown
      - run: |
          cd crates/pii-wasm
          wasm-pack build --target web --target nodejs --target wasm32-wasip1 --release --out-dir pkg
      - name: Publish to NPM
        if: startsWith(github.ref, 'refs/tags/v')
        run: |
          cd crates/pii-wasm/pkg/web
          npm publish --access public --provenance
        env:
          NODE_AUTH_TOKEN: ${{ secrets.NPM_TOKEN }}

  docker:
    needs: build
    runs-on: ubuntu-latest
    permissions:
      packages: write
      id-token: write
      attestations: write
    steps:
      - uses: actions/checkout@v4
      - uses: docker/setup-qemu-action@v3
      - uses: docker/setup-buildx-action@v3
      - uses: docker/login-action@v3
        with:
          registry: ghcr.io
          username: ${{ github.actor }}
          password: ${{ secrets.GITHUB_TOKEN }}
      - id: meta
        uses: docker/metadata-action@v5
        with:
          images: ghcr.io/${{ github.repository }}/pii-api
          tags: |
            type=ref,event=tag
            type=sha,prefix=
            type=raw,value=latest,enable={{is_default_branch}}
      - id: build
        uses: docker/build-push-action@v5
        with:
          context: .
          file: crates/pii-api/Dockerfile
          platforms: linux/amd64,linux/arm64
          push: true
          tags: ${{ steps.meta.outputs.tags }}
          labels: ${{ steps.meta.outputs.labels }}
          cache-from: type=gha
          cache-to: type=gha,mode=max
          sbom: true
          provenance: true
      - name: Sign with cosign
        if: startsWith(github.ref, 'refs/tags/v')
        uses: sigstore/cosign-installer@v3
        with:
          cosign-release: 'latest'
      - run: |
          cosign sign --yes ghcr.io/${{ github.repository }}/pii-api@${{ steps.build.outputs.digest }}
          cosign attest --yes --type=spdx ghcr.io/${{ github.repository }}/pii-api@${{ steps.build.outputs.digest }}

  helm:
    needs: docker
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - run: |
          cd helm/xberg-pii
          helm package . --version ${{ github.ref_name }} --app-version ${{ github.ref_name }}
          helm package . --sign --keyring ~/.gnupg/pubring.kbx --key ${{ secrets.HELM_SIGNING_KEY }}
      - run: |
          helm push xberg-pii-${{ github.ref_name }}.tgz oci://ghcr.io/${{ github.repository }}/charts
      - run: cosign sign --yes ghcr.io/${{ github.repository }}/charts/xberg-pii:${{ github.ref_name }}

  release:
    needs: [build, wasm, docker, helm]
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: actions/download-artifact@v4
        with:
          name: dist-artifacts
          path: dist/
      - run: |
          cd dist
          sha256sum * > checksums.txt
          minisign -Sm * -K ${{ secrets.MINISIGN_KEY }}
      - uses: softprops/action-gh-release@v1
        with:
          files: |
            dist/*
            dist/checksums.txt
            dist/*.minisig
          generate_release_notes: true
          draft: false
          prerelease: ${{ contains(github.ref_name, 'rc') || contains(github.ref_name, 'beta') }}
```

---

## 🐳 Task 2: Docker Multi-arch + Distroless

### Files
```
crates/pii-api/Dockerfile
crates/pii-mcp-server/Dockerfile
crates/pii-cli/Dockerfile
.dockerignore
```

### pii-api Dockerfile
```dockerfile
# crates/pii-api/Dockerfile
# syntax = docker/dockerfile:1.7

# 1. PLANNER - cargo-chef for cache
FROM rust:1.78-slim AS planner
WORKDIR /app
RUN cargo install cargo-chef --locked
COPY . .
RUN cargo chef prepare --workspace --recipe-path recipe.json

# 2. BUILDER - Compile deps (cached)
FROM rust:1.78-slim AS builder
WORKDIR /app
RUN cargo install cargo-chef --locked
COPY --from=planner /app/recipe.json recipe.json
RUN cargo chef cook --release --workspace --recipe-path recipe.json

COPY . .
RUN cargo build --release --workspace --bin pii-api

# 3. RUNTIME - Minimal, non-root, distroless
FROM gcr.io/distroless/cc-debian12:nonroot AS runtime
WORKDIR /app

COPY --from=builder /app/target/release/pii-api /app/pii-api
COPY config/production.toml /app/config/production.toml

USER nonroot:nonroot

HEALTHCHECK --interval=30s --timeout=5s --start-period=10s --retries=3 \
  CMD ["/app/pii-api", "health-check"]

ENTRYPOINT ["/app/pii-api"]
CMD ["serve", "http", "--config", "/app/config/production.toml"]

LABEL org.opencontainers.image.title="xberg-pii API" \
      org.opencontainers.image.description="GDPR/DORA/AI Act compliant PII detection & redaction API" \
      org.opencontainers.image.licenses="Apache-2.0" \
      org.opencontainers.image.source="https://github.com/your-org/xberg-pii" \
      org.opencontainers.image.documentation="https://docs.your-org.com/xberg-pii"
```

### pii-mcp-server Dockerfile
```dockerfile
# crates/pii-mcp-server/Dockerfile
FROM rust:1.78-slim AS builder
WORKDIR /app
RUN cargo install cargo-chef --locked
COPY . .
RUN cargo chef prepare --recipe-path recipe.json
RUN cargo chef cook --release --recipe-path recipe.json
COPY . .
RUN cargo build --release --bin pii-mcp-server

FROM gcr.io/distroless/cc-debian12:nonroot
WORKDIR /app
COPY --from=builder /app/target/release/pii-mcp-server /app/pii-mcp-server
USER nonroot:nonroot
ENTRYPOINT ["/app/pii-mcp-server"]
CMD ["serve", "mcp", "--config", "/config/production.toml"]
```

---

## ⎈ Task 3: Helm Chart + OCI Registry

### Files
```
helm/xberg-pii/
├── Chart.yaml
├── values.yaml
├── templates/
│   ├── deployment.yaml
│   ├── service.yaml
│   ├── configmap.yaml
│   ├── secret.yaml
│   ├── hpa.yaml
│   ├── servicemonitor.yaml
│   └── _helpers.tpl
└── README.md
```

### Chart.yaml
```yaml
apiVersion: v2
name: xberg-pii
description: GDPR/DORA/AI Act compliant PII detection & redaction
type: application
version: 1.0.0
appVersion: "1.0.0"
keywords:
  - pii
  - gdpr
  - dora
  - ai-act
  - redaction
maintainers:
  - name: xberg-pii team
    email: pii@company.com
```

### values.yaml
```yaml
replicaCount: 3

image:
  repository: ghcr.io/your-org/xberg-pii
  tag: latest
  pullPolicy: IfNotPresent

imagePullSecrets: []
nameOverride: ""
fullnameOverride: ""

service:
  type: ClusterIP
  port: 8080

resources:
  limits:
    cpu: 2000m
    memory: 4Gi
  requests:
    cpu: 1000m
    memory: 2Gi

autoscaling:
  enabled: true
  minReplicas: 3
  maxReplicas: 20
  targetCPUUtilizationPercentage: 70

config:
  production: |
    [pii_pipeline]
    regex_first = true
    model_threshold_default = 0.5
    
    [pii_pipeline.model]
    model_id = "fastino/GLiNER2-Guardrails-PII-Multi"
    dtype = "f16"
    revision = "sha256:pinned_hash"
    
    [pii_pipeline.redaction]
    default_mode = "pseudonymize"
    fpe_key_env = "XBERG_FPE_KEY"
    
    [pii_pipeline.audit]
    enabled = true
    log_path = "/var/log/xberg-pii/audit.log"
    format = "jsonl"
    rotation = "daily"
    max_files = 90
    
    [compliance.gdpr]
    enabled = true
    dpo_email = "dpo@company.com"
    
    [compliance.dora]
    enabled = true
    entity_type = "financial"
    
    [compliance.ai_act]
    enabled = true
    risk_level = "high"
    intended_use = "document_processing_financial"

serviceMonitor:
  enabled: true
  interval: 30s

podSecurityContext:
  runAsNonRoot: true
  runAsUser: 1000
  fsGroup: 1000
  seccompProfile:
    type: RuntimeDefault

nodeSelector: {}
tolerations: []
affinity: {}
```

### Deployment Template
```yaml
# templates/deployment.yaml
apiVersion: apps/v1
kind: Deployment
metadata:
  name: {{ include "xberg-pii.fullname" . }}
  labels: {{- include "xberg-pii.labels" . | nindent 4 }}
spec:
  replicas: {{ .Values.replicaCount }}
  selector:
    matchLabels: {{- include "xberg-pii.selectorLabels" . | nindent 6 }}
  template:
    metadata:
      labels: {{- include "xberg-pii.selectorLabels" . | nindent 8 }}
      annotations:
        checksum/config: {{ include (print $.Template.BasePath "/configmap.yaml") . | sha256sum }}
    spec:
      serviceAccountName: {{ include "xberg-pii.serviceAccountName" . }}
      securityContext: {{- toJson .Values.podSecurityContext | nindent 8 }}
      containers:
        - name: {{ .Chart.Name }}
          image: "{{ .Values.image.repository }}:{{ .Values.image.tag }}"
          imagePullPolicy: {{ .Values.image.pullPolicy }}
          ports:
            - name: http
              containerPort: 8080
              protocol: TCP
          env:
            - name: RUST_LOG
              value: {{ .Values.config.log_level | default "info" }}
            - name: XBERG_FPE_KEY
              valueFrom:
                secretKeyRef:
                  name: {{ include "xberg-pii.fullname" . }}-secrets
                  key: fpe-key
          ports:
            - containerPort: 8080
          resources: {{- toJson .Values.resources | nindent 12 }}
          livenessProbe:
            httpGet:
              path: /health
              port: http
            initialDelaySeconds: 10
            periodSeconds: 30
          readinessProbe:
            httpGet:
              path: /ready
              port: http
            initialDelaySeconds: 5
            periodSeconds: 10
          volumeMounts:
            - name: config
              mountPath: /config
            - name: audit-logs
              mountPath: /var/log/xberg-pii
      volumes:
        - name: config
          configMap:
            name: {{ include "xberg-pii.fullname" . }}-config
        - name: audit-logs
          emptyDir: {}
```

---

## 📦 Task 4: NPM Package (wasm-pack)

### Build Script
```bash
# build-wasm.sh
#!/bin/bash
set -e

# Browser target
wasm-pack build \
  --target web \
  --out-dir pkg/web \
  --features web \
  --release

# Node.js target
wasm-pack build \
  --target nodejs \
  --out-dir pkg/node \
  --features wasi \
  --release

# WASI target
wasm-pack build \
  --target wasm32-wasip1 \
  --out-dir pkg/wasi \
  --features wasi \
  --release

# Generate package.json
cat > pkg/web/package.json << 'EOF'
{
  "name": "@xberg/pii-wasm",
  "version": "0.1.0",
  "description": "GDPR/DORA/AI Act compliant PII detection for Web, Node.js, WASI",
  "keywords": ["pii", "gdpr", "dora", "ai-act", "redaction", "wasm", "gliner"],
  "license": "Apache-2.0",
  "main": "pii_wasm.js",
  "module": "pii_wasm.js",
  "types": "pii_wasm.d.ts",
  "sideEffects": false,
  "files": [
    "pii_wasm.js",
    "pii_wasm.d.ts",
    "pii_wasm_bg.wasm"
  ],
  "scripts": {
    "test:web": "vitest run --pool=web",
    "test:node": "vitest run --pool=node"
  },
  "engines": {
    "node": ">=18"
  }
}
EOF

cp pkg/web/package.json pkg/node/
cp pkg/web/package.json pkg/wasi/
```

---

## 🔐 Task 5: SBOM + Provenance + Cosign Signing

### SBOM Generation
```yaml
# .github/workflows/sbom.yml
name: SBOM & Attestations

on:
  workflow_run:
    workflows: ["Release"]
    types: [completed]

jobs:
  sbom:
    runs-on: ubuntu-latest
    permissions:
      contents: read
      id-token: write
      attestations: write
      packages: write
    steps:
      - uses: actions/checkout@v4
      
      - name: Generate Cargo SBOM (CycloneDX)
        uses: cyclonedx/github-action@v1
        with:
          command: generate
          output: sbom-cargo.cdx.json
          format: json
          
      - name: Generate Docker SBOM (Syft)
        uses: anchore/sbom-action@v0
        with:
          image: ghcr.io/${{ github.repository }}/pii-api:${{ github.sha }}
          format: spdx-json
          output-file: sbom-docker.spdx.json
          
      - name: Sign & Attest
        if: startsWith(github.ref, 'refs/tags/v')
        run: |
          # Sign binaries
          cosign sign --yes \
            ghcr.io/${{ github.repository }}/pii-api@${{ github.sha }}
          
          # Attest SBOM Cargo
          cosign attest --yes \
            --type=cyclonedx \
            --predicate=sbom-cargo.cdx.json \
            ghcr.io/${{ github.repository }}/pii-api@${{ github.sha }}
            
          # Attest SBOM Docker
          cosign attest --yes \
            --type=spdx \
            --predicate=sbom-docker.spdx.json \
            ghcr.io/${{ github.repository }}/pii-api@${{ github.sha }}
            
          # Attest Provenance (SLSA)
          cosign attest --yes \
            --type=slsaprovenance \
            --predicate=provenance.intoto.json \
            ghcr.io/${{ github.repository }}/pii-api@${{ github.sha }}
```

### Cosign Verification (User-facing)
```bash
# Verify binary
minisign -Vm pii-cli -P RWQf6LRCGA9i53mlYecO4IzT51TGPpvWucNSCh1CBM0QTaLn73Y7GFO3

# Verify Docker image
cosign verify ghcr.io/your-org/pii-api:v1.0.0 \
  --certificate-identity-regexp '^https://github.com/your-org/xberg-pii/.github/workflows/release\.yml@' \
  --certificate-oidc-issuer 'https://token.actions.githubusercontent.com'

# Verify SBOM
cosign verify-attestation --type spdx ghcr.io/your-org/pii-api:v1.0.0 | jq .

# Verify Helm chart
cosign verify ghcr.io/your-org/charts/xberg-pii:v1.0.0
helm verify xberg-pii-1.0.0.tgz
```

---

## 🔄 Task 6: Helm Chart Signing

```bash
# In CI/CD pipeline
helm package . --version ${{ github.ref_name }} --app-version ${{ github.ref_name }}
helm package . --sign --keyring ~/.gnupg/pubring.kbx --key ${{ secrets.HELM_SIGNING_KEY }}

helm push xberg-pii-${{ github.ref_name }}.tgz oci://ghcr.io/${{ github.repository }}/charts

cosign sign --yes ghcr.io/${{ github.repository }}/charts/xberg-pii:${{ github.ref_name }}
```

### Verify
```bash
cosign verify ghcr.io/your-org/charts/xberg-pii:v1.0.0
helm verify xberg-pii-1.0.0.tgz
```

---

## 🔄 Task 7: Auto-Update (self_update)

### CLI Integration
```rust
// pii-cli/src/self_update.rs
use self_update::backends::github::Update;

pub async fn check_and_update() -> Result<()> {
    let status = Update::configure()
        .repo_owner("your-org")
        .repo_name("xberg-pii")
        .bin_name("pii-cli")
        .current_version(env!("CARGO_PKG_VERSION"))
        .build()?
        .update()?;
    
    match status.updated() {
        true => println!("Updated to version {}", status.version()),
        false => println!("Already up to date ({})", env!("CARGO_PKG_VERSION")),
    }
    Ok(())
}
```

### MCP Server Auto-Update
```toml
# config/production.toml
[server.mcp]
auto_update = true
update_channel = "stable"  # stable | beta | nightly
update_interval_hours = 24
```

```rust
// pii-mcp-server/src/self_update.rs
pub async fn auto_update(config: &McpConfig) -> Result<()> {
    if !config.auto_update { return Ok(()); }
    
    let status = Update::configure()
        .repo_owner("your-org")
        .repo_name("xberg-pii")
        .bin_name("pii-mcp-server")
        .current_version(env!("CARGO_PKG_VERSION"))
        .build()?
        .update()?;
    
    if status.updated() {
        tracing::info!("Auto-updated to version {}", status.version());
        // Signal restart needed
    }
    Ok(())
}
```

---

## ✅ Task 7: Release Checklist Automation

### Release Checklist PR Check
```yaml
# .github/workflows/release-checklist.yml
name: Release Checklist

on:
  pull_request:
    types: [opened, synchronize, reopened]

jobs:
  checklist:
    runs-on: ubuntu-latest
    steps:
      - name: Check conventional commits
        uses: gsactions/commit-message-checker@v2
        with:
          pattern: '^(feat|fix|docs|style|refactor|perf|test|chore|release)(\(.+\))?: .+'
          
      - name: Check version bump
        run: |
          if [[ "${{ github.event.pull_request.title }}" =~ ^release: ]]; then
            echo "Release PR detected"
          fi
          
      - name: Check CHANGELOG updated
        run: |
          if ! git diff --name-only origin/main...HEAD | grep -q "CHANGELOG.md"; then
            echo "::error::CHANGELOG.md not updated"
            exit 1
          fi
          
      - name: Check version consistency
        run: |
          cargo metadata --format-version=1 | jq -r '.packages[] | select(.name | startswith("pii-")) | "\(.name)=\(.version)"' | sort | uniq -c | awk '$1 > 1 {print "Version mismatch: " $2}' && exit 1 || echo "All versions consistent"
```

---

## 📋 Task 8: Release Artifacts Summary

| Artifact | Signature | SBOM | Provenance | Auto-update |
|---|---|---|---|---|
| `pii-cli` (tar.gz, zip, msi, dmg) | ✅ minisign + cosign | ✅ CycloneDX | ✅ SLSA Level 2 | ✅ self_update |
| `pii-mcp-server` (tar.gz) | ✅ minisign + cosign | ✅ CycloneDX | ✅ SLSA Level 2 | ✅ cargo-dist |
| `pii-api` (Docker multi-arch) | ✅ cosign | ✅ SPDX + CycloneDX | ✅ SLSA Level 3 | — |
| `pii-wasm` (NPM) | ✅ npm attestations | ✅ CycloneDX | ✅ npm provenance | — |
| Helm Chart | ✅ cosign + gpg | ✅ CycloneDX | ✅ cosign | — |

---

## 📅 Timeline

| Week | Focus |
|------|-------|
| 1 | cargo-dist + Docker + SBOM |
| 2 | Helm + NPM + Cosign |
| 3 | Auto-update + Release automation |
| 4 | Testing + Documentation |

---

## ✅ Acceptance Criteria

| Criterion | Target |
|---|---|
| `cargo-dist` builds all 3 binaries for 5 targets | ✅ |
| Docker images multi-arch (amd64/arm64) | ✅ |
| SBOM (CycloneDX + SPDX) attached to release | ✅ |
| Cosign signatures on all artifacts | ✅ |
| Helm chart in OCI registry + signed | ✅ |
| NPM package with provenance | ✅ |
| `pii-cli --self-update` works | ✅ |
| `pii-mcp-server` auto-updates | ✅ |
| Release PR checklist enforced | ✅ |

---

**Plan Status:** Ready for execution  
**Next Plan:** `06-observability-production.md` (Production hardening)