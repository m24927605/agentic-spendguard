#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
ONBOARDING="$ROOT/docs/customer/plugin-onboarding.md"
CHECKLIST="$ROOT/docs/customer/plugin-certification-checklist.md"
TAXONOMY="$ROOT/docs/customer/plugin-error-taxonomy.md"
TRIAGE="$ROOT/docs/reviews/ga-readiness/GA_10_customer_plugin_onboarding_backlog/backlog-triage.md"
README="$ROOT/contrib/output_predictor_template/README.md"

require_file() {
  local file="$1"
  if [[ ! -f "$file" ]]; then
    echo "missing required file: $file" >&2
    exit 1
  fi
}

require_text() {
  local file="$1"
  local pattern="$2"
  local description="$3"
  if ! grep -Eiq "$pattern" "$file"; then
    echo "missing $description in $file" >&2
    exit 1
  fi
}

for file in "$ONBOARDING" "$CHECKLIST" "$TAXONOMY" "$TRIAGE" "$README"; do
  require_file "$file"
done

require_text "$ONBOARDING" "SVID" "SVID onboarding requirement"
require_text "$ONBOARDING" "mTLS|mutual TLS" "mTLS onboarding requirement"
require_text "$ONBOARDING" "/v1/predictor/plugins" "live predictor plugin API path"
require_text "$ONBOARDING" "client_cert_id" "client certificate binding field"
require_text "$ONBOARDING" "plugin-client-svid" "client SVID evidence path"
require_text "$ONBOARDING" "50 ms|timeout" "timeout requirement"
require_text "$ONBOARDING" "retry" "retry/idempotency guidance"
require_text "$ONBOARDING" "circuit breaker" "circuit breaker guidance"
require_text "$ONBOARDING" "audit" "audit expectation"
require_text "$CHECKLIST" "python3 -m pytest conformance_test.py -q" "conformance command"
require_text "$CHECKLIST" "spiffe://spendguard.platform/predictor-client" "exact SVID subject"
require_text "$CHECKLIST" "Hard Fail Conditions" "hard fail section"

for mode in timeout grpc_error invalid_zero_or_negative invalid_overflow invalid_confidence deserialization_error tls_error not_serving not_configured breaker_open; do
  require_text "$TAXONOMY" "$mode" "taxonomy mode $mode"
done

for n in $(seq 85 177); do
  require_text "$TRIAGE" "#$n" "issue coverage for #$n"
done

require_text "$TRIAGE" "#155.*GA_10 closure" "duplicate/process closure evidence for #155"
require_text "$TRIAGE" "#170.*GA_10 closure" "duplicate closure evidence for #170"
require_text "$README" "Certification path" "template certification path"

python3 - "$TRIAGE" <<'PY'
import re
import sys
from collections import defaultdict

path = sys.argv[1]
lines = open(path, encoding="utf-8").read().splitlines()
named = defaultdict(set)
coverage = {}
section = None

for line in lines:
    if line.startswith("## Named Post-GA Slices"):
        section = "named"
        continue
    if line.startswith("## Issue Coverage"):
        section = "coverage"
        continue
    if line.startswith("## ") and section in {"named", "coverage"}:
        section = None

    if section == "named":
        m = re.match(r"\| (POST_GA_\d+_[A-Za-z0-9_]+) \| [^|]+ \| ([^|]*) \|", line)
        if m:
            slice_name, issues = m.groups()
            named[slice_name].update(int(n) for n in re.findall(r"#(\d+)", issues))

    if section == "coverage":
        m = re.match(r"\| #(\d+) \| ([^|]+) \| ([^|]+) \| ([^|]+) \|", line)
        if m:
            issue = int(m.group(1))
            disposition = m.group(4)
            coverage[issue] = set(re.findall(r"POST_GA_\d+_[A-Za-z0-9_]+", disposition))

errors = []
for issue, slices in sorted(coverage.items()):
    if len(slices) > 1:
        errors.append(f"issue #{issue} maps to multiple slices: {sorted(slices)}")
    for slice_name in slices:
        if issue not in named.get(slice_name, set()):
            errors.append(f"coverage maps #{issue} to {slice_name}, but named table does not")

reverse = defaultdict(set)
for slice_name, issues in named.items():
    for issue in issues:
        reverse[issue].add(slice_name)
        if slice_name not in coverage.get(issue, set()):
            errors.append(f"named table maps #{issue} to {slice_name}, but coverage row does not")

for issue, slices in sorted(reverse.items()):
    if len(slices) > 1:
        errors.append(f"named table assigns #{issue} to multiple slices: {sorted(slices)}")

if errors:
    print("GA_10 backlog triage mismatch:", file=sys.stderr)
    for error in errors:
        print(f"- {error}", file=sys.stderr)
    sys.exit(1)
PY

echo "GA_10 docs validation PASS"
