# Runbook: Capacity Exhaustion

## Trigger

- HPA at max replicas for > 10 minutes
- Alert: HaciendaAPIHPAAtMax firing
- Latency increasing, queue lag growing

## Diagnosis

```bash
# 1. Check HPA status
kubectl get hpa hacienda-api -n hacienda-prod

# 2. Check resource usage
kubectl top pods -n hacienda-prod -l app=hacienda-api

# 3. Check queue lag
curl -sf https://hacienda.example.com/metrics | grep hacienda_job_queue_lag

# 4. Check dependency health
curl -sf https://hacienda.example.com/ready
```

## Common Causes & Fixes

| Cause | Indicators | Fix |
|-------|------------|-----|
| Traffic spike | Sudden request increase | Wait for HPA, check if organic |
| Slow queries | High CPU, PG slow query log | Optimize queries, add indexes |
| Memory leak | Memory growing steadily | Restart pods, investigate leak |
| Dependency slow | /ready failing, high latency | Fix dependency (PG/Redis/S3) |
| Queue backlog | Job lag increasing | Scale workers, optimize jobs |

## Mitigation

```bash
# 1. Emergency: Increase HPA max
kubectl patch hpa hacienda-api -n hacienda-prod -p '{"spec":{"maxReplicas":30}}'

# 2. Scale workers
kubectl scale deployment hacienda-worker -n hacienda-prod --replicas=10

# 3. Enable request queuing at ingress
kubectl patch ingress hacienda-api -n hacienda-prod -p '{"metadata":{"annotations":{"nginx.ingress.kubernetes.io/limit-rps":"200"}}}'

# 4. Shed load: return 503 for non-critical endpoints
# (Requires code change / feature flag)
```

## Escalation

- 15 min: If HPA at max and latency > SLO
- 30 min: If mitigation not working
- 1 hour: Engage Platform lead for architecture review

## Prevention

- Load testing before launches
- Capacity planning quarterly
- Auto-scaling policies tuned
- Circuit breakers on dependencies
