# Webhook System Specification

## Overview

Webhooks allow tenants to receive real-time notifications for asynchronous events.

## Event Types

| Event | Description | Payload |
|-------|-------------|---------|
| `document.processed` | Extraction/redaction complete | Document ID, status, PII counts |
| `document.failed` | Processing failed | Document ID, error, retry info |
| `audit.exported` | Audit chain export ready | Export ID, download URL, expires |
| `review.created` | New review item queued | Review ID, document, priority |
| `review.decided` | Review decision made | Review ID, decision, reviewer |
| `job.completed` | Async job finished | Job ID, result, artifacts |
| `job.failed` | Async job failed | Job ID, error, retry info |
| `quota.warning` | Quota threshold reached | Tenant ID, current usage, limit |
| `quota.exceeded` | Quota hard limit reached | Tenant ID, current usage, limit |

## Subscription

```http
POST /v1/webhooks
Authorization: Bearer <token>
Content-Type: application/json

{
  "url": "https://tenant.example.com/webhooks/hacienda",
  "events": ["document.processed", "document.failed"],
  "secret": "tenant-provided-secret",
  "active": true
}
```

## Delivery

- **Method**: POST
- **Content-Type**: application/json
- **Headers**:
  - `X-Hacienda-Event`: Event type
  - `X-Hacienda-Delivery`: UUID
  - `X-Hacienda-Signature`: HMAC-SHA256(payload, secret)
  - `X-Hacienda-Timestamp`: Unix timestamp
  - `User-Agent`: hacienda-webhook/1.0
- **Timeout**: 10 seconds
- **Retries**: Exponential backoff (1m, 5m, 15m, 1h, 6h, 24h) - max 6 retries
- **Dead Letter**: After 6 failures, move to DLQ, alert tenant

## Payload Format

```json
{
  "id": "evt_abc123",
  "type": "document.processed",
  "created_at": "2026-08-25T10:30:00Z",
  "tenant_id": "tenant_xyz",
  "data": {
    "document_id": "doc_123",
    "status": "completed",
    "pii_detected": 5,
    "pii_redacted": 5,
    "processing_time_ms": 1250
  }
}
```

## Signature Verification

```python
import hmac
import hashlib

def verify_signature(payload: bytes, signature: str, secret: str) -> bool:
    expected = hmac.new(
        secret.encode(),
        payload,
        hashlib.sha256
    ).hexdigest()
    return hmac.compare_digest(expected, signature)
```

## Security

- HTTPS required for webhook URLs
- Secret rotation via API
- IP allowlist optional (configured per webhook)
- Maximum payload size: 1MB
- Idempotency: `X-Hacienda-Delivery` for deduplication

## Monitoring

- Delivery success/failure metrics
- Latency percentiles
- Dead letter queue size
- Per-tenant webhook health dashboard
