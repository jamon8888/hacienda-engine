# SaaS Production Plan - Summary

## Document Inventory

### Main Plan
- `SAAS_PRODUCTION_PLAN.md` - Complete 18-week phased roadmap

### Specifications (`specs/`)
| File | Description |
|------|-------------|
| `api-versioning.md` | URL versioning, deprecation policy, breaking changes |
| `deployment.md` | Environments, pipeline, rollback, canary, health checks |
| `rate-limiting.md` | Token bucket, tiers, Lua script, headers, DDoS |
| `secrets.md` | Vault + ExternalSecrets, rotation, key hierarchy |
| `webhooks.md` | Events, delivery, signatures, security, monitoring |

### Runbooks (`runbooks/`)
| File | Trigger | RTO |
|------|---------|-----|
| `api-down.md` | /health failing | 15 min |
| `db-failover.md` | PG primary down | 5 min (auto) |
| `s3-outage.md` | S3 errors high | 30 min |
| `pseudonym-key-compromise.md` | Key exposure | 1 hour |
| `audit-chain-corruption.md` | Verify fails | 4 hours |
| `tenant-data-deletion.md` | GDPR Art. 17 | 30 days |
| `capacity-exhaustion.md` | HPA at max | 15 min |
| `dependency-outage.md` | Upstream down | 30 min |
| `README.md` | Index and incident process | - |

### Kubernetes Manifests (`deploy/`)
```
deploy/
├── base/
│   ├── namespace.yaml          # hacienda-prod namespace
│   ├── deployment.yaml         # Deployment + Service
│   ├── ingress.yaml            # TLS, rate limiting, security headers
│   ├── configmap.yaml          # Non-secret configuration
│   ├── secret.yaml             # Secret + ExternalSecret
│   ├── hpa.yaml                # HorizontalPodAutoscaler
│   ├── pdb.yaml                # PodDisruptionBudget + NetworkPolicy
│   └── servicemonitor.yaml     # Prometheus scrape config
├── overlays/
│   ├── staging/
│   │   └── kustomization.yaml  # 2 replicas, lower resources
│   └── production/
│       └── kustomization.yaml  # 3-20 replicas, full resources
├── monitoring/
│   ├── prometheus-rules.yaml   # 15+ alert rules
│   └── grafana-dashboards.yaml # Overview dashboard
└── dependencies/
    ├── postgresql.yaml         # CloudNativePG or managed PG
    ├── redis.yaml              # Redis Operator or managed Redis
    └── minio.yaml              # MinIO Operator or cloud S3
```

### CI/CD Workflows (`.github/workflows/`)
| Workflow | Purpose |
|----------|---------|
| `ci-rust.yaml` | Rust check, test, feature-matrix (existing) |
| `ci-docker.yaml` | Docker build + smoke test (existing) |
| `ci-lint.yaml` | Poly lint (existing) |
| `ci-postgres.yaml` | PG integration tests (existing) |
| `ci-wasm-freshness.yaml` | WASM build check (existing) |
| `ci-security.yaml` | **Enhanced**: cargo-audit, cargo-deny, Trivy, Syft, Grype |
| `ci-k8s.yaml` | **New**: Kustomize build, kubeconform, OPA policy |
| `ci-load.yaml` | **New**: k6 load testing on PR |
| `ci-contract.yaml` | **New**: Schemathesis + Pact contract tests |
| `ci-integrations.yaml` | **Enabled**: Testcontainers E2E tests |

## Phase Summary

| Phase | Weeks | Focus | Key Deliverables |
|-------|-------|-------|------------------|
| **0** | 1-2 | Foundation | Vault, PG, S3, K8s, ArgoCD, TLS |
| **1** | 3-4 | Scaling | Stateless API, PG stores, Redis, HPA, graceful shutdown |
| **2** | 5-6 | Multi-tenancy | RLS, S3 prefix isolation, audit segmentation, quotas |
| **3** | 7-8 | Observability | OTel tracing, JSON logs, SLO dashboards, alerting, chaos |
| **4** | 9-10 | Security | Secrets rotation, rate limiting, validation, pen test |
| **5** | 11-12 | Disaster Recovery | RTO/RPO, automated backups, DR drills, runbooks |
| **6** | 13-14 | Developer Experience | API versioning, 15 FFI bindings, SDK publishing, docs |
| **7** | 15-18 | SaaS Features | Billing, portal, webhooks, job scheduling, marketplace |

## Resource Requirements

- **Team**: 3-4 engineers (2 backend, 1-2 platform, 1 security part-time)
- **Timeline**: 18 weeks (~4.5 months)
- **Infrastructure Cost**: ~$4,800/month for 100 tenants

## Success Criteria

- [ ] 99.9% availability SLA met for 3 consecutive months
- [ ] p95 latency < 500ms under load
- [ ] Zero data isolation incidents in pen test
- [ ] All 8 runbooks tested in DR drill
- [ ] 15 SDK languages published
- [ ] SOC2 Type II evidence package complete
- [ ] GDPR Art. 17 deletion verified end-to-end
- [ ] Canary deployment with automated rollback working

## Next Steps

1. **Week 1**: Assign owners for Phase 0 tasks
2. **Week 1**: Provision Vault, PostgreSQL, S3, Kubernetes cluster
3. **Week 2**: Deploy ArgoCD, configure ExternalSecrets, TLS
4. **Week 2**: Validate all Phase 0 gate criteria
5. **Week 3**: Begin Phase 1 implementation

---

*Generated as part of hacienda-engine SaaS production readiness initiative*
*Worktree: feat/saas-production-plan*
*Location: /home/jamin/Documents/hacienda-engine-saas-plan/docs/saas-production/*
