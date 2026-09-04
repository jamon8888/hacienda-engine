# Phase 1 Implementation: Horizontal Scaling (Weeks 3-4)

> **Goal**: Stateless API with horizontal scaling capability
> **Duration**: 2 weeks (10 working days)
> **Team**: 2 Backend Engineers
> **Prerequisites**: Phase 0 complete (PG, Redis, S3, K8s, ArgoCD, Vault)

---

## Week 3: Stateless API & PostgreSQL Stores

### Day 11-12: Verify Stateless API

| Task | Owner | Acceptance Criteria |
|------|-------|---------------------|
| Audit hacienda-api for local file state | Backend | No writes to local filesystem except /tmp |
| Configure all stores for PostgreSQL | Backend | FileAuditStore -> PostgresAuditStore |
| Configure review queue for PostgreSQL | Backend | FileReviewQueue -> PostgresReviewQueue |
| Configure RAG for PgVector | Backend | MemoryRagStore -> PgVectorRagStore |
| Verify config via production.toml | Backend | All backend = "postgres" settings |

### Day 13-14: PostgreSQL Audit Store Hardening

| Task | Owner | Acceptance Criteria |
|------|-------|---------------------|
| Implement PostgresAuditStore | Backend | Implements AuditStore trait fully |
| Add connection pooling config | Backend | sqlx pool: min 5, max 20 connections |
| Implement segment coordination | Backend | node_id + tenant_id segmentation |
| Add batch insert for performance | Backend | Batch 100 entries, flush every 1s |
| Add retry with exponential backoff | Backend | Retry 3x on transient errors |
| Write integration tests | Backend | Testcontainers PG, concurrent writers |

### Day 15: PostgreSQL Review Queue

| Task | Owner | Acceptance Criteria |
|------|-------|---------------------|
| Implement PostgresReviewQueue | Backend | Implements ReviewQueue trait |
| Add concurrent assignment safety | Backend | SELECT ... FOR UPDATE SKIP LOCKED |
| Add priority ordering | Backend | Order by priority DESC, created_at ASC |
| Add deadline/escalation tracking | Backend | Background job checks deadlines |
| Write integration tests | Backend | Testcontainers PG, concurrent reviewers |

---

## Week 4: Redis, HPA, Graceful Shutdown

### Day 16: Redis for Caching & Rate Limiting

| Task | Owner | Acceptance Criteria |
|------|-------|---------------------|
| Add Redis connection pool | Backend | deadpool-redis pool, configurable size |
| Implement rate limiter (token bucket) | Backend | Lua script for atomicity |
| Add distributed lock for jobs | Backend | SET NX EX pattern |
| Add session cache for auth | Backend | TTL 15m, invalidate on key rotation |
| Configure Redis Cluster mode | Platform | 3 masters + replicas, sentinel |

### Day 17: Horizontal Pod Autoscaler

| Task | Owner | Acceptance Criteria |
|------|-------|---------------------|
| Verify /metrics endpoint exposes custom metrics | Backend | http_requests_per_second available |
| Configure HPA with CPU, memory, custom metrics | Platform | deploy/base/hpa.yaml applied |
| Test scale-up under load | Backend | k6 test triggers scale-up |
| Test scale-down stabilization | Platform | 5 min stabilization window |
| Configure PodDisruptionBudget | Platform | minAvailable: 2 |

### Day 18: Graceful Shutdown

| Task | Owner | Acceptance Criteria |
|------|-------|---------------------|
| Implement SIGTERM handler | Backend | Stops accepting new requests |
| Drain in-flight requests | Backend | Wait up to 30s for completion |
| Close DB/Redis connections gracefully | Backend | pool.close().await |
| Add preStop hook in deployment | Platform | sleep 30 before SIGTERM |
| Test zero-downtime deploy | Platform | Rolling update with 0 errors |

### Day 19: Readiness Probe Implementation

| Task | Owner | Acceptance Criteria |
|------|-------|---------------------|
| Implement /ready endpoint | Backend | Checks PG, Redis, S3 connectivity |
| Return 503 if dependencies unhealthy | Backend | K8s removes pod from service |
| Distinguish from /health (liveness) | Backend | /health = process alive only |
| Test pod removal during DB outage | Platform | Traffic routed to healthy pods |

### Day 20: Load Testing & Validation

All gate criteria must pass:
- Multiple replicas serving traffic
- No local file state
- PG audit store working
- PG review queue working
- PgVector RAG working
- Redis rate limiting working
- HPA scales on CPU/memory/custom metrics
- Graceful shutdown tested
- Readiness probe works
