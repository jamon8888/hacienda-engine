# hacienda Dockerfile
# syntax = docker/dockerfile:1.7

# 1. PLANNER - cargo-chef for cache
# 1.78 (this image's original pin) could no longer `cargo install cargo-chef`: current
# cargo-chef releases declare `edition = "2024"` (needs 1.85+), and cargo-chef's own
# dependency graph (cargo-platform, cargo_metadata, guppy, target-spec) needs 1.86-1.88
# on top of that. 1.90 (the next pin) then failed too, one layer deeper: `xberg`/
# `xberg-gliner` (this workspace's real dependency, resolved once `cargo chef prepare`
# reaches actual crate metadata) require rustc 1.91, caught by ci-docker.yaml's
# validate-main job actually building the image for the first time (previously
# `docker buildx build --check ... || true` never ran a real build). 1.94 gives
# headroom past the highest individual requirement seen so far, matching the
# nightly-2026-08-06 toolchain ci-rust.yaml et al. already build and test against
# successfully, rather than re-chasing this one point release at a time again next
# time a dependency bumps its own MSRV. Explicit `-bookworm` suffix (not the bare
# `rust:1.94-slim`, which floats to whatever Debian release is current — trixie as of
# this pin): the runtime stage below is `distroless/cc-debian12` (bookworm, glibc
# 2.36), and a binary built against a newer glibc than the runtime ships fails to
# even start (`GLIBC_2.38' not found`) rather than failing to build — caught by
# validate-main's smoke test actually running the built image, not just building it.
FROM rust:1.94-slim-bookworm AS planner
WORKDIR /app
RUN cargo install cargo-chef --locked
COPY . .
# No `--workspace` flag: current cargo-chef's `prepare`/`cook` don't accept one (`error:
# unexpected argument '--workspace' found`) — they operate on the whole workspace by
# default from whatever manifest `recipe.json` is generated against, unlike plain
# `cargo build`, which does need `--workspace` to not default to just the root package.
RUN cargo chef prepare --recipe-path recipe.json

# 2. BUILDER - Compile deps (cached)
FROM rust:1.94-slim-bookworm AS builder
WORKDIR /app
RUN cargo install cargo-chef --locked
COPY --from=planner /app/recipe.json recipe.json
# Root Cargo.toml's `[patch.crates-io]` path dependencies have to exist before
# `cargo chef cook`, not just before the `COPY . .` below: unlike the registry deps
# `recipe.json` otherwise covers, a `path =` patch is resolved from the real source on
# disk, at every cargo invocation that touches the dependency graph — `cargo chef cook`
# is exactly that, run here specifically to warm the dependency-build cache before the
# full source lands. Without this, cook fails immediately with "failed to read
# vendor/candle-core-0.11.0-wasm-fix/Cargo.toml: No such file or directory". Only these
# two, not the whole `vendor/` tree: `vendor/xberg-wasm-ner-fix` is a 39MB compiled
# WASM binary for the browser frontend (apps/hacienda-studio), irrelevant to this
# Dockerfile's Rust server build and not worth the copy weight here.
COPY vendor/candle-core-0.11.0-wasm-fix/ vendor/candle-core-0.11.0-wasm-fix/
COPY vendor/candle-transformers-0.11.0-wasm-fix/ vendor/candle-transformers-0.11.0-wasm-fix/
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