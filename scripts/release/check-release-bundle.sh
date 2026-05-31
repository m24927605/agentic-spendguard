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
  ".spendguard-release-bundle"
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

chart_count="$(find "$bundle_dir/charts" -maxdepth 1 -type f -name 'spendguard-*.tgz' | wc -l | tr -d ' ')"
if [[ "$chart_count" != "1" ]]; then
  echo "missing packaged spendguard Helm chart" >&2
  exit 1
fi
chart_pkg="$(find "$bundle_dir/charts" -maxdepth 1 -type f -name 'spendguard-*.tgz' | sort | head -n 1)"

commit_sha="$(tr -d '[:space:]' < "$bundle_dir/commit.txt")"
if [[ ! "$commit_sha" =~ ^[0-9a-f]{40}$ ]]; then
  echo "commit.txt must contain a full 40-character git SHA" >&2
  exit 1
fi

manifest_commit="$(awk -F= '$1 == "commit" {print $2}' "$bundle_dir/manifest.txt")"
if [[ "$manifest_commit" != "$commit_sha" ]]; then
  echo "manifest commit does not match commit.txt" >&2
  exit 1
fi

manifest_chart_version="$(awk -F= '$1 == "chart_version" {print $2}' "$bundle_dir/manifest.txt")"
chart_name="$(helm show chart "$chart_pkg" | awk -F': *' '$1 == "name" {print $2; exit}')"
chart_version="$(helm show chart "$chart_pkg" | awk -F': *' '$1 == "version" {print $2; exit}')"
if [[ "$chart_name" != "spendguard" ]]; then
  echo "packaged chart name is not spendguard: $chart_name" >&2
  exit 1
fi
if [[ "$chart_version" != "$manifest_chart_version" ]]; then
  echo "packaged chart version does not match manifest" >&2
  exit 1
fi

release_notes_pointer="$(tr -d '[:space:]' < "$bundle_dir/release-notes.pointer")"
manifest_release_notes_pointer="$(awk -F= '$1 == "release_notes_pointer" {print $2}' "$bundle_dir/manifest.txt")"
if [[ "$release_notes_pointer" != "$manifest_release_notes_pointer" ]]; then
  echo "release notes pointer does not match manifest" >&2
  exit 1
fi
repo_root="$(git rev-parse --show-toplevel 2>/dev/null || pwd)"
if [[ ! -f "$repo_root/$release_notes_pointer" ]]; then
  echo "release notes pointer does not resolve in repo: $release_notes_pointer" >&2
  exit 1
fi

(
  cd "$bundle_dir"
  find . -type f ! -name SHA256SUMS | sort > /tmp/spendguard-release-files-expected.txt
  awk '{print $2}' SHA256SUMS | sort > /tmp/spendguard-release-files-recorded.txt
  if ! diff -u /tmp/spendguard-release-files-expected.txt /tmp/spendguard-release-files-recorded.txt >/tmp/spendguard-release-files.diff; then
    echo "SHA256SUMS does not cover exactly every bundle file" >&2
    cat /tmp/spendguard-release-files.diff >&2
    exit 1
  fi
  shasum -a 256 -c SHA256SUMS >/dev/null
  shasum -a 256 -c migrations/inventory.sha256 >/dev/null
)

chart_scan_dir="$(mktemp -d)"
trap 'rm -rf "$chart_scan_dir"' EXIT
tar -xzf "$chart_pkg" -C "$chart_scan_dir"

if grep -RInE '(postgres(ql)?://|BEGIN ((RSA|EC|OPENSSH) )?PRIVATE KEY|AKIA[0-9A-Z]{16}|xox[baprs]-|sk-[A-Za-z0-9_-]{20,})' "$bundle_dir" "$chart_scan_dir" >/tmp/spendguard-release-secret-scan.txt; then
  echo "release bundle contains a possible secret pattern" >&2
  cat /tmp/spendguard-release-secret-scan.txt >&2
  exit 1
fi

echo "release bundle validated: $bundle_dir"
