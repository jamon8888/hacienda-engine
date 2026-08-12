# txtai-hacienda

**Status: Planned, not yet implemented.**

## What this would be

A txtai `Pipeline` that redacts text through `POST /v1/pii/redact` before it enters a
txtai `Embeddings` index — txtai users typically build a local embeddings index
directly from raw text, so this is the "redact at the door" pattern for that workflow
specifically, as distinct from `langchain-hacienda`'s full document loader.

## Reference

xberg's own `integrations/python/txtai/` for the extraction-only pipeline this would
add a redaction step in front of.
