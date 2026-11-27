#!/usr/bin/env bash
# Install and register a GitHub Actions self-hosted runner from
# https://github.com/actions/runner — the same runner binary that powers
# GitHub-hosted runners — for the hacienda-engine CI build fleet.
#
# The script is idempotent: re-running it re-registers the runner in place
# (config.sh --replace) and restarts the service. It verifies the downloaded
# archive against the SHA-256 digest published on the GitHub release before
# executing anything from it.
#
# Required environment:
#   RUNNER_TOKEN   Registration token (repo or org settings -> Actions -> Runners)
#
# Common optional environment:
#   RUNNER_SCOPE   repo (default) or org
#   RUNNER_REPO    owner/repo (default: derived from `git remote get-url origin`)
#   RUNNER_ORG     organisation, required when RUNNER_SCOPE=org
#   RUNNER_VERSION exact tag such as v2.338.2, or `latest` (default)
#   RUNNER_LABELS  comma-separated labels (default: labels from .github/actionlint.yaml)
#   RUNNER_NAME    runner name (default: hostname)
#   RUNNER_DIR     install directory (default: /opt/actions-runner, or $HOME/actions-runner)
#   RUNNER_USER    OS user the service runs as (default: current user)
#   GH_TOKEN       optional, raises the GitHub API rate limit for version lookup
#
# Usage:
#   RUNNER_TOKEN=... ./scripts/ci/setup-self-hosted-runner.sh
#   ./scripts/ci/setup-self-hosted-runner.sh --help
set -euo pipefail

usage() {
  sed -n '2,32p' "$0" | sed 's/^# \{0,1\}//'
}

if [[ "${1:-}" == "--help" || "${1:-}" == "-h" ]]; then
  usage
  exit 0
fi

die() { echo "error: $*" >&2; exit 1; }
log() { echo "==> $*"; }

command -v curl >/dev/null || die "curl is required"
command -v tar >/dev/null || die "tar is required"
command -v python3 >/dev/null || die "python3 is required to parse the release metadata"

: "${RUNNER_TOKEN:?RUNNER_TOKEN is required (see --help)}"

RUNNER_SCOPE="${RUNNER_SCOPE:-repo}"
RUNNER_VERSION="${RUNNER_VERSION:-latest}"
RUNNER_LABELS="${RUNNER_LABELS:-self-hosted,runner-medium}"
RUNNER_NAME="${RUNNER_NAME:-$(hostname)}"
RUNNER_WORKDIR="${RUNNER_WORKDIR:-_work}"
RUNNER_USER="${RUNNER_USER:-$(id -un)}"

