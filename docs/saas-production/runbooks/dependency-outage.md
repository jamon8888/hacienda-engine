# Runbook: Dependency Outage

## Trigger

- Upstream service unavailable (xberg, OLLAMA, candle models, etc.)
- Alert: DependencyErrorsHigh firing
- Application logs show connection errors to external services

## Dependencies

| Dependency | Purpose | Failure Mode |
|------------|---------|--------------|
| **xberg** | Document extraction | Extraction fails, fallback to text-only |
| **OLLAMA/candle** | NER/embeddings | ML features disabled, regex-only |
| **PostgreSQL** | Primary store | API down (covered in db-failover.md) |
| **Redis** | Cache/rate limit | Degraded performance, no rate limiting |
| **S3** | Object storage | Upload/download fails (covered in s3-outage.md) |
| **Vault** | Secrets | Cannot rotate, but cached secrets work |

## Diagnosis

```bash
# 1. Check which dependency
kubectl logs -n hacienda-prod -l app=hacienda-api | grep -E "(xberg|ollama|candle|redis|vault)" | tail -20

# 2. Test connectivity
kubectl run -n hacienda-prod --rm -i --restart=Never net-test --image=curlimages/curl -- \
  curl -sf http://xberg-service:8080/health

# 3. Check dependency status page
# 4. Check if it's regional or global
```

## Mitigation Strategies

### xberg Outage

```bash
# 1. Enable text-only fallback (feature flag)
kubectl set env deployment/hacienda-api -n hacienda-prod XBERG_FALLBACK_TEXT_ONLY=true

# 2. Queue documents for retry
# 3. Alert tenants of degraded extraction
```

### ML Model Outage (OLLAMA/candle)

```bash
# 1. Disable ML features
kubectl set env deployment/hacienda-api -n hacienda-prod \
  PII_MODEL_ENABLED=false \
  EMBEDDINGS_ENABLED=false

# 2. Fallback to regex-only PII detection
# 3. Queue ML tasks for retry when restored
```

### Redis Outage

```bash
# 1. Disable rate limiting (fail-open for availability)
kubectl set env deployment/hacienda-api -n hacienda-prod RATE_LIMIT_ENABLED=false

# 2. Disable caching
kubectl set env deployment/hacienda-api -n hacienda-prod CACHE_ENABLED=false

# 3. Restart Redis or failover
# 4. Re-enable when healthy
```

## Escalation

- **30 min**: If dependency not recovering
- **1 hour**: Engage dependency owner / vendor support
- **4 hours**: Consider disaster recovery for that dependency

## Post-Incident

- Re-enable all feature flags
- Process queued work
- Update runbook with new findings
- Add monitoring for faster detection

## Prevention

- Circuit breakers on all external calls
- Graceful degradation for each dependency
- Health checks in /ready endpoint
- Synthetic monitoring of dependencies
- Contractual SLAs with vendors
