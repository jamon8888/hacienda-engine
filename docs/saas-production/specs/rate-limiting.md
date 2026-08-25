# Rate Limiting Specification

## Strategy

**Token Bucket Algorithm** per principal (API key) with Redis-backed distributed state.

## Tier Configuration

| Tier | Requests/Second | Burst | Monthly Quota | Use Case |
|------|----------------|-------|---------------|----------|
| **Free** | 5 | 10 | 10,000 docs | Development/Testing |
| **Starter** | 20 | 50 | 100,000 docs | Small teams |
| **Professional** | 100 | 200 | 1,000,000 docs | Production apps |
| **Enterprise** | Custom | Custom | Custom | High-volume |

## Implementation

```rust
// Pseudocode
struct RateLimiter {
    redis: RedisClient,
    lua_script: String, // Atomic check-and-consume
}

impl RateLimiter {
    async fn check(&self, principal_id: &str, tier: Tier) -> RateLimitResult {
        let key = format!("ratelimit:{}:{}", tier, principal_id);
        let (allowed, remaining, reset) = self.redis.eval(
            TOKEN_BUCKET_SCRIPT,
            keys: [key],
            args: [tier.rate, tier.burst, now()]
        ).await?;
        RateLimitResult { allowed, remaining, reset }
    }
}
```

## Lua Script (Atomic)

```lua
-- KEYS[1] = rate limit key
-- ARGV[1] = rate (tokens per second)
-- ARGV[2] = burst (max tokens)
-- ARGV[3] = now (unix timestamp ms)

local key = KEYS[1]
local rate = tonumber(ARGV[1])
local burst = tonumber(ARGV[2])
local now = tonumber(ARGV[3])

local bucket = redis.call('HMGET', key, 'tokens', 'last_refill')
local tokens = tonumber(bucket[1]) or burst
local last_refill = tonumber(bucket[2]) or now

-- Refill tokens
local elapsed = (now - last_refill) / 1000
local new_tokens = math.min(burst, tokens + elapsed * rate)

if new_tokens >= 1 then
    new_tokens = new_tokens - 1
    redis.call('HMSET', key, 'tokens', new_tokens, 'last_refill', now)
    redis.call('EXPIRE', key, 86400)
    return {1, math.floor(new_tokens), math.ceil((1 - new_tokens) / rate * 1000)}
else
    redis.call('HMSET', key, 'tokens', new_tokens, 'last_refill', now)
    redis.call('EXPIRE', key, 86400)
    return {0, 0, math.ceil((1 - new_tokens) / rate * 1000)}
end
```

## Response Headers

```http
# Successful request
X-RateLimit-Limit: 100
X-RateLimit-Remaining: 99
X-RateLimit-Reset: 1693000000

# Rate limited
HTTP/1.1 429 Too Many Requests
X-RateLimit-Limit: 100
X-RateLimit-Remaining: 0
X-RateLimit-Reset: 1693000000
Retry-After: 45
Content-Type: application/json

{
  "error": "rate_limit_exceeded",
  "message": "Rate limit exceeded. Retry after 45 seconds.",
  "retry_after": 45
}
```

## Endpoint-Specific Limits

| Endpoint | Limit |
|----------|-------|
| `POST /v1/documents` | Tier limit |
| `POST /v1/pii/scan` | 2x Tier limit |
| `POST /v1/pii/redact` | Tier limit |
| `POST /v1/rag/query` | 0.5x Tier limit |
| `GET /v1/audit` | 0.2x Tier limit |
| `POST /v1/uploads/presign` | 10/minute |
| `POST /v1/webhooks` | 5/minute |

## Quota Enforcement

Separate from rate limiting - monthly document/entity quotas tracked in Redis with daily rollup to Postgres.

```rust
// Monthly quota check (async, non-blocking)
async fn check_quota(tenant_id: &str) -> QuotaResult {
    let used = redis.get(format!("quota:{}:documents", tenant_id)).await?;
    let limit = get_tenant_limit(tenant_id).await?;
    
    if used >= limit {
        return QuotaResult::Exceeded;
    }
    
    // Async increment
    redis.incr(format!("quota:{}:documents", tenant_id)).await;
    QuotaResult::Ok { remaining: limit - used }
}
```

## DDoS Protection

- **Cloud Armor / Cloudflare**: Layer 7 WAF rules
- **Per-IP limits**: 1000 req/min global
- **Geo-blocking**: Configurable per tenant
- **Bot detection**: Challenge suspicious patterns

## Monitoring

- Rate limit hit rate per tier
- 429 response rate
- Quota utilization per tenant
- Top consumers dashboard
