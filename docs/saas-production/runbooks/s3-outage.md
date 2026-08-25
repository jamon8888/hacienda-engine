# Runbook: S3 Outage

## Trigger

- Presigned upload/download failing
- Alert: S3ErrorsHigh firing
- Application logs show S3 connection errors

## Impact

- Document uploads fail (presigned URLs)
- Document downloads fail
- Audit export to S3 fails
- RAG embeddings backup may fail

## Diagnosis

```bash
# 1. Check S3 service status (AWS/GCS/MinIO dashboard)
# 2. Test connectivity from cluster
kubectl run -n hacienda-prod --rm -i --restart=Never s3-test --image=curlimages/curl -- \
  curl -sf https://s3.example.com/health

# 3. Check application logs
kubectl logs -n hacienda-prod -l app=hacienda-api | grep -i s3

# 4. Check if it's a specific bucket or global
aws s3 ls s3://hacienda-prod --region us-east-1
```

## Common Causes & Fixes

| Cause | Fix |
|-------|-----|
| S3 service outage | Wait for provider; switch to secondary region if CRR enabled |
| Network partition | Check VPC/peering; security groups; DNS |
| Bucket policy change | Revert policy; check IAM conditions |
| Credentials expired | Rotate S3 credentials; update ExternalSecret |
| Rate limiting | Implement exponential backoff; request quota increase |

## Mitigation

```bash
# 1. If CRR enabled, redirect to secondary region
# Update S3_ENDPOINT to secondary region endpoint
kubectl patch configmap hacienda-config -n hacienda-prod \
  --patch '{"data":{"S3_ENDPOINT":"https://s3-secondary.example.com"}}'

# 2. Restart API pods to pick up new endpoint
kubectl rollout restart deployment/hacienda-api -n hacienda-prod

# 3. For uploads: queue locally, retry when S3 recovers
# 4. For downloads: serve from local cache if available
```

## Escalation

- 15 min: If S3 provider status shows degraded
- 30 min: Engage cloud support
- 1 hour: Consider read-only mode (disable uploads)

## Post-Incident

- Verify audit exports caught up
- Check for orphaned multipart uploads
- Update runbook
