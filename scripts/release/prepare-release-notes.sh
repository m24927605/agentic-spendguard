#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'USAGE'
Usage:
  scripts/release/prepare-release-notes.sh --check-template FILE
  scripts/release/prepare-release-notes.sh --check FILE
  scripts/release/prepare-release-notes.sh --check-tag vYYYY.MM.DD-ga.N
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
    --check-template)
      mode="check-template"
      file="${2:-}"
      shift 2
      ;;
    --version)
      version="${2:-}"
      shift 2
      ;;
    --check-tag)
      mode="check-tag"
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
  local allow_placeholders="${2:-false}"
  if [[ ! -f "$target" ]]; then
    echo "release notes file does not exist: $target" >&2
    exit 1
  fi

  local release_line commit_line date_line
  release_line="$(awk -F': *' '/^> \*\*Release\*\*/ {print $2; exit}' "$target" | tr -d '`<>')"
  commit_line="$(awk -F': *' '/^> \*\*Commit\*\*/ {print $2; exit}' "$target" | tr -d '`<>')"
  date_line="$(awk -F': *' '/^> \*\*Date\*\*/ {print $2; exit}' "$target" | tr -d '`<>')"

  if [[ "$release_line" == "version" || "$commit_line" == "40-character git SHA" || "$date_line" == "YYYY-MM-DD" ]]; then
    if [[ "$allow_placeholders" != "true" ]]; then
      echo "final release notes must not contain template placeholders" >&2
      exit 1
    fi
  fi

  if [[ "$release_line" != "version" ]] && ! valid_version "$release_line"; then
    echo "release version must match a calendar-valid vYYYY.MM.DD-ga.N or be the template placeholder" >&2
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
  if [[ "$date_line" != "YYYY-MM-DD" ]] && ! valid_calendar_date "$date_line"; then
    echo "date must be calendar-valid YYYY-MM-DD" >&2
    exit 1
  fi
  if [[ "$commit_line" != "40-character git SHA" ]] && ! git cat-file -e "$commit_line^{commit}" 2>/dev/null; then
    echo "commit does not exist in this repository: $commit_line" >&2
    exit 1
  fi

  for section in "${required_sections[@]}"; do
    if ! grep -q "^## $section$" "$target"; then
      echo "release notes missing required section: $section" >&2
      exit 1
    fi
    if [[ "$allow_placeholders" != "true" ]]; then
      body="$(awk -v section="$section" '
        $0 == "## " section {in_section=1; next}
        in_section && /^## / {exit}
        in_section {print}
      ' "$target" | sed '/^[[:space:]]*$/d' | sed '/^<!--/,/^-->/d')"
      if [[ -z "$body" ]]; then
        echo "release notes section is empty: $section" >&2
        exit 1
      fi
      if grep -Eiq '<[^>]+>|Describe the|List |TODO|TBD|fill in|to be determined|replace this|placeholder' <<<"$body"; then
        echo "release notes section still contains template text: $section" >&2
        exit 1
      fi
      compact_body="$(printf '%s' "$body" | tr -d '[:space:]' | tr '[:upper:]' '[:lower:]')"
      if [[ "$section" != "Breaking Changes" ]] && [[ "$compact_body" =~ ^(none\.?|n/a|na|notapplicable|-|--|_)$ ]]; then
        echo "release notes section must contain concrete content: $section" >&2
        exit 1
      fi
    fi
  done

  if grep -Eiq '(^|[^[:alpha:]])(latest|current|stable)([^[:alpha:]]|$)' "$target"; then
    echo "release notes must avoid ambiguous latest/current/stable release wording" >&2
    exit 1
  fi
}

valid_version() {
  local candidate="$1"
  [[ "$candidate" =~ ^v([0-9]{4})\.([0-9]{2})\.([0-9]{2})-ga\.([0-9]+)$ ]] || return 1
  local y="${BASH_REMATCH[1]}"
  local m="${BASH_REMATCH[2]}"
  local d="${BASH_REMATCH[3]}"
  valid_calendar_date "$y-$m-$d"
}

valid_calendar_date() {
  local candidate="$1"
  python3 - "$candidate" <<'PY' >/dev/null 2>&1
import datetime
import sys

datetime.datetime.strptime(sys.argv[1], "%Y-%m-%d")
PY
}

if [[ "$mode" == "check-template" ]]; then
  validate_file "$file" true
  echo "release notes template validated: $file"
  exit 0
fi

if [[ "$mode" == "check" ]]; then
  validate_file "$file" false
  echo "release notes validated: $file"
  exit 0
fi

if [[ "$mode" == "check-tag" ]]; then
  if ! valid_version "$version"; then
    echo "--check-tag value must match a calendar-valid vYYYY.MM.DD-ga.N" >&2
    exit 1
  fi
  if git rev-parse -q --verify "refs/tags/$version" >/dev/null; then
    echo "tag already exists: $version" >&2
    exit 1
  fi
  remote_status=0
  git ls-remote --exit-code --tags origin "refs/tags/$version" >/tmp/spendguard-release-tag-check.out 2>/tmp/spendguard-release-tag-check.err || remote_status=$?
  if [[ "$remote_status" == "0" ]]; then
    echo "remote tag already exists: $version" >&2
    exit 1
  fi
  if [[ "$remote_status" != "2" ]]; then
    echo "could not verify remote tag availability for $version" >&2
    cat /tmp/spendguard-release-tag-check.err >&2
    exit 1
  fi
  echo "tag available: $version"
  exit 0
fi

if [[ -n "$version$output$commit_sha" ]]; then
  if ! valid_version "$version"; then
    echo "--version must match a calendar-valid vYYYY.MM.DD-ga.N" >&2
    exit 1
  fi
  if [[ ! "$commit_sha" =~ ^[0-9a-f]{40}$ ]]; then
    echo "--commit must be a 40-character lowercase git SHA" >&2
    exit 1
  fi
  if ! git cat-file -e "$commit_sha^{commit}" 2>/dev/null; then
    echo "--commit does not exist in this repository: $commit_sha" >&2
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
  echo "release notes written: $output"
  echo "fill in all required sections, then run: scripts/release/prepare-release-notes.sh --check $output"
  exit 0
fi

usage >&2
exit 2
