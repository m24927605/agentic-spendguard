#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
MASTER="$ROOT/docs/post-ga-backlog-spec-v1alpha1.md"

docs=(
  "$ROOT/docs/slices/POST_GA_01_ledger_replay_semantics.md"
  "$ROOT/docs/slices/POST_GA_02_contract_spec_cleanup.md"
  "$ROOT/docs/slices/POST_GA_03_tokenizer_runtime_hardening.md"
  "$ROOT/docs/slices/POST_GA_04_tokenizer_asset_performance.md"
  "$ROOT/docs/slices/POST_GA_05_provider_coverage.md"
  "$ROOT/docs/slices/POST_GA_06_stats_drift_hygiene.md"
  "$ROOT/docs/slices/POST_GA_07_predictor_api_evolution.md"
  "$ROOT/docs/slices/POST_GA_08_db_index_and_rls_polish.md"
  "$ROOT/docs/slices/POST_GA_09_strategy_c_resilience.md"
  "$ROOT/docs/slices/POST_GA_10_test_quality.md"
)

required_review='Reviewer: codex CLI via `codex review --base main`. Max 5 rounds. Staff+ panel arbitration if 5 rounds fail.'

[[ -f "$MASTER" ]] || {
  echo "missing master spec: $MASTER" >&2
  exit 1
}

for doc in "${docs[@]}"; do
  [[ -f "$doc" ]] || {
    echo "missing slice doc: $doc" >&2
    exit 1
  }
  for section in $(seq 0 14); do
    if ! grep -q "§$section" "$doc"; then
      echo "missing §$section in $doc" >&2
      exit 1
    fi
  done
  if ! grep -Fq "$required_review" "$doc"; then
    echo "missing required review execution sentence in $doc" >&2
    exit 1
  fi
  if ! grep -q "Staff" "$doc"; then
    echo "missing Staff+ decision trace in $doc" >&2
    exit 1
  fi
done

python3 - "$MASTER" "${docs[@]}" <<'PY'
import re
import sys
from pathlib import Path

expected = {
    "POST_GA_01_ledger_replay_semantics": {85, 86, 87},
    "POST_GA_02_contract_spec_cleanup": {91, 93, 97, 99, 101, 113, 121, 123, 131, 136, 141, 147, 154, 158, 159, 167, 177},
    "POST_GA_03_tokenizer_runtime_hardening": {92, 94, 96, 98, 100, 103, 105, 110, 111, 112, 114, 115, 117, 118, 119, 126, 127, 129, 133, 135, 148, 149, 151, 152, 156},
    "POST_GA_04_tokenizer_asset_performance": {95, 102, 104, 108, 116, 120, 122, 125, 130, 134, 140},
    "POST_GA_05_provider_coverage": {139},
    "POST_GA_06_stats_drift_hygiene": {157, 162},
    "POST_GA_07_predictor_api_evolution": {161, 165},
    "POST_GA_08_db_index_and_rls_polish": {146, 163, 164, 166},
    "POST_GA_09_strategy_c_resilience": {172, 173, 174, 175, 176},
    "POST_GA_10_test_quality": {109, 124},
}

errors = []
all_seen = set()
for path_s in sys.argv[2:]:
    path = Path(path_s)
    name = path.stem
    issues = {int(n) for n in re.findall(r"#(\d+)", path.read_text(encoding="utf-8"))}
    missing = expected[name] - issues
    if missing:
        errors.append(f"{name} missing issues: {sorted(missing)}")
    all_seen |= (issues & set().union(*expected.values()))

all_expected = set().union(*expected.values())
if all_seen != all_expected:
    errors.append(f"overall coverage mismatch: missing={sorted(all_expected - all_seen)} extra={sorted(all_seen - all_expected)}")

master_text = Path(sys.argv[1]).read_text(encoding="utf-8")
for name, issues in expected.items():
    if name not in master_text:
        errors.append(f"master spec missing slice {name}")
    for issue in issues:
        if f"#{issue}" not in master_text:
            errors.append(f"master spec missing issue #{issue}")

if errors:
    print("post-GA doc validation failed:", file=sys.stderr)
    for err in errors:
        print(f"- {err}", file=sys.stderr)
    sys.exit(1)
PY

echo "POST_GA docs validation PASS"
