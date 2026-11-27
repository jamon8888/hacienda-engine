# CI build system

`ci-build-all.yaml` is the single build gate for hacienda-engine. It proves that
every artifact the project can ship still compiles on each pull request and push
to `main`, and it is the place to add new build surfaces as the engine grows.

## What it covers

| Surface | Jobs | Source of truth for the command |
| --- | --- | --- |
| Rust workspace (debug + release) + CLI | `rust-debug`, `rust-release`, `cli-release` | `Taskfile.yml` `build:*` |
| WASM package | `wasm` | `Taskfile.yml` `build:wasm` + `[profile.wasm-release]` in `Cargo.toml` |
| Language bindings (15 languages) | `bindings-*` | `alef.toml` + `publish.yaml` matrix |
| Python / TypeScript SDKs | `sdk-python`, `sdk-typescript` | `ci-sdk-python.yaml`, `ci-sdk-typescript.yaml` |
| Docker images | `docker-images` | `ci-docker.yaml`, `publish-docker.yaml` |
| Integration packages | `integrations` | `ci-integrations.yaml.disabled` |
| Tauri desktop bundles | `desktop` (manual runs only) | delegated to `build-desktop.yaml` |

Static checks and tests stay in their dedicated workflows (`ci-lint.yaml`,
`ci-rust.yaml`, `ci-postgres.yaml`, `ci-e2e.yaml`, `ci-security.yaml`). Releases
stay in `publish.yaml`, `publish-docker.yaml`, `publish-sdk.yaml`.

## Layout

- `.github/workflows/ci-build-all.yaml` — the matrix. Each `include` entry
  declares a `name`, a `runner`, a `cmd`, and optionally `targets`, `alef`,
  `python`, `node`, `workdir`, and `artifact`.
- `.github/actions/setup-build-env/action.yaml` — composite action that
  installs the pinned Rust nightly, protoc, caching, and (when requested) alef.
  All build jobs call this, so toolchain drift is fixed in one place.
- `.github/workflows/build-desktop.yaml` — now also callable via
  `workflow_call`, so the aggregate workflow can delegate desktop builds
  without copying its signing/ONNX/lipo steps.
- `scripts/ci/setup-self-hosted-runner.sh` — installs the `actions/runner`
  agent on a machine you control and registers it with labels from
  `.github/actionlint.yaml`.

## Adding a new build target

1. Add one `include` block to the matrix in `ci-build-all.yaml`:

   ```yaml
   - name: bindings-my-new-language   # appears as "build / bindings-my-new-language"
     runner: ubuntu-24.04             # or macos-15 / windows-2022
     alef: "true"                     # only if the command needs alef
     cmd: alef build --lang my_lang --release
     artifact: packages/my_lang/dist/**   # optional
   ```

2. If the target needs extra setup that the composite action does not already
   provide, add a guarded step in the `build` job using a matrix field (see the
   existing `matrix.python`, `matrix.node`, and `matrix.name == 'wasm'` steps).
   Keep toolchain/protoc/cache changes inside the composite action.

3. If it needs a language-specific toolchain that hosted runners do not ship,
   either add an install step or register a self-hosted runner with the right
   label (below).

4. Update the table above.

The workflow is also reusable (`workflow_call`), so a future release workflow
can run the entire build matrix with:

```yaml
jobs:
  build-all:
    uses: ./.github/workflows/ci-build-all.yaml
    with:
      upload_artifacts: true
```

## Runner profiles

By default the matrix uses GitHub-hosted runners (`ubuntu-24.04`, `macos-15`,
`windows-2022`). These are `actions/runner` agents operated by GitHub, so no
runner setup is required.

To move **Linux** builds onto your own machines without editing the workflow,
set a repository variable named `CI_LINUX_RUNNER` to a `runs-on` label list,
for example:

```text
self-hosted, runner-medium
```

The job-level expression in `ci-build-all.yaml` substitutes that variable for
every `ubuntu-*` matrix entry. macOS and Windows entries stay hosted until
equivalent self-hosted agents are registered. Available labels are declared in
`.github/actionlint.yaml`:

| Label | Intended use |
| --- | --- |
| `runner-small` | light builds, validation |
| `runner-medium`, `runner-medium-arm64` | normal Rust/binding builds |
| `runner-large`, `runner-large-arm64` | release, LTO, full-matrix builds |
| `runner-large-spot`, `runner-medium-arm64-spot` | cost-tolerant interruptible work |
| `runner-gpu-l4` | GPU-backed model/embedding builds |

## Registering a self-hosted runner

1. In GitHub, go to **Settings → Actions → Runners → New self-hosted runner**
   (repository or organisation scope) and copy the registration token. It is
   short-lived; generate it just before running the script.

2. On the build machine:

   ```bash
   export RUNNER_TOKEN="<token from GitHub>"
   export RUNNER_LABELS="self-hosted,runner-medium"
   ./scripts/ci/setup-self-hosted-runner.sh
   ```

   Useful overrides: `RUNNER_SCOPE=org RUNNER_ORG=...`, `RUNNER_REPO=owner/repo`,
   `RUNNER_VERSION=v2.338.2` (pin rather than tracking `latest` on production
   agents), `RUNNER_DIR=/opt/actions-runner`, `RUNNER_USER=jamin`, `GH_TOKEN=...`
   (raises the API rate limit during version lookup).

3. Confirm the runner is **Idle** in GitHub's runner list, then set
   `CI_LINUX_RUNNER` to a label list that includes one of its labels.

The script downloads the official `actions/runner` release for the host's OS
and architecture, verifies its SHA-256 against the digest published on the
release, configures it with `config.sh --replace` (idempotent), and installs a
systemd/launchd service via `svc.sh`.

> Self-hosted runners execute arbitrary workflow code from anyone with write
> access to the repository. Only register trusted, isolated machines, and keep
> `permissions: contents: read` on workflows that do not need write access.

## Artifacts and caching

Artifacts are uploaded only for manual (`workflow_dispatch`) or reusable
(`workflow_call`) runs that set `upload_artifacts: true`, keeping PR runs cheap.
Rust compilation is cached by `Swatinem/rust-cache@v2` with a per-target
`shared-key`, so adding a target does not invalidate existing caches.
