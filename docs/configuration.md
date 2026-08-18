# hacienda Configuration

This document describes the real, current configuration surface — every section maps
directly to a `#[serde(deny_unknown_fields)]` struct in `hacienda-core`, so a typo'd key
fails to load instead of silently doing nothing (see `hacienda-core/src/config.rs`'s
module doc for why that trade was made). `hacienda config show` prints the *effective*
configuration for any given file, with the provenance of each value — the fastest way to
check what a config actually resolves to, rather than reading source.

## Full example (`hacienda.toml`)

```toml
[extraction]
# xberg's own ExtractionConfig — output format, concurrency, etc. See xberg's docs.

[pii]
regex_first = true
model_threshold_default = 0.5
merge_overlap_threshold = 0.5
concurrency = 1

[pii.redaction]
mode = "mask"  # mask | hash | pseudonymize | remove | custom
preserve_format = false

[pii.audit]
enabled = true

[pii.vertical]
id = "business_law"
labels = ["ContractParty", "Court", "Statute"]

[compliance]
model_name = "hacienda-pii-v1"
enabled_reports = ["dpia", "model_card", "dora", "checklist"]

[review]
confidence_threshold = 0.5
deadline_hours = 24

[glossary]
enabled = true
link_style = "markdown"  # markdown | html | wiki
min_confidence = 0.5
min_count = 1

[auth]
enabled = false  # required (with a non-loopback bind) to run `hacienda serve` off localhost
resolver = "memory"  # memory | dev

[[auth.static_tokens]]
id = "example"
token = "replace-me"
principal_id = "example"
capabilities = ["documents:process"]
```

Every top-level section except `extraction` and `auth` is optional — omit a section to
skip that stage entirely (`pii` absent means extraction runs with no PII detection at
all, not "detection disabled but still reported"). `auth` is always present; its default
(`enabled = false`, `resolver = "dev"`) is correct for the CLI and a desktop app, where
the process boundary *is* the trust boundary — see `[auth]` below for when it must be
turned on.

### `[auth]` — required reading before running `hacienda serve` off localhost

`hacienda serve` refuses to bind any address that isn't loopback (`127.0.0.0/8`, `::1`)
unless `auth.enabled = true` — there is no override flag, because "reachable from the
network, no authentication" has no legitimate use on a product built around redacting
personal data. See `docker/Dockerfile`/`config/production.toml` for a working example
(the Docker image binds `0.0.0.0`, so its config sets `auth.enabled = true`).

## Environment variables

| Variable | Description |
| --- | --- |
| `HACIENDA_PSEUDONYM_ACTIVE_KEY` | ID of the pseudonymisation key new tokens are minted under. Required for `mode = "pseudonymize"`. |
| `HACIENDA_PSEUDONYM_KEY_<ID>` | Hex-encoded key material for pseudonymisation key `<ID>` (uppercased). |
| `RUST_LOG` | Standard `tracing`/`env_logger`-style log filter, e.g. `info` or `hacienda=debug`. |

There is no `HACIENDA_CONFIG`/`HACIENDA_JWT_SECRET`/`HACIENDA_FPE_KEY`/`HACIENDA_MODELS_DIR`
env var — config discovery is `--config <path>` (a global CLI flag) or the platform config
directory (see `hacienda-cli/src/config.rs`), and there is no JWT-based auth resolver or
FPE mode today.

## CLI reference

The real commands, from `hacienda-cli/src/cli.rs` (deliberately closed — a subcommand that
doesn't exist is absent from `--help`, not present-and-stubbed):

```bash
hacienda extract <inputs...> [--mode mask|hash|pseudonymize|remove] [--threshold 0.7] \
  [--model-dir ./model] [--lora-dir ./lora] [--format json|text] [--audit-out ./audit] \
  [--vault ./vault] [--glossary-out ./glossary] [--concurrency 4]

hacienda scan <inputs...> [--threshold 0.7] [--format json|text] [--glossary-out ./glossary]

hacienda config show [--format json|text]

hacienda serve [--bind 127.0.0.1:8787]

hacienda pii reveal <TOKEN> [--format json|text]

hacienda audit verify <DIR> [--format json|text]
```

`--config <path>` / `--config-json <json>` are global flags, valid before or after the
subcommand.

## REST API

```bash
curl -X POST http://localhost:8787/v1/pii/redact \
  -H "Authorization: Bearer <token>" \
  -H "Content-Type: application/json" \
  -d '{"text": "Contact john@example.com"}'
```

```json
{
  "redacted_text": "Contact [EMAIL]",
  "entity_count": 1,
  "processing_time_ms": 4,
  "audit_chain_tip": "5e884898da28..."
}
```

See `hacienda-api/README.md` for the audit endpoints and `GET /openapi.json` (public, no
auth) for the full schema — 44 operations across 14 tags.

## Docker

```bash
docker build -f docker/Dockerfile -t hacienda .
docker run --rm -p 8787:8787 \
  -v $(pwd)/config/production.toml:/app/config/production.toml:ro \
  hacienda
curl http://localhost:8787/health
```

`config/production.toml` in this repo is a minimal starting point (auth enabled, one
placeholder static token) — replace the token before deploying, and add whichever
sections above your deployment needs. There is no Docker-level `HEALTHCHECK` (the
distroless runtime has no shell to run a probe command with) — point your orchestrator's
own health check at `GET /health`.

## What this document does not cover (yet)

Real-time metrics (`/metrics`), a `[server]`/`[security]`/`[observability]` config
surface, and a Kubernetes manifest are not implemented today. `HaciendaConfig` itself
carries `#[serde(deny_unknown_fields)]`, so any of those sections in a config file fails
to load rather than being silently ignored — a previous version of this document
described them anyway, which is exactly the "control looks configured, isn't" failure
shape `deny_unknown_fields` exists to prevent, just one layer up (in documentation
instead of in the parser).
