#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
COMPOSE=(docker compose -f "$ROOT/deploy/demo/compose.yaml")
EVIDENCE_DIR="${EVIDENCE_DIR:-$ROOT/docs/reviews/ga-readiness/GA_06_alerting_runbooks_drills}"
EVIDENCE_FILE="$EVIDENCE_DIR/outbox_lag_recovery.json"
ALERT_FOR_SECONDS="${ALERT_FOR_SECONDS:-300}"
STARTED_AT="$(date -u +"%Y-%m-%dT%H:%M:%SZ")"

require_cmd() {
  if ! command -v "$1" >/dev/null 2>&1; then
    echo "missing required command: $1" >&2
    exit 127
  fi
}

psql_ledger() {
  "${COMPOSE[@]}" exec -T postgres psql -U spendguard -d spendguard_ledger -At -v ON_ERROR_STOP=1 -c "$1"
}

pending_count() {
  psql_ledger "SELECT count(*) FROM audit_outbox WHERE pending_forward = TRUE;"
}

scrape_outbox_metrics() {
  curl -fsS http://127.0.0.1:9096/metrics
}

outbox_lag_metric() {
  scrape_outbox_metrics | awk '$1 == "spendguard_outbox_pending_oldest_age_seconds" { print int($2); found=1 } END { if (!found) print 0 }'
}

cleanup() {
  "${COMPOSE[@]}" start canonical-ingest >/dev/null 2>&1 || true
}
trap cleanup EXIT

require_cmd docker
require_cmd curl
require_cmd awk
require_cmd python3

mkdir -p "$EVIDENCE_DIR"

echo "[drill] resetting demo stack"
make -C "$ROOT" demo-down

echo "[drill] running default demo to create signed, real audit_outbox rows"
make -C "$ROOT" demo-up DEMO_MODE=default

echo "[drill] stopping canonical-ingest so outbox-forwarder records backlog"
"${COMPOSE[@]}" stop canonical-ingest >/dev/null

echo "[drill] reopening one already-forwarded audit row via forwarder-state columns"
REARMED="$(
  psql_ledger "
    WITH candidate AS (
      SELECT recorded_month, audit_outbox_id
        FROM audit_outbox
       WHERE pending_forward = FALSE
         AND forwarded_at IS NOT NULL
         AND last_forward_error IS NULL
       ORDER BY recorded_at ASC
       LIMIT 1
    )
    UPDATE audit_outbox a
       SET pending_forward = TRUE,
           forwarded_at = NULL,
           forward_attempts = 0,
           last_forward_error = 'GA_06 outbox lag recovery drill'
      FROM candidate c
     WHERE a.recorded_month = c.recorded_month
       AND a.audit_outbox_id = c.audit_outbox_id
    RETURNING a.audit_outbox_id::text || '|' ||
              EXTRACT(EPOCH FROM (clock_timestamp() - a.recorded_at))::bigint;
  "
)"
REARMED="$(printf '%s\n' "$REARMED" | head -n1)"
if [[ -z "$REARMED" ]]; then
  echo "no forwarded audit_outbox row was available to rearm" >&2
  exit 1
fi
REARMED_ID="${REARMED%%|*}"
REARMED_AGE="${REARMED##*|}"

PENDING_DURING="$(pending_count)"
if [[ "$PENDING_DURING" -le 0 ]]; then
  echo "expected pending outbox rows during canonical-ingest outage" >&2
  exit 1
fi

echo "[drill] waiting for lag metric to cross 60s alert threshold"
LAG_DURING=0
for _ in $(seq 1 90); do
  LAG_DURING="$(outbox_lag_metric)"
  if [[ "$LAG_DURING" -gt 60 ]]; then
    break
  fi
  sleep 2
done
if [[ "$LAG_DURING" -le 60 ]]; then
  echo "outbox lag metric did not exceed 60s; last value=$LAG_DURING" >&2
  exit 1
fi

echo "[drill] holding lag predicate true for ${ALERT_FOR_SECONDS}s"
HELD_SECONDS=0
while [[ "$HELD_SECONDS" -lt "$ALERT_FOR_SECONDS" ]]; do
  sleep 10
  HELD_SECONDS=$((HELD_SECONDS + 10))
  LAG_DURING="$(outbox_lag_metric)"
  if [[ "$LAG_DURING" -le 60 ]]; then
    echo "outbox lag predicate dropped before alert for-duration; value=$LAG_DURING held=${HELD_SECONDS}s" >&2
    exit 1
  fi
done

METRICS_DURING="$(scrape_outbox_metrics | grep -E 'spendguard_outbox_(pending_oldest_age_seconds|forwarder_is_leader)' || true)"

echo "[drill] restarting canonical-ingest and waiting for drain"
"${COMPOSE[@]}" start canonical-ingest >/dev/null
for _ in $(seq 1 60); do
  PENDING_AFTER="$(pending_count)"
  if [[ "$PENDING_AFTER" -eq 0 ]]; then
    break
  fi
  sleep 2
done
PENDING_AFTER="$(pending_count)"
if [[ "$PENDING_AFTER" -ne 0 ]]; then
  echo "outbox backlog did not drain after canonical-ingest recovery; pending=$PENDING_AFTER" >&2
  exit 1
fi

LAG_AFTER="$(outbox_lag_metric)"
METRICS_AFTER="$(scrape_outbox_metrics | grep -E 'spendguard_outbox_(pending_oldest_age_seconds|forwarder_is_leader)' || true)"
FINISHED_AT="$(date -u +"%Y-%m-%dT%H:%M:%SZ")"

STARTED_AT="$STARTED_AT" \
FINISHED_AT="$FINISHED_AT" \
REARMED_ID="$REARMED_ID" \
REARMED_AGE="$REARMED_AGE" \
PENDING_DURING="$PENDING_DURING" \
LAG_DURING="$LAG_DURING" \
ALERT_FOR_SECONDS="$ALERT_FOR_SECONDS" \
PENDING_AFTER="$PENDING_AFTER" \
LAG_AFTER="$LAG_AFTER" \
METRICS_DURING="$METRICS_DURING" \
METRICS_AFTER="$METRICS_AFTER" \
EVIDENCE_FILE="$EVIDENCE_FILE" \
python3 - <<'PY'
import json
import os
from pathlib import Path

evidence = {
    "result": "pass",
    "started_at": os.environ["STARTED_AT"],
    "finished_at": os.environ["FINISHED_AT"],
    "rearmed_audit_outbox_id": os.environ["REARMED_ID"],
    "rearmed_row_age_seconds": int(os.environ["REARMED_AGE"]),
    "pending_during_outage": int(os.environ["PENDING_DURING"]),
    "lag_metric_during_outage": int(os.environ["LAG_DURING"]),
    "alert_predicate_hold_seconds": int(os.environ["ALERT_FOR_SECONDS"]),
    "pending_after_recovery": int(os.environ["PENDING_AFTER"]),
    "lag_metric_after_recovery": int(os.environ["LAG_AFTER"]),
    "metrics_excerpt_during_outage": os.environ["METRICS_DURING"].splitlines(),
    "metrics_excerpt_after_recovery": os.environ["METRICS_AFTER"].splitlines(),
}
path = Path(os.environ["EVIDENCE_FILE"])
path.write_text(json.dumps(evidence, indent=2, sort_keys=True) + "\n", encoding="utf-8")
print(path)
PY

echo "[drill] PASS: evidence written to $EVIDENCE_FILE"
