#!/usr/bin/env bash
set -euo pipefail

# POST_GA_03 / #149: drift_alert_decided is a writer-owned decision, not
# a database default. A DEFAULT would hide worker regressions that forget
# to set the column.

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
MIGRATION="${ROOT}/services/ledger/migrations/0051_tokenizer_t1_samples.sql"

if grep -Eiq 'drift_alert_decided[[:space:]]+[^,]*DEFAULT' "${MIGRATION}"; then
  echo "drift_alert_decided must not declare a DEFAULT in ${MIGRATION}" >&2
  exit 1
fi

echo "ok tokenizer_t1_samples drift_alert_decided has no DEFAULT"
