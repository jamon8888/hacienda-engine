# Phase 2 Implementation: Multi-Tenancy (Weeks 5-6)

> **Goal**: Complete data isolation between tenants
> **Duration**: 2 weeks (10 working days)
> **Team**: 2 Backend Engineers
> **Prerequisites**: Phase 1 complete

---

## Week 5: Tenant Resolution & Database RLS

### Day 21-22: Tenant Resolution Middleware

| Task | Owner | Acceptance Criteria |
|------|-------|---------------------|
| Extract TenantId from JWT/API key | Backend | Middleware adds tenant_id to request extensions |
| Inject into AuthContext | Backend | AuthContext.tenant_id populated |
| Validate tenant exists & active | Backend | 401 if tenant not found, 403 if inactive |
| Reject cross-tenant requests | Backend | 403 if tenant_id mismatch |

### Day 23-24: Row-Level Security (RLS) Implementation

| Task | Owner | Acceptance Criteria |
|------|-------|---------------------|
| Add tenant_id column to all tables | Backend | Migration adds column + index |
| Create RLS policies per table | Backend | USING (tenant_id = current_setting(...)) |
| Test policy enforcement | Backend | Cross-tenant queries return 0 rows |
| Add migration for existing data | Backend | Backfill tenant_id for existing rows |
| Document RLS troubleshooting | Backend | Runbook for common issues |

### Day 25: S3 Prefix Isolation

| Task | Owner | Acceptance Criteria |
|------|-------|---------------------|
| Enforce tenant prefix on all S3 ops | Backend | All keys prefixed with tenants/{tenant_id}/ |
| Update presigned URL generation | Backend | Prefix automatically added |
| Add IAM policy for prefix restriction | Platform | Tenant can only access their prefix |
| Test cross-tenant access blocked | Backend | 403 on wrong prefix |

---

## Week 6: Audit Segmentation, Quotas, Onboarding

### Day 26: Audit Chain Per-Tenant Segmentation

| Task | Owner | Acceptance Criteria |
|------|-------|---------------------|
| Segment audit by tenant_id + node_id | Backend | Separate chain per tenant per node |
| Block cross-tenant audit queries | Backend | 403 if tenant mismatch |
| Update audit export to include tenant | Backend | Export filtered by tenant |
| Verify chain integrity per tenant | Backend | hacienda audit verify works per tenant |

### Day 27: Quota Enforcement

| Task | Owner | Acceptance Criteria |
|------|-------|---------------------|
| Define quota tiers in config | Backend | Free/Starter/Pro/Enterprise limits |
| Redis counters for real-time tracking | Backend | INCR with TTL = month end |
| 429 response with Retry-After | Backend | Proper headers on quota exceeded |
| Admin override API | Backend | POST /admin/tenants/{id}/quota |
| Monthly rollup to Postgres | Backend | Cron job persists counters |

### Day 28: Tenant Onboarding API

| Task | Owner | Acceptance Criteria |
|------|-------|---------------------|
| POST /v1/auth/tenants (admin) | Backend | Creates tenant + provisions resources |
| Provision DB schema (RLS ready) | Backend | Tenant row inserted, RLS works |
| Provision S3 prefix | Backend | Prefix exists, IAM policy attached |
| Provision default quotas | Backend | Tier-based quota initialized |
| Provision audit node | Backend | Audit segment created for tenant |
| Return tenant credentials | Backend | API key + webhook secret returned |

### Day 29-30: Integration Testing & Validation

All gate criteria must pass:
- Tenant resolution from JWT/subdomain/API key
- RLS enforced on all tables
- S3 prefix isolation working
- Audit chain segmented per tenant
- Quota enforcement with 429
- Tenant onboarding API functional
- Integration tests passing