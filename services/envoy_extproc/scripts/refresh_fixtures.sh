#!/usr/bin/env bash
# SLICE 5 — fixture refresh helper. Pulls the latest Envoy AI Gateway
# release tag, downloads the upstream reference YAML files, and diffs
# them against the committed fixtures. Operator confirms before
# overwriting; the README provenance block must be updated manually.
#
# Usage:
#   ./services/envoy_extproc/scripts/refresh_fixtures.sh           # interactive
#   ./services/envoy_extproc/scripts/refresh_fixtures.sh --tag v0.7.0  # explicit
#
# Requires `gh` CLI (authenticated against github.com/envoyproxy/ai-gateway).

set -euo pipefail

REPO="envoyproxy/ai-gateway"
FIXTURE_DIR="$(cd "$(dirname "$0")/../tests/fixtures/v0_6" && pwd)"

TAG=""
while [[ $# -gt 0 ]]; do
  case "$1" in
    --tag) TAG="$2"; shift 2 ;;
    *) echo "unknown arg: $1" >&2; exit 64 ;;
  esac
done

if [[ -z "$TAG" ]]; then
  if ! command -v gh >/dev/null 2>&1; then
    echo "error: gh CLI required to discover latest tag; pass --tag explicitly" >&2
    exit 65
  fi
  TAG=$(gh api "repos/${REPO}/releases/latest" --jq '.tag_name')
  echo "[refresh] latest upstream tag: ${TAG}"
fi

echo "[refresh] downloading from ${REPO}@${TAG} into ${FIXTURE_DIR}"

# Map upstream path → local fixture name. SLICE 5 deviation #1: upstream
# names the budget-shaped manifest `token_ratelimit.yaml`; we rename
# locally to `budget.yaml` to match the SLICE 5 spec naming convention.
declare -a MAPPINGS=(
  "examples/basic/basic.yaml:token_counting.yaml"
  "examples/token_ratelimit/token_ratelimit.yaml:budget.yaml"
)

TMP_DIR=$(mktemp -d)
trap 'rm -rf "$TMP_DIR"' EXIT

for entry in "${MAPPINGS[@]}"; do
  upstream="${entry%%:*}"
  local_name="${entry##*:}"
  echo "[refresh] -> ${upstream} → ${local_name}"
  if ! gh api "repos/${REPO}/contents/${upstream}?ref=${TAG}" \
        -H "Accept: application/vnd.github.v3.raw" > "${TMP_DIR}/${local_name}"; then
    echo "[refresh] FAIL: could not fetch ${upstream}@${TAG}" >&2
    exit 1
  fi
  if [[ -f "${FIXTURE_DIR}/${local_name}" ]]; then
    if diff -u "${FIXTURE_DIR}/${local_name}" "${TMP_DIR}/${local_name}" > "${TMP_DIR}/${local_name}.diff"; then
      echo "[refresh]    no change"
      continue
    fi
    echo "[refresh]    diff:"
    sed 's/^/    /' "${TMP_DIR}/${local_name}.diff"
    read -r -p "[refresh]    overwrite ${local_name}? [y/N] " ans
    if [[ "${ans}" == "y" || "${ans}" == "Y" ]]; then
      cp "${TMP_DIR}/${local_name}" "${FIXTURE_DIR}/${local_name}"
      echo "[refresh]    overwrote"
    else
      echo "[refresh]    skipped"
    fi
  else
    cp "${TMP_DIR}/${local_name}" "${FIXTURE_DIR}/${local_name}"
    echo "[refresh]    new fixture installed"
  fi
done

echo "[refresh] done."
echo "[refresh] REMINDER: update ${FIXTURE_DIR}/README.md provenance + refresh date."
