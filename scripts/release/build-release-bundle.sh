#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'USAGE'
Usage: scripts/release/build-release-bundle.sh --output DIR

Build a local SpendGuard GA release bundle from a clean checkout.
USAGE
}

output_dir=""
while [[ $# -gt 0 ]]; do
  case "$1" in
    --output)
      output_dir="${2:-}"
      shift 2
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "unknown argument: $1" >&2
      usage >&2
      exit 2
      ;;
  esac
done

if [[ -z "$output_dir" ]]; then
  echo "--output is required" >&2
  usage >&2
  exit 2
fi

repo_root="$(git rev-parse --show-toplevel)"
cd "$repo_root"

if [[ -n "$(git status --porcelain)" ]]; then
  echo "release bundle requires a clean git worktree" >&2
  git status --short >&2
  exit 1
fi

command -v helm >/dev/null 2>&1 || {
  echo "helm is required to build the release bundle" >&2
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
    find services -maxdepth 3 -type f -path 'services/*/migrations/*.sql' | sort | while read -r migration; do
      checksum="$(shasum -a 256 "$migration" | awk '{print $1}')"
      printf '%s  %s\n' "$checksum" "$migration"
    done
  )
}

output_parent="$(dirname "$output_dir")"
if [[ ! -d "$output_parent" ]]; then
  echo "release bundle output parent must already exist: $output_parent" >&2
  exit 1
fi
output_parent_real="$(cd "$output_parent" && pwd -P)"
output_base="$(basename "$output_dir")"
output_real="$output_parent_real/$output_base"
repo_real="$(pwd -P)"
home_real="$(cd "$HOME" && pwd -P)"

case "$output_real" in
  "/"|"$repo_real"|"$repo_real"/*|"$home_real"|"$home_real/.ssh"|"$home_real/.gnupg")
    echo "refusing unsafe release bundle output path: $output_real" >&2
    exit 1
    ;;
esac

if [[ -e "$output_real" && ! -f "$output_real/.spendguard-release-bundle" ]]; then
  echo "refusing to delete existing non-SpendGuard bundle directory: $output_real" >&2
  exit 1
fi

commit_sha="$(git rev-parse HEAD)"
branch_name="$(git rev-parse --abbrev-ref HEAD)"
chart_version="$(awk '/^version:/ {print $2; exit}' charts/spendguard/Chart.yaml)"
timestamp_utc="$(date -u '+%Y-%m-%dT%H:%M:%SZ')"

rm -rf "$output_real"
mkdir -p "$output_real/charts" "$output_real/migrations" "$output_real/sbom"
touch "$output_real/.spendguard-release-bundle"

helm lint charts/spendguard >/dev/null
helm template spendguard charts/spendguard --set chart.profile=demo >/dev/null
helm template spendguard charts/spendguard -f scripts/helm-validate-test-values.yaml >/dev/null
helm package charts/spendguard --destination "$output_real/charts" >/dev/null

printf '%s\n' "$commit_sha" > "$output_real/commit.txt"
cat > "$output_real/manifest.txt" <<MANIFEST
release_bundle_version=v1alpha1
commit=$commit_sha
branch=$branch_name
built_at_utc=$timestamp_utc
chart_version=$chart_version
helm_version=$(helm version --short 2>/dev/null || helm version)
release_notes_pointer=docs/release/release-notes-template.md
MANIFEST

cat > "$output_real/release-notes.pointer" <<'POINTER'
docs/release/release-notes-template.md
POINTER

release_migration_inventory "$repo_root" "$commit_sha" > "$output_real/migrations/inventory.txt"
(
  cd "$output_real/migrations"
  shasum -a 256 inventory.txt > inventory.sha256
)

cat > "$output_real/sbom/README.md" <<'SBOM'
# SBOM Status

GA_01 records the SBOM slot in the release bundle. GA_09 owns actual SBOM generation, vulnerability scanning, image signing, and provenance verification.
SBOM

(
  cd "$output_real"
  find . -type f ! -name SHA256SUMS | sort | xargs shasum -a 256 > SHA256SUMS
)

echo "release bundle written to $output_real"
