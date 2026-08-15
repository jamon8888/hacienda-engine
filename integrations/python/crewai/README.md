# crewai-hacienda

**Status: Planned, not yet implemented.**

## What this would be

CrewAI tools (not a loader — CrewAI agents call tools) wrapping:

- `HaciendaExtractTool` — `POST /v1/documents`, redacted extraction for a crew agent
  to read a document without ever seeing raw PII in its context.
- `HaciendaScanTool` — `POST /v1/pii/scan`, lets an agent check whether a piece of
  text it's about to output/send contains PII before doing so.
- `HaciendaRevealTool` — `POST /v1/pii/reveal`, gated behind the human-review workflow
  (`POST /v1/review/{id}/decide`) rather than a direct call, so a crew can request
  reveal but not silently grant itself one.

This is the integration most worth building for an "agent handles sensitive documents
autonomously but a human stays in the loop for de-anonymization" workflow — CrewAI's
tool-calling model maps directly onto `/v1/review`'s decide/approve step.

## Reference

xberg's own `integrations/python/crewai/` for the extraction-only tool this would
extend with the scan/redact/reveal/review tools above.
