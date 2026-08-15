# spring-ai-hacienda

**Status: Planned, not yet implemented.**

## What this would be

A Spring AI `DocumentReader` (and, longer term, a `VectorStore` proxying
`/v1/rag/collections/{name}/retrieve`) calling `POST /v1/documents` — same shape as
`../../python/langchain`'s `HaciendaLoader`, for JVM shops running Spring AI. There is
no `hacienda-sdk` equivalent for Java yet (only Python and TypeScript exist under
`sdks/`), so this integration would need to either generate a Java client from
`GET /openapi.json` first (e.g. via `openapi-generator`) or call the REST API directly
with Spring's own `RestClient`.

## Reference

xberg's own `integrations/java/spring-ai/` for the extraction-only reader this would
parallel, and its `pom.xml` / `checkstyle.xml` for Java packaging conventions.
