# Phase 7 Implementation: Advanced SaaS Features (Weeks 15-18)

> **Goal**: Differentiated SaaS capabilities
> **Duration**: 4 weeks (20 working days)
> **Team**: 2 Backend Engineers + 1 Frontend Engineer
> **Prerequisites**: Phases 1-5 complete

---

## Week 15-16: Billing & Customer Portal

### Day 71-74: Usage-Based Billing Integration

| Task | Owner | Acceptance Criteria |
|------|-------|---------------------|
| Design billing data model | Backend | Invoices, line items, payments |
| Integrate Stripe/Metronome/Lago | Product + Backend | Webhook handling |
| Map metering to billing events | Backend | Documents, entities, API calls |
| Implement per-tenant invoicing | Backend | Monthly PDF/HTML invoices |
| Add billing portal (Stripe Billing Portal) | Frontend | Self-service payment methods |
| Add usage charts in portal | Frontend | Daily/monthly breakdown |
| Handle failed payments | Backend | Retry, dunning, suspension |
| Add tax calculation (Stripe Tax) | Product | Automatic tax |

#### Billing Events

| Event | Meter | Price |
|-------|-------|-------|
| Document processed | documents | $0.01/doc |
| Entity extracted | entities | $0.001/entity |
| API call | requests | Included in tier |
| RAG query | rag_queries | $0.005/query |
| Audit export | exports | $0.10/export |
| Review queue item | reviews | $0.50/item |

### Day 75-80: Customer Portal (Self-Service)

| Task | Owner | Acceptance Criteria |
|------|-------|---------------------|
| Create React portal app | Frontend | apps/hacienda-portal |
| Implement authentication (OIDC/OAuth) | Frontend | Login, SSO, MFA |
| API key management UI | Frontend | Create, rotate, revoke, scopes |
| Usage dashboard | Frontend | Charts: docs, entities, API calls, costs |
| Audit download UI | Frontend | Filter, export CSV/JSON/JSONL |
| Compliance report generation | Frontend | DPIA, Model Card, DORA, AI Act |
| Quota management | Frontend | View limits, request increase |
| Webhook management | Frontend | Create, test, view deliveries |
| Team/invite management | Frontend | Invite members, roles |
| Billing & payment methods | Frontend | Stripe Elements integration |

---

## Week 17: Webhooks & Job Scheduling

### Day 81-83: Webhook System

| Task | Owner | Acceptance Criteria |
|------|-------|---------------------|
| Design webhook event schema | Backend | JSON with id, type, timestamp, data |
| Implement POST /v1/webhooks CRUD | Backend | Create, list, get, update, delete |
| Implement delivery with retry | Backend | Exponential backoff (1m, 5m, 15m, 1h, 6h, 24h) |
| Add HMAC-SHA256 signatures | Backend | X-Hacienda-Signature header |
| Add dead letter queue | Backend | After 6 failures, alert tenant |
| Add idempotency keys | Backend | X-Hacienda-Delivery for dedup |
| Add delivery dashboard | Frontend | Success/failure, latency, retry count |
| Add webhook testing UI | Frontend | Send test event |

#### Webhook Events

- document.processed
- document.failed
- audit.exported
- review.created
- review.decided
- job.completed
- job.failed
- quota.warning (80%)
- quota.exceeded (100%)

### Day 84-85: Async Job Priority & Scheduling

| Task | Owner | Acceptance Criteria |
|------|-------|---------------------|
| Implement priority queues (Redis Streams) | Backend | High/Normal/Low streams |
| Add cron-like scheduling | Backend | schedule: "0 2 * * *" |
| Add job chaining | Backend | on_success, on_failure hooks |
| Add visibility timeout | Backend | Prevent duplicate processing |
| Add job priority API | Backend | POST /v1/jobs with priority |
| Add scheduled job management | Frontend | Create, pause, delete cron jobs |

---

## Week 18: Plugin Marketplace & Multi-Region

### Day 86-88: Custom Model/Plugin Marketplace

| Task | Owner | Acceptance Criteria |
|------|-------|---------------------|
| Design plugin manifest schema | Product | wasm/module.wit, metadata.json |
| Implement plugin registry API | Backend | CRUD for plugins |
| Implement WASM sandbox runtime | Backend | wasmtime, fuel limiting |
| Add plugin types: OCR, Embedder, Reranker, Tokenizer, Validator, Renderer | Backend | Trait implementations |
| Add security review workflow | Product | Static analysis, permissions |
| Implement revenue sharing | Product | 70/30 split, monthly payout |
| Create developer portal | Frontend | Submit, test, publish plugins |
| Add plugin installation UI | Frontend | One-click install to tenant |

#### Plugin Types

| Type | Interface | Use Case |
|------|-----------|----------|
| OCR | OcrBackend | Tesseract, PaddleOCR, TrOCR |
| Embedder | EmbeddingBackend | Custom models, fine-tuned |
| Reranker | RerankerBackend | Cross-encoder, custom |
| Tokenizer | TokenizerBackend | Domain-specific tokenization |
| Validator | ValidatorBackend | Schema validation, custom rules |
| Renderer | RendererBackend | PDF, HTML, Markdown output |

### Day 89-90: Multi-Region Active-Active

| Task | Owner | Acceptance Criteria |
|------|-------|---------------------|
| Deploy GKE clusters in 2+ regions | Platform | us-central1, us-east1, europe-west1 |
| Configure Global Load Balancer | Platform | HTTPS LB with geo-routing |
| Configure GeoDNS | Platform | Latency-based routing |
| Implement audit chain conflict resolution | Backend | Vector clocks, last-writer-wins |
| Implement cross-region replication | Platform | PG async replica, S3 CRR, Redis replica |
| Test failover between regions | Platform | < 30s failover |
| Add region affinity API | Backend | X-Hacienda-Region header |

### Day 91-100: Integration, Testing, Launch Prep

All gate criteria must pass:
- Usage-based billing with Stripe/Metronome
- Customer portal with all self-service features
- Webhook system with retry, DLQ, signatures
- Priority job queues with cron scheduling
- Plugin marketplace with WASM sandbox
- Multi-region active-active with <30s failover
- Beta customers onboarded successfully