# Phase 6 Implementation: Developer Experience (Weeks 13-14)

> **Goal**: Complete SDK coverage and developer tooling
> **Duration**: 2 weeks (10 working days)
> **Team**: 2 Backend Engineers
> **Prerequisites**: Can run in parallel with Phases 3-5

---

## Week 13: API Versioning & FFI Bindings

### Day 61: API Versioning Policy

| Task | Owner | Acceptance Criteria |
|------|-------|---------------------|
| Implement URL versioning (/v1/, /v2/) | Backend | Router prefixes all routes |
| Add Sunset header for deprecated versions | Backend | RFC 8594 compliant (Sunset header) |
| Add Deprecation header | Backend | Links to migration guide |
| Implement version negotiation (Accept-Version) | Backend | Vary: Accept-Version |
| Create migration guide template | Backend | docs/api/migration-v1-v2.md |
| Document 12-month support policy | Backend | Version lifecycle doc |

### Day 62-64: Native FFI Bindings (15 Languages)

| Task | Owner | Acceptance Criteria |
|------|-------|---------------------|
| Create alef source files (6 missing) | Backend | hacienda/src/{cli,api,prelude,config}.rs etc |
| Configure alef.toml for all 15 languages | Backend | python, node, ruby, php, go, java, csharp, elixir, dart, kotlin, swift, zig, ffi, jni, wasm |
| Generate Python bindings | Backend | cargo alef generate --lang python |
| Generate Node.js bindings | Backend | cargo alef generate --lang node |
| Generate Go bindings | Backend | cargo alef generate --lang go |
| Generate Java bindings | Backend | cargo alef generate --lang java |
| Generate C# bindings | Backend | cargo alef generate --lang csharp |
| Generate remaining 10 languages | Backend | cargo alef generate (all configured) |
| Verify all bindings compile | Backend | cargo alef build --all-targets |
| Write integration tests per language | Backend | cargo alef test --all-languages |

#### Missing alef Source Files

```rust
// hacienda/src/cli.rs - CLI command definitions for bindings
// hacienda/src/api.rs - API types for bindings
// hacienda/src/prelude.rs - Re-exports for bindings
// hacienda/src/config.rs - Config types for bindings
// hacienda-core/src/pii/profiles.rs - PII profiles for bindings
// hacienda-core/src/pii/xberg_integration.rs - xberg integration for bindings
```

### Day 65: SDK Publishing Automation

| Task | Owner | Acceptance Criteria |
|------|-------|---------------------|
| Configure trusted publishing (PyPI) | Platform | OIDC token, no passwords |
| Configure trusted publishing (npm) | Platform | OIDC token, provenance |
| Enable publish-sdk.yaml workflow | Platform | Runs on tag push |
| Add version sync from Cargo.toml | Platform | alef sync-versions |
| Test publish to TestPyPI/TestNPM | Platform | Dry-run successful |

## Week 14: API Docs, CLI Distribution, Integration Tests

### Day 66: Interactive API Documentation

| Task | Owner | Acceptance Criteria |
|------|-------|---------------------|
| Deploy Swagger UI at /docs | Backend | utoipa/swagger-ui integration |
| Deploy Redoc at /redoc | Backend | Alternative docs |
| Add authentication demo | Backend | Bearer token input |
| Add code samples per language | Backend | Python, TypeScript, Rust, curl |
| Add try-it-out functionality | Backend | Live API calls from docs |

### Day 67: CLI Distribution

| Task | Owner | Acceptance Criteria |
|------|-------|---------------------|
| Build release binaries (linux/mac/windows) | Platform | GitHub Actions cross-compile |
| Publish to crates.io (cargo install hacienda) | Platform | cargo publish |
| Create Homebrew tap | Platform | brew install jamon8888/tap/hacienda |
| Publish GHCR binary releases | Platform | ghcr.io/jamon8888/hacienda-cli |
| Add shell completions | Backend | bash, zsh, fish, powershell, elvish |

### Day 68-69: Integration Test Suite

| Task | Owner | Acceptance Criteria |
|------|-------|---------------------|
| Enable ci-integrations.yaml | Platform | Testcontainers PG/S3/Redis |
| Write E2E: extract -> redact -> audit | Backend | Full pipeline test |
| Write E2E: PII scan + reveal | Backend | Pseudonym round-trip |
| Write E2E: Review queue flow | Backend | Create, assign, decide |
| Write E2E: RAG ingest + query | Backend | Vector search test |
| Write E2E: Multi-tenant isolation | Backend | Cross-tenant 403 |
| Configure parallel test execution | Platform | --test-threads=4 |

### Day 70: Validation

```bash
# 1. Test API versioning
curl -H "Accept-Version: v1" https://api.example.com/v1/documents
curl -H "Accept-Version: v2" https://api.example.com/v2/documents

# 2. Verify all 15 bindings generate
cargo alef generate

# 3. Verify all bindings build
cargo alef build --all-targets

# 4. Verify all bindings test
cargo alef test

# 5. Check Swagger UI at /docs
# 6. Check Redoc at /redoc

# 7. Test CLI install
cargo install hacienda --features ner-candle
hacienda --help

# 8. Run integration tests
cargo test --workspace --features postgres,integration

# 9. All gate criteria pass
✅ API versioning with Sunset headers (RFC 8594)
✅ 15 FFI bindings generated, build, test
✅ SDK publishing to PyPI/npm automated
✅ Swagger UI + Redoc at /docs, /redoc
✅ CLI on crates.io, Homebrew, GHCR
✅ Integration test suite passing
```