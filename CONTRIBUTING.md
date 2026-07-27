# Contributing to hacienda

Thank you for your interest in contributing! Whether you're fixing a typo, adding a feature, or improving documentation, every contribution makes a difference.

## First Time Contributing?

Welcome! Start by choosing an issue that matches your experience level:

- [Good first issue](https://github.com/jamon8888/hacienda-engine/issues?q=is%3Aissue+is%3Aopen+label%3A%22good+first+issue%22) — small, well-scoped tasks ideal for newcomers
- [Help wanted](https://github.com/jamon8888/hacienda-engine/issues?q=is%3Aissue+is%3Aopen+label%3A%22help+wanted%22) — tasks where we'd especially appreciate community help

Want to work on something bigger or propose a new feature? [Open a discussion](https://github.com/jamon8888/hacienda-engine/issues) with maintainers first.

## What Can I Contribute To?

hacienda is a polyglot project with many areas where you can help:

| Area                  | Description                                                                      |
| --------------------- | -------------------------------------------------------------------------------- |
| **Rust core**         | PII pipeline, redaction engine, compliance generators, audit chain, review queue |
| **Language bindings** | Python, TypeScript, Ruby, Go, Java, C#, PHP, Elixir, Dart, WASM                  |
| **Documentation**     | Guides, API references, examples, tutorials                                      |
| **Testing**           | Unit tests, E2E test fixtures, cross-language coverage                           |
| **CI/CD**             | Build pipelines, release automation, cross-platform testing                      |
| **Compliance**        | New regulation profiles, checklist items, report templates                       |

## Development Setup

### Prerequisites

```bash
# Install all toolchains
task setup
```

This installs: Rust (nightly), alef, poly, cargo-hack, and all language toolchains.

### Quick Start

```bash
# Format + lint
task check

# Run tests
task test

# Full test suite (all languages)
task test:all

# Regenerate bindings after API changes
task alef:generate

# Verify bindings match source
task alef:verify
```

## Workflow

### Quick Fixes (typos, small doc improvements)

1. Edit the file directly on GitHub
2. Submit a pull request — that's it!

### Larger Contributions (features, bug fixes, new bindings)

1. **Read the full [Contributing Guide](https://docs.hacienda.io/contributing/)** on our docs site
2. **Set up your development environment** (see above)
3. **Follow our workflow**: branch → code → test → PR
4. **Write tests** for new functionality
5. **Update documentation** if needed

## Code Style

We use automated tools for consistency:

- **Rust**: `poly fmt --fix .` (rustfmt + clippy)
- **Python**: `ruff format . && ruff check --fix .`
- **TypeScript/Node**: `npm run format` (prettier + oxlint)
- **Ruby**: `rubocop -a`
- **PHP**: `php-cs-fixer fix`
- **Go**: `gofmt -w .`
- **Java/C#**: `spotless:apply` / `dotnet format`
- **Elixir**: `mix format`
- **Dart**: `dart format .`
- **Zig**: `zig fmt`

Run `task fmt` to format everything.

## Testing

```bash
# Rust tests
cargo test -p hacienda -p hacienda-core

# All language tests
task test:all

# Single language E2E
task e2e:lang LANG=python

# With coverage
task test:cov
```

## Commit Messages

We use [Conventional Commits](https://www.conventionalcommits.org/):

```
feat: add PCI-DSS redaction profile
fix: fix audit chain verification on empty chain
docs: update API reference for /v1/pii/redact
test: add E2E test for batch redaction
chore: update dependencies
```

## Pull Request Checklist

Before submitting, ensure:

- [ ] `task check` passes (format + lint)
- [ ] `task test` passes
- [ ] Tests added for new functionality
- [ ] Documentation updated if API changed
- [ ] CHANGELOG.md updated (if applicable)
- [ ] Conventional commit message format

## Code Review

We use GitHub's review system. Reviews focus on:

1. **Correctness** — Does it solve the problem?
2. **Maintainability** — Is it clean, well-structured?
3. **Tests** — Are they comprehensive, not just mocks?
4. **Security** — No secrets, proper validation, safe defaults
5. **Performance** — No obvious regressions
6. **Documentation** — Clear, accurate, up-to-date

## Community

- **Discord**: [Join our community](https://discord.gg/hacienda)
- **Issues**: [GitHub Issues](https://github.com/jamon8888/hacienda-engine/issues)
- **Discussions**: [GitHub Discussions](https://github.com/jamon8888/hacienda-engine/discussions)

## License

By contributing, you agree that your contributions will be licensed under the Apache-2.0 license.
