# hacienda Dockerfile
# syntax = docker/dockerfile:1.7

# 1. PLANNER - cargo-chef for cache
FROM rust:1.78-slim AS planner
WORKDIR /app
RUN cargo install cargo-chef --locked
COPY . .
RUN cargo chef prepare --workspace --recipe-path recipe.json

# 2. BUILDER - Compile deps (cached)
FROM rust:1.78-slim AS builder
WORKDIR /app
RUN cargo install cargo-chef --locked
COPY --from=planner /app/recipe.json recipe.json
RUN cargo chef cook --release --workspace --recipe-path recipe.json

COPY . .
RUN cargo build --release --workspace --bin hacienda

# 3. RUNTIME - Minimal, non-root, distroless
FROM gcr.io/distroless/cc-debian12:nonroot AS runtime
WORKDIR /app

COPY --from=builder /app/target/release/hacienda /app/hacienda
COPY config/production.toml /app/config/production.toml

USER nonroot:nonroot

HEALTHCHECK --interval=30s --timeout=5s --start-period=10s --retries=3 \
  CMD ["/app/hacienda", "health-check"]

ENTRYPOINT ["/app/hacienda"]
CMD ["serve", "http", "--config", "/app/config/production.toml"]

LABEL org.opencontainers.image.title="hacienda" \
      org.opencontainers.image.description="xberg + PII/Compliance Distribution — GDPR/DORA/AI Act compliant document intelligence" \
      org.opencontainers.image.licenses="Apache-2.0" \
      org.opencontainers.image.source="https://github.com/jamon8888/hacienda-engine" \
      org.opencontainers.image.documentation="https://github.com/jamon8888/hacienda-engine"