#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'USAGE'
Usage: scripts/release/check-release-bundle.sh DIR

Validate a local SpendGuard GA release bundle.
USAGE
}

if [[ $# -ne 1 || "${1:-}" == "-h" || "${1:-}" == "--help" ]]; then
  usage
  [[ $# -eq 1 ]] && exit 0 || exit 2
fi

bundle_dir="$1"

if [[ ! -d "$bundle_dir" ]]; then
  echo "bundle directory does not exist: $bundle_dir" >&2
  exit 1
fi

required_files=(
  "commit.txt"
  "manifest.txt"
  "release-notes.pointer"
  "migrations/inventory.txt"
  "migrations/inventory.sha256"
  "sbom/README.md"
  "SHA256SUMS"
)

for file in "${required_files[@]}"; do
  if [[ ! -f "$bundle_dir/$file" ]]; then
    echo "missing required bundle file: $file" >&2
    exit 1
  fi
done

if ! find "$bundle_dir/charts" -maxdepth 1 -type f -name 'spendguard-*.tgz' | grep -q .; then
  echo "missing packaged spendguard Helm chart" >&2
  exit 1
fi

commit_sha="$(tr -d '[:space:]' < "$bundle_dir/commit.txt")"
if [[ ! "$commit_sha" =~ ^[0-9a-f]{40}$ ]]; then
  echo "commit.txt must contain a full 40-character git SHA" >&2
  exit 1
fi

(
  cd "$bundle_dir"
  shasum -a 256 -c SHA256SUMS >/dev/null
  shasum -a 256 -c migrations/inventory.sha256 >/dev/null
)

if grep -RInE '(postgres(ql)?://|BEGIN (RSA |EC |OPENSSH |)PRIVATE KEY|AKIA[0-9A-Z]{16}|xox[baprs]-|sk-[A-Za-z0-9_-]{20,})' "$bundle_dir" >/tmp/spendguard-release-secret-scan.txt; then
  echo "release bundle contains a possible secret pattern" >&2
  cat /tmp/spendguard-release-secret-scan.txt >&2
  exit 1
fi

echo "release bundle validated: $bundle_dir"
