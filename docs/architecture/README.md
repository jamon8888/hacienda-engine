# Architecture and product analysis archive

These documents record decisions, investigations, and plans made at the dates in their filenames. They are not a live specification. For current shipped behavior, use the [feature and status catalog](../features-and-roadmap.md), then confirm the owning code and configuration.

## Current architecture references

| Document | Subject | Use |
| --- | --- | --- |
| [`2026-11-20-email-rag-store-topology.md`](2026-11-20-email-rag-store-topology.md) | Shared RAG store topology for email and documents | Current design rationale; validate implementation before changing it |
| [`2026-08-20-wasm-worker-performance-plan.md`](2026-08-20-wasm-worker-performance-plan.md) | Studio/WASM worker performance | Historical implementation plan and measurements |

## Product analysis archive

[`2026-07-31-analyse-architecture-et-pistes-produit.md`](2026-07-31-analyse-architecture-et-pistes-produit.md) is an early product/architecture analysis. Its status descriptions and recommendations have aged; use it for context only.

## Specs and implementation records

Design specs and implementation plans live under `superpowers/specs/` and `superpowers/plans/`. Before using one as a task definition:

1. Check its date, referenced PRs, and stated assumptions.
2. Check the current implementation and open/merged PR status.
3. Update the feature catalog if the investigation changes a user-facing status or known limitation.

Do not infer that a planned capability exists because a spec describes its intended API or architecture. Do not infer that an old “not implemented” finding remains true without checking the code.
