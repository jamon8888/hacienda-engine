# Runbook: Tenant Data Deletion (GDPR Article 17)

## Trigger

- Tenant submits Data Deletion Request (DSAR)
- Legal team validates request
- Automated via tenant portal or manual via API

## Legal Requirements

- GDPR Art. 17: Right to erasure
- Response deadline: 30 days (extendable to 60)
- Verification: Confirm identity before deletion
- Scope: All personal data, including backups
- Exceptions: Legal obligations, public interest, legal claims

## Process

### 1. Request Validation (Day 1-2)

Verify tenant identity, check request authenticity, confirm scope, log in compliance register.

### 2. Data Inventory (Day 2-5)

Identify all data stores for tenant across PostgreSQL, S3, Redis.

### 3. Deletion Execution (Day 5-20)

#### Phase A: Soft Delete (Reversible)

Mark tenant as deleted, revoke API keys, disable webhooks.

#### Phase B: Hard Delete (Irreversible)

Delete PostgreSQL data in FK order, delete S3 objects, delete Redis keys, mark audit segments as deleted.

### 4. Backup Purge (Day 20-25)

Identify backups, plan purge (PITR cannot purge individual tenant, logical backups can be restored/deleted/re-backed-up).

### 5. Verification (Day 25-28)

Verify no tenant data remains in any store, verify audit chain integrity.

### 6. Confirmation (Day 28-30)

Generate deletion certificate with completion details.

## Escalation

- Day 15: If deletion not started -> Legal escalation
- Day 25: If verification fails -> Engineering lead
- Day 30: Deadline -> DPO notification

## Special Cases

- Active legal hold: Reject deletion, notify legal
- Shared documents: Delete tenant's copy only
- Anonymized analytics: Retain (not personal data)
- Audit chain: Mark segments deleted, preserve chain integrity