if [[ "$RUNNER_SCOPE" == "repo" ]]; then
  RUNNER_REPO="${RUNNER_REPO:-$(git remote get-url origin 2>/dev/null | sed -E 's#.*github\.com[:/]([^/]+/[^/.]+)(\.git)?$#\1#')}"
  [[ "$RUNNER_REPO" == */* ]] || die "could not derive owner/repo; set RUNNER_REPO explicitly"
  REGISTRATION_URL="https://github.com/${RUNNER_REPO}"
elif [[ "$RUNNER_SCOPE" == "org" ]]; then
  : "${RUNNER_ORG:?RUNNER_ORG is required when RUNNER_SCOPE=org}"
  REGISTRATION_URL="https://github.com/${RUNNER_ORG}"
else
  die "RUNNER_SCOPE must be 'repo' or 'org'"
fi

if [[ -z "${RUNNER_DIR:-}" ]]; then
  if [[ "$(id -u)" -eq 0 ]]; then
    RUNNER_DIR="/opt/actions-runner"
  else
    RUNNER_DIR="$HOME/actions-runner"
  fi
fi

# Map the current host to the asset naming used by actions/runner releases.
case "$(uname -s)" in
  Linux)  runner_os="linux" ;;
  Darwin) runner_os="osx" ;;
  *) die "unsupported OS: $(uname -s) (Windows runners use svc.cmd, not this script)" ;;
esac
case "$(uname -m)" in
  x86_64|amd64) runner_arch="x64" ;;
  aarch64|arm64) runner_arch="arm64" ;;
  armv7l|armv6l) runner_arch="arm" ;;
  *) die "unsupported architecture: $(uname -m)" ;;
esac

asset_name="actions-runner-${runner_os}-${runner_arch}"
log "Resolving actions/runner ${RUNNER_VERSION} for ${runner_os}/${runner_arch}"

api_headers=(-H "Accept: application/vnd.github+json" -H "X-GitHub-Api-Version: 2022-11-28")
if [[ -n "${GH_TOKEN:-}" ]]; then
  api_headers+=(-H "Authorization: Bearer ${GH_TOKEN}")
fi

if [[ "$RUNNER_VERSION" == "latest" ]]; then
  release_json="$(curl -fsSL --retry 3 "${api_headers[@]}" \
    https://api.github.com/repos/actions/runner/releases/latest)"
else
  release_json="$(curl -fsSL --retry 3 "${api_headers[@]}" \
    "https://api.github.com/repos/actions/runner/releases/tags/${RUNNER_VERSION}")"
fi

# Extract this platform's download URL and the release's published SHA-256.
read -r download_url expected_sha < <(printf '%s' "$release_json" | python3 -c '
import json, sys
release = json.load(sys.stdin)
prefix = sys.argv[1]
for asset in release["assets"]:
    if asset["name"].startswith(prefix + "-") and asset["name"].endswith((".tar.gz", ".zip")):
        print(asset["browser_download_url"], asset["digest"].split(":", 1)[-1])
        break
else:
    raise SystemExit("no release asset found for " + prefix)
' "$asset_name")

[[ -n "$download_url" && -n "$expected_sha" ]] || die "could not resolve release asset metadata"
resolved_version="$(basename "$download_url" | sed -E "s/^${asset_name}-(.*)\.(tar\.gz|zip)$/\1/")"
log "Using actions/runner v${resolved_version}"

archive="${asset_name}-${resolved_version}.tar.gz"
tmp_dir="$(mktemp -d)"
trap 'rm -rf "$tmp_dir"' EXIT

log "Downloading to $tmp_dir/$archive"
curl -fL --retry 3 -o "$tmp_dir/$archive" "$download_url"

actual_sha="$(sha256sum "$tmp_dir/$archive" | awk '{print $1}')"
[[ "$actual_sha" == "$expected_sha" ]] \
  || die "SHA-256 mismatch: expected $expected_sha, got $actual_sha"
log "Checksum verified"

mkdir -p "$RUNNER_DIR"
tar xzf "$tmp_dir/$archive" -C "$RUNNER_DIR"

log "Registering runner '$RUNNER_NAME' with $REGISTRATION_URL"
(cd "$RUNNER_DIR" && ./config.sh \
  --url "$REGISTRATION_URL" \
  --token "$RUNNER_TOKEN" \
  --name "$RUNNER_NAME" \
  --labels "$RUNNER_LABELS" \
  --work "$RUNNER_WORKDIR" \
  --unattended \
  --replace)

if [[ "$(uname -s)" == "Linux" ]]; then
  log "Installing and starting systemd service as '$RUNNER_USER'"
  (cd "$RUNNER_DIR" && sudo ./svc.sh install "$RUNNER_USER")
  (cd "$RUNNER_DIR" && sudo ./svc.sh start)
else
  log "Installing and starting launchd service as '$RUNNER_USER'"
  (cd "$RUNNER_DIR" && sudo ./svc.sh install "$RUNNER_USER")
  (cd "$RUNNER_DIR" && sudo ./svc.sh start)
fi

log "Runner status:"
(cd "$RUNNER_DIR" && sudo ./svc.sh status) || true
log "Done. Runner '$RUNNER_NAME' labels: $RUNNER_LABELS"
