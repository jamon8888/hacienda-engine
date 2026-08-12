# @hacienda-engine/n8n-nodes-hacienda

**Status: Planned, not yet implemented.** No `build`/`check`/`test:unit` scripts are
declared yet — this package is intentionally invisible to `turbo run build` and the
other root-level workspace tasks until real nodes exist.

## What this would be

n8n community nodes, compliance/ops-workflow shaped rather than RAG-shaped:

- **"Hacienda: Extract & Redact"** — `POST /v1/documents`, drop a file in, get
  redacted markdown + entities out, wire into any downstream node.
- **"Hacienda: PII Scan"** — `POST /v1/pii/scan`, a pre-send guard node (e.g. before
  a "Send Email" node) that fails the workflow if PII is detected in the payload.
- **"Hacienda: Audit Export"** — `POST /v1/audit/export`, scheduled export of the
  audit chain to wherever a compliance team's workflow already lands artifacts.
- **"Hacienda: Compliance Report"** — `GET /v1/compliance/dpia` /
  `GET /v1/compliance/report` on a schedule.

n8n's low-code, ops-team audience is a better fit for "audit export on a cron" and
"block this workflow if PII leaks" than for RAG retrieval — that's `langchain-hacienda`
/ the future `llama-index-readers-hacienda`'s job.

## Reference

xberg's own `integrations/node/n8n-nodes-xberg/` for the extraction-only node this
would sit alongside (`nodes/`, `gulpfile.js`, n8n's node packaging conventions).
