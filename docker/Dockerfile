# hacienda Dockerfile
# syntax = docker/dockerfile:1.7

# 1. PLANNER - cargo-chef for cache
# 1.78 (this image's prior pin) can no longer `cargo install cargo-chef`: current
# cargo-chef releases declare `edition = "2024"` (needs 1.85+), and cargo-chef's own
# dependency graph (cargo-platform, cargo_metadata, guppy, target-spec) needs 1.86-1.88
# on top of that — 1.85 alone still isn't enough, confirmed by the same "rustc N.NN is
# not supported by the following packages" error one version pin at a time. 1.90 gives
# headroom past the highest individual requirement seen (cargo-platform's 1.88) rather
# than re-chasing this one point release at a time again next time a dependency bumps
# its own MSRV. Caught by ci-docker.yaml's validate-main job actually building the
# image for the first time (previously `docker buildx build --check ... || true` never
# ran a real build).
FROM rust:1.90-slim AS planner
WORKDIR /app
RUN cargo install cargo-chef --locked
COPY . .
# No `--workspace` flag: current cargo-chef's `prepare`/`cook` don't accept one (`error:
# unexpected argument '--workspace' found`) — they operate on the whole workspace by
# default from whatever manifest `recipe.json` is generated against, unlike plain
# `cargo build`, which does need `--workspace` to not default to just the root package.
RUN cargo chef prepare --recipe-path recipe.json

# 2. BUILDER - Compile deps (cached)
FROM rust:1.90-slim AS builder
WORKDIR /app
RUN cargo install cargo-chef --locked
COPY --from=planner /app/recipe.json recipe.json
RUN cargo chef cook --release --recipe-path recipe.json

COPY . .
RUN cargo build --release --workspace --bin hacienda

# 3. RUNTIME - Minimal, non-root, distroless
FROM gcr.io/distroless/cc-debian12:nonroot AS runtime
WORKDIR /app

COPY --from=builder /app/target/release/hacienda /app/hacienda
COPY config/production.toml /app/config/production.toml

USER nonroot:nonroot

# No Docker-level HEALTHCHECK: the distroless runtime has no shell and no curl to run a
# probe command with, and there is no `hacienda health-check` subcommand — this crate's
# CLI surface is deliberately closed rather than carrying stubbed commands (see
# hacienda-cli/src/cli.rs's module doc). `GET /health` (public, no auth required) already
# exists and works — point your orchestrator's own probe (Kubernetes `httpGet`, ECS
# health check, etc.) at it instead.
ENTRYPOINT ["/app/hacienda"]
CMD ["serve", "--config", "/app/config/production.toml", "--bind", "0.0.0.0:8787"]

LABEL org.opencontainers.image.title="hacienda" \
      org.opencontainers.image.description="xberg + PII/Compliance Distribution — GDPR/DORA/AI Act compliant document intelligence" \
      org.opencontainers.image.licenses="Apache-2.0" \
      org.opencontainers.image.source="https://github.com/jamon8888/hacienda-engine" \
      org.opencontainers.image.documentation="https://github.com/jamon8888/hacienda-engine"