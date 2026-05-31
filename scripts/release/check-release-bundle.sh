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

command -v helm >/dev/null 2>&1 || {
  echo "helm is required to check the release bundle" >&2
  exit 1
}

release_migration_inventory() {
  local root="$1"
  local commit="$2"
  (
    cd "$root"
    printf '# SpendGuard migration inventory\n'
    printf 'commit=%s\n' "$commit"
    printf '\n'
    find services -type f -name '*.sql' | sort | while read -r migration; do
      case "$migration" in
        services/*/migrations/*.sql)
          case "$migration" in
            services/*/migrations/*/*.sql) continue ;;
          esac
          ;;
        *) continue ;;
      esac
      checksum="$(shasum -a 256 "$migration" | awk '{print $1}')"
      printf '%s  %s\n' "$checksum" "$migration"
    done
  )
}

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
  if [[ -L "$bundle_dir/$file" || ! -f "$bundle_dir/$file" ]]; then
    echo "missing required bundle file: $file" >&2
    exit 1
  fi
done

if find "$bundle_dir" -type l | grep -q .; then
  echo "release bundle must not contain symlinks" >&2
  find "$bundle_dir" -type l >&2
  exit 1
fi

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
if [[ "$release_notes_pointer" != "docs/release/release-notes-template.md" ]]; then
  echo "release notes pointer must be docs/release/release-notes-template.md for v1alpha1 bundles" >&2
  exit 1
fi
repo_root="$(git rev-parse --show-toplevel 2>/dev/null || pwd)"
if [[ -n "$(git -C "$repo_root" status --porcelain)" ]]; then
  echo "release bundle verification requires a clean git worktree" >&2
  git -C "$repo_root" status --short >&2
  exit 1
fi
if ! git -C "$repo_root" cat-file -e "$commit_sha^{commit}" 2>/dev/null; then
  echo "bundle commit does not exist in this repository: $commit_sha" >&2
  exit 1
fi

required_manifest_fields=(
  release_bundle_version
  commit
  branch
  built_at_utc
  chart_version
  helm_version
  release_notes_pointer
)
for field in "${required_manifest_fields[@]}"; do
  if ! awk -F= -v field="$field" '$1 == field {found=1} END {exit found ? 0 : 1}' "$bundle_dir/manifest.txt"; then
    echo "manifest missing required field: $field" >&2
    exit 1
  fi
done
bundle_version="$(awk -F= '$1 == "release_bundle_version" {print $2}' "$bundle_dir/manifest.txt")"
if [[ "$bundle_version" != "v1alpha1" ]]; then
  echo "unsupported release bundle version: $bundle_version" >&2
  exit 1
fi

committed_tree="$(mktemp -d)"
git -C "$repo_root" archive "$commit_sha" | tar -x -C "$committed_tree"
if [[ ! -f "$committed_tree/$release_notes_pointer" ]]; then
  echo "release notes pointer does not resolve in committed release tree: $release_notes_pointer" >&2
  exit 1
fi

expected_inventory="$(mktemp)"
release_migration_inventory "$committed_tree" "$commit_sha" > "$expected_inventory"
if ! diff -u "$expected_inventory" "$bundle_dir/migrations/inventory.txt" >/tmp/spendguard-release-inventory.diff; then
  echo "migration inventory does not match committed release tree" >&2
  cat /tmp/spendguard-release-inventory.diff >&2
  exit 1
fi

chart_compare_dir="$(mktemp -d)"
helm package "$committed_tree/charts/spendguard" --destination "$chart_compare_dir" >/dev/null
expected_chart="$(find "$chart_compare_dir" -maxdepth 1 -type f -name 'spendguard-*.tgz' | sort | head -n 1)"
expected_chart_tree="$(mktemp -d)"
actual_chart_tree="$(mktemp -d)"
tar -xzf "$expected_chart" -C "$expected_chart_tree"
tar -xzf "$chart_pkg" -C "$actual_chart_tree"
if ! diff -qr "$expected_chart_tree" "$actual_chart_tree" >/tmp/spendguard-release-chart.diff; then
  echo "packaged chart content does not match chart rebuilt from committed release tree" >&2
  cat /tmp/spendguard-release-chart.diff >&2
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
  (cd migrations && shasum -a 256 -c inventory.sha256 >/dev/null)
)

chart_scan_dir="$(mktemp -d)"
trap 'rm -rf "$chart_scan_dir" "$chart_compare_dir" "$expected_chart_tree" "$actual_chart_tree" "$expected_inventory" "$committed_tree"' EXIT
tar -xzf "$chart_pkg" -C "$chart_scan_dir"

if grep -RInE '(postgres(ql)?://|BEGIN ((RSA|EC|OPENSSH) )?PRIVATE KEY|AKIA[0-9A-Z]{16}|xox[baprs]-|sk-[A-Za-z0-9_-]{20,})' "$bundle_dir" "$chart_scan_dir" >/tmp/spendguard-release-secret-scan.txt; then
  echo "release bundle contains a possible secret pattern" >&2
  cat /tmp/spendguard-release-secret-scan.txt >&2
  exit 1
fi

echo "release bundle validated: $bundle_dir"
