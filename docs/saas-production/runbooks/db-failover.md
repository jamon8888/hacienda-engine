# Runbook: Database Failover

## Trigger

- PostgreSQL primary unreachable (health check failing)
- Alert: PostgresPrimaryDown firing
- Application logs show connection refused to primary

## Impact

- Automatic failover (managed PG): ~30-60 seconds downtime
- Manual failover: 5-15 minutes
- In-flight transactions may fail (retry from client)
- Read replicas may lag

## Diagnosis

```bash
# 1. Check Cloud SQL / RDS status in console
# 2. Verify replica status
kubectl exec -n hacienda-prod <postgres-client-pod> -- pg_isready -h <replica-host>

# 3. Check replication lag
kubectl exec -n hacienda-prod <postgres-client-pod> -- psql -h <replica-host> -c "SELECT EXTRACT(EPOCH FROM (now() - pg_last_xact_replay_timestamp())) AS lag_seconds;"

# 4. Check application connection pool
kubectl logs -n hacienda-prod -l app=hacienda-api | grep -i pool
```

## Automatic Failover (Managed PG)

**Cloud SQL / RDS / Cloud SQL**:
1. Failover typically automatic within 60 seconds
2. Monitor kubectl get pods -n hacienda-prod -l app=hacienda-api for restarts
3. Verify /ready endpoint returns healthy
4. Check application logs for successful reconnection

## Manual Failover (Self-Hosted)

```bash
# 1. Promote replica
kubectl exec -n hacienda-prod <replica-pod> -- pg_ctl promote -D /var/lib/postgresql/data

# 2. Update DNS / Service endpoint
kubectl patch svc hacienda-postgres -n hacienda-prod -p '{"spec":{"selector":{"role":"primary"}}}'

# 3. Verify new primary
kubectl exec -n hacienda-prod <postgres-client-pod> -- pg_isready -h hacienda-postgres

# 4. Restart API pods to pick up new endpoint
kubectl rollout restart deployment/hacienda-api -n hacienda-prod
```

## Post-Failover

```bash
# 1. Verify replication re-established
# 2. Run audit chain verification
hacienda audit verify /var/log/hacienda/audit

# 3. Check for data loss
# Compare row counts on critical tables

# 4. Update runbook if manual steps were needed
```

## Escalation

- 5 min: If auto-failover hasn't triggered
- 15 min: If manual failover in progress > 10 min
- 30 min: Engage DBA / Cloud support

## Prevention

- Enable automatic failover on managed PG
- Set max_replication_slots and wal_keep_size appropriately
- Monitor replication lag alert (< 30s warning, < 60s critical)
- Test failover quarterly (DR drill)
