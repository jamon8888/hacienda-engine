# Phase 4 Implementation: Security Hardening (Weeks 9-10)

> **Goal**: Production-grade security posture
> **Duration**: 2 weeks (10 working days)
> **Team**: 1 Security Engineer + 1 Backend Engineer
> **Prerequisites**: Phase 1 complete

---

## Week 9: Secrets Rotation, Rate Limiting, Input Validation

### Day 41-42: Secrets Rotation Automation

| Task | Owner | Acceptance Criteria |
|------|-------|---------------------|
| Configure ExternalSecret refreshInterval | Platform | 1h for prod, 15m for staging |
| Implement quarterly pseudonym key rotation | Security | rotate-pseudonym-key.sh script |
| Implement API key rotation on demand | Backend | POST /v1/auth/keys/{id}/rotate |
| Implement DB password rotation | Platform | Cloud SQL automated rotation |
| Implement S3 credential rotation | Platform | IAM key rotation |
| Add rotation audit trail | Security | Vault audit log entries |
| Document emergency rotation procedure | Security | Runbook for key compromise |

### Day 43: API Rate Limiting & Abuse Protection

| Task | Owner | Acceptance Criteria |
|------|-------|---------------------|
| Deploy token bucket rate limiter | Backend | Redis-backed, per principal |
| Configure tier limits (Free/Starter/Pro/Ent) | Backend | From config/quotas.toml |
| Add endpoint-specific multipliers | Backend | /rag/query = 0.5x, /pii/scan = 2x |
| Add 429 response with Retry-After | Backend | Proper headers |
| Deploy Cloud Armor / Cloudflare WAF | Platform | Layer 7 DDoS protection |
| Add per-IP global limit | Platform | 1000 req/min |
| Add geo-blocking config | Platform | Per-tenant configurable |

### Day 44: Input Validation & Sanitization

| Task | Owner | Acceptance Criteria |
|------|-------|---------------------|
| Add Zod schemas for all API boundaries | Backend | Request/response validation |
| Implement MIME type validation | Backend | From content, not extension |
| Add file size limits | Backend | Per-endpoint configurable |
| Add zip bomb protection | Backend | Max compression ratio 100:1 |
| Add request body size limits | Platform | Ingress: 1GB, API: 100MB |
| Add XML/XXE protection | Backend | Disable external entities |
| Add path traversal protection | Backend | Normalize paths |

## Week 10: Dependency Scanning, Pen Test, Compliance

### Day 45: Dependency Scanning & SBOM

| Task | Owner | Acceptance Criteria |
|------|-------|---------------------|
| Enable cargo-audit in CI | Platform | .github/workflows/ci-security.yaml |
| Enable cargo-deny in CI | Platform | deny.toml configured |
| Enable Trivy filesystem scan | Platform | SARIF upload to GitHub |
| Enable Trivy image scan | Platform | On main branch pushes |
| Enable Syft SBOM generation | Platform | SPDX JSON artifact |
| Enable Grype scan on SBOM | Platform | Fail on HIGH+ |
| Add dependabot/renovate for auto-PRs | Platform | Weekly dependency updates |

### Day 46: Penetration Test Preparation

| Task | Owner | Acceptance Criteria |
|------|-------|---------------------|
| Define pen test scope | Security | API, infra, auth, data isolation |
| Select 3rd party pen test vendor | Security | CREST/OSCP certified |
| Schedule test (staging env) | Security | 2-week engagement |
| Prepare test credentials | Security | Tenant A + Tenant B tokens |
| Document known limitations | Security | WAF, rate limiting in place |

### Day 47: Penetration Test Execution

| Task | Owner | Acceptance Criteria |
|------|-------|---------------------|
| Run external pen test | Vendor | OWASP Top 10 coverage |
| Run internal pen test | Vendor | AuthZ, data isolation, injection |
| Run API fuzzing | Vendor | schemathesis/fuzzers |
| Document findings | Vendor | Report with CVSS scores |
| Create remediation tickets | Security | Critical 7d, High 30d, Med 90d |

### Day 48-49: Compliance Attestation Pack

| Task | Owner | Acceptance Criteria |
|------|-------|---------------------|
| Generate SOC2 Type II evidence | Compliance | Audit chain + configs + runbooks |
| Map ISO 27001 controls | Compliance | Control -> implementation |
| Generate GDPR DPIA artifact | Compliance | hacienda compliance dpia |
| Generate AI Act conformity assessment | Compliance | hacienda compliance checklist |
| Create compliance dashboard | Platform | Grafana: compliance status |
| Schedule annual re-assessment | Compliance | Calendar reminders |

### Day 50: Validation

```bash
# 1. Verify ExternalSecret rotation works
kubectl annotate externalsecret hacienda-secrets -n hacienda-prod force-sync=$(date +%s)

# 2. Test rate limiting
for i in {1..150}; do curl -H "Authorization: Bearer $TOKEN" https://api.example.com/v1/pii/scan -d '{"text":"test"}'; done
# Should get 429 after tier limit

# 3. Test input validation
# Oversized file -> 413
# Invalid MIME -> 400
# Zip bomb -> 400
# XXE payload -> 400

# 4. Verify CI security scans pass
# cargo-audit, cargo-deny, Trivy, Grype all green

# 5. Pen test findings remediated per SLA

# 6. Compliance artifacts generated
# hacienda compliance dpia --format json > dpia.json

# 7. All gate criteria pass
✅ Secrets rotation automated
✅ Rate limiting with tiers
✅ Input validation at all boundaries
✅ CI security scans passing
✅ Pen test completed, critical/high fixed
✅ Compliance attestation pack ready