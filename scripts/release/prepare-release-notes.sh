#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'USAGE'
Usage:
  scripts/release/prepare-release-notes.sh --check FILE
  scripts/release/prepare-release-notes.sh --version vYYYY.MM.DD-ga.N --commit SHA --output FILE

Validate or generate SpendGuard release notes.
USAGE
}

mode=""
file=""
version=""
commit_sha=""
output=""

while [[ $# -gt 0 ]]; do
  case "$1" in
    --check)
      mode="check"
      file="${2:-}"
      shift 2
      ;;
    --version)
      version="${2:-}"
      shift 2
      ;;
    --commit)
      commit_sha="${2:-}"
      shift 2
      ;;
    --output)
      output="${2:-}"
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

required_sections=(
  "Summary"
  "Breaking Changes"
  "Migrations"
  "Helm Values"
  "Operator Actions"
  "Security Notes"
  "Rollback"
  "Verification"
)

validate_file() {
  local target="$1"
  if [[ ! -f "$target" ]]; then
    echo "release notes file does not exist: $target" >&2
    exit 1
  fi

  local release_line commit_line date_line
  release_line="$(awk -F': *' '/^> \*\*Release\*\*/ {print $2; exit}' "$target" | tr -d '`<>')"
  commit_line="$(awk -F': *' '/^> \*\*Commit\*\*/ {print $2; exit}' "$target" | tr -d '`<>')"
  date_line="$(awk -F': *' '/^> \*\*Date\*\*/ {print $2; exit}' "$target" | tr -d '`<>')"

  if [[ "$release_line" != "version" && ! "$release_line" =~ ^v[0-9]{4}\.[0-9]{2}\.[0-9]{2}-ga\.[0-9]+$ ]]; then
    echo "release version must match vYYYY.MM.DD-ga.N or be the template placeholder" >&2
    exit 1
  fi
  if [[ "$commit_line" != "40-character git SHA" && ! "$commit_line" =~ ^[0-9a-f]{40}$ ]]; then
    echo "commit must be a 40-character lowercase git SHA or the template placeholder" >&2
    exit 1
  fi
  if [[ "$date_line" != "YYYY-MM-DD" && ! "$date_line" =~ ^[0-9]{4}-[0-9]{2}-[0-9]{2}$ ]]; then
    echo "date must be YYYY-MM-DD or the template placeholder" >&2
    exit 1
  fi

  for section in "${required_sections[@]}"; do
    if ! grep -q "^## $section$" "$target"; then
      echo "release notes missing required section: $section" >&2
      exit 1
    fi
  done

  if grep -Eiq '(^|[^[:alpha:]])(latest|current|stable)([^[:alpha:]]|$)' "$target"; then
    echo "release notes must avoid ambiguous latest/current/stable release wording" >&2
    exit 1
  fi
}

if [[ "$mode" == "check" ]]; then
  validate_file "$file"
  echo "release notes validated: $file"
  exit 0
fi

if [[ -n "$version$output$commit_sha" ]]; then
  if [[ ! "$version" =~ ^v[0-9]{4}\.[0-9]{2}\.[0-9]{2}-ga\.[0-9]+$ ]]; then
    echo "--version must match vYYYY.MM.DD-ga.N" >&2
    exit 1
  fi
  if [[ ! "$commit_sha" =~ ^[0-9a-f]{40}$ ]]; then
    echo "--commit must be a 40-character lowercase git SHA" >&2
    exit 1
  fi
  if [[ -z "$output" ]]; then
    echo "--output is required when generating release notes" >&2
    exit 1
  fi
  sed \
    -e "s/<version>/$version/g" \
    -e "s/<40-character git SHA>/$commit_sha/g" \
    -e "s/<YYYY-MM-DD>/$(date -u '+%Y-%m-%d')/g" \
    docs/release/release-notes-template.md > "$output"
  validate_file "$output"
  echo "release notes written: $output"
  exit 0
fi

usage >&2
exit 2
