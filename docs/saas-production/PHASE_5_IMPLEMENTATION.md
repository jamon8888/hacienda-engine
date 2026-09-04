# Phase 5 Implementation: Disaster Recovery (Weeks 11-12)

> **Goal**: Automated backup/restore with tested RTO/RPO
> **Duration**: 2 weeks (10 working days)
> **Team**: 1 Platform Engineer + 1 Backend Engineer
> **Prerequisites**: Phases 1-2 complete

---

## Week 11: RTO/RPO Definition & Backup Implementation

### Day 51: RTO/RPO Definition

| Task | Owner | Acceptance Criteria |
|------|-------|---------------------|
| Define RTO/RPO with stakeholders | Platform + Product | Document signed off |
| RTO < 4 hours (service restoration) | Platform | Measured in DR drill |
| RPO < 1 hour (audit chain) | Platform | S3 export frequency |
| RPO < 5 minutes (Postgres) | Platform | PITR + WAL archiving |
| RPO < 15 minutes (S3 objects) | Platform | CRR replication lag |
| Document in DR plan | Platform | runbooks/dr-plan.md |

### Day 52-53: Automated Postgres Backup

| Task | Owner | Acceptance Criteria |
|------|-------|---------------------|
| Enable PITR on Cloud SQL | Platform | Point-in-time recovery |
| Configure WAL archiving to S3 | Platform | Continuous WAL shipping |
| Schedule daily base backup | Platform | pg_dump + WAL at 03:00 UTC |
| Configure cross-region replica | Platform | Async replica in DR region |
| Test restore to DR region | Platform | Weekly automated test |
| Document restore procedure | Platform | runbooks/db-restore.md |

### Day 54: S3 Cross-Region Replication

| Task | Owner | Acceptance Criteria |
|------|-------|---------------------|
| Enable CRR on production bucket | Platform | Replication to DR region |
| Enable versioning on both buckets | Platform | Required for CRR |
| Configure lifecycle policies | Platform | Delete markers 90d, versions 365d |
| Verify replication lag < 15 min | Platform | S3 replication metrics |
| Test failover to DR bucket | Platform | Monthly manual test |

### Day 55: Audit Chain Backup

| Task | Owner | Acceptance Criteria |
|------|-------|---------------------|
| Implement daily audit export job | Backend | Cron: 0 2 * * * |
| Export to S3 with tenant partitioning | Backend | s3://bucket/audit/tenant_id/date/ |
| Verify hash chain integrity on export | Backend | hacienda audit verify before upload |
| Implement restore from export | Backend | hacienda audit import command |
| Test restore quarterly | Platform | DR drill includes audit |

---

## Week 12: Runbooks, DR Drill, Validation

### Day 56: Runbook Documentation

| Task | Owner | Acceptance Criteria |
|------|-------|---------------------|
| Document failover procedures | Platform | runbooks/db-failover.md |
| Document restore procedures | Platform | runbooks/db-restore.md, s3-restore.md |
| Document key rotation | Platform | runbooks/pseudonym-key-compromise.md |
| Document incident response | Platform | runbooks/incident-response.md |
| Link runbooks from alerts | Platform | PrometheusRule annotations |
| Store in Git, version controlled | Platform | PR review required |

### Day 57: DR Drill Preparation

| Task | Owner | Acceptance Criteria |
|------|-------|---------------------|
| Schedule quarterly DR drill | Platform | Calendar invite, stakeholders |
| Prepare drill scenario | Platform | Region loss, data corruption |
| Create drill runbook | Platform | Step-by-step with expected times |
| Notify stakeholders | Platform | Email, Slack, status page |
| Prepare rollback plan | Platform | If drill affects production |

### Day 58: DR Drill Execution

| Task | Owner | Acceptance Criteria |
|------|-------|---------------------|
| Execute DR drill (staging) | Platform | Simulated region loss |
| Measure actual RTO/RPO | Platform | Stopwatch from detection to recovery |
| Document results | Platform | Actual vs target |
| Identify improvements | Platform | Update runbooks, automation |
| Update stakeholders | Platform | Post-drill summary |

### Day 59-60: Validation

All gate criteria must pass:
- RTO/RPO documented and approved
- PG PITR + CRR configured and tested
- Audit chain daily export to S3 working
- Restore procedures documented
- DR drill completed, actual RTO/RPO measured
- Runbooks linked from alerts