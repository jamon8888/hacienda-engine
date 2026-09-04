# Runbook: API Down

## Trigger

- `/health` endpoint returns non-200 for > 1 minute
- 5xx error rate > 5% for 5 minutes
- Alert: `HaciendaAPIDown` firing

## Impact

- All document processing APIs unavailable
- Tenants cannot submit/extract/redact documents
- Audit chain continues (background workers)

## Diagnosis

```bash
# 1. Check pod status
kubectl get pods -n hacienda-prod -l app=hacienda-api

# 2. Check recent logs
kubectl logs -n hacienda-prod -l app=hacienda-api --tail=100

# 3. Check resource usage
kubectl top pods -n hacienda-prod -l app=hacienda-api

# 4. Check dependencies
curl -sf https://hacienda.example.com/ready  # Should check DB, Redis, S3
```

## Common Causes & Fixes

| Cause | Symptom | Fix |
|-------|---------|-----|
| OOM Kill | Pod restarts, memory spike in logs | Increase memory limit; investigate leak |
| DB Connection Exhaustion | `connection pool timeout` in logs | Scale PG; increase pool size; check for leaks |
| Redis Unavailable | `connection refused` to Redis | Check Redis pod; failover if needed |
| Config Error | Panic on startup | Rollback ArgoCD; fix ConfigMap |
| Dependency Deadlock | High CPU, no progress | Restart pods; investigate upstream |

## Escalation

- **15 min**: Page on-call if not resolved
- **30 min**: Engage Platform lead
- **1 hour**: Incident commander; status page update

## Post-Incident

- Run `hacienda audit verify` to ensure chain integrity
- Update runbook with new findings
- Schedule blameless postmortem within 48h
