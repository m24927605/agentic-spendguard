#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
COMPOSE=(docker compose -f "$ROOT/deploy/demo/compose.yaml")
TENANT_ID="${TENANT_ID:-00000000-0000-4000-8000-000000000001}"
SCENARIO="$ROOT/benchmarks/ga-load/scenarios/local-100-tenants.yaml"
EVIDENCE_DIR="$ROOT/docs/reviews/ga-readiness/GA_08_scale_performance_slo_proof"
RESET_STACK=1

usage() {
  cat <<'USAGE'
Usage: benchmarks/ga-load/run.sh [options]

Options:
  --scenario <path>      Scenario file. Default: benchmarks/ga-load/scenarios/local-100-tenants.yaml.
  --evidence-dir <path>  Evidence output directory. Default: docs/reviews/ga-readiness/GA_08_scale_performance_slo_proof.
  --no-reset             Reuse the running compose stack.
  -h, --help             Show this help.
USAGE
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --scenario)
      [[ $# -ge 2 ]] || { echo "--scenario requires a value" >&2; usage >&2; exit 2; }
      SCENARIO="$2"; shift 2 ;;
    --evidence-dir)
      [[ $# -ge 2 ]] || { echo "--evidence-dir requires a value" >&2; usage >&2; exit 2; }
      EVIDENCE_DIR="$2"; shift 2 ;;
    --no-reset) RESET_STACK=0; shift ;;
    -h|--help) usage; exit 0 ;;
    *) echo "unknown argument: $1" >&2; usage >&2; exit 2 ;;
  esac
done

require_cmd() {
  if ! command -v "$1" >/dev/null 2>&1; then
    echo "missing required command: $1" >&2
    exit 127
  fi
}

psql_db() {
  local db="$1"
  local sql="$2"
  "${COMPOSE[@]}" exec -T postgres \
    psql -U spendguard -d "$db" -At -F '|' -v ON_ERROR_STOP=1 -c "$sql"
}

parse_json_field() {
  local file="$1"
  local expr="$2"
  python3 - "$file" "$expr" <<'PY'
import json, sys
data = json.load(open(sys.argv[1]))
cur = data
for part in sys.argv[2].split("."):
    cur = cur[part]
print(cur)
PY
}

wait_for_http() {
  local url="$1"
  local label="$2"
  for _ in $(seq 1 90); do
    if curl -fsS --connect-timeout 2 --max-time 5 "$url" >/dev/null 2>&1; then
      return 0
    fi
    sleep 2
  done
  echo "timed out waiting for $label at $url" >&2
  return 1
}

wait_for_container_http() {
  local service="$1"
  local url="$2"
  local label="$3"
  for _ in $(seq 1 90); do
    if "${COMPOSE[@]}" exec -T "$service" \
      wget -T 5 -q -O - "$url" >/dev/null 2>&1; then
      return 0
    fi
    sleep 2
  done
  echo "timed out waiting for $label at $service:$url" >&2
  return 1
}

wait_for_outbox_drain() {
  local max_wait="$1"
  local waited=0
  local pending
  while [[ "$waited" -le "$max_wait" ]]; do
    pending="$(psql_db spendguard_ledger "SELECT count(*)::int FROM audit_outbox WHERE pending_forward = TRUE;" | tail -n1)"
    if [[ "$pending" == "0" ]]; then
      return 0
    fi
    sleep 2
    waited=$((waited + 2))
  done
  echo "outbox did not drain within ${max_wait}s" >&2
  return 1
}

require_cmd docker
require_cmd python3
require_cmd curl

SCENARIO="$(cd "$(dirname "$SCENARIO")" && pwd)/$(basename "$SCENARIO")"
EVIDENCE_DIR="$(mkdir -p "$EVIDENCE_DIR" && cd "$EVIDENCE_DIR" && pwd)"
LOAD_RESULTS="$EVIDENCE_DIR/load-results.json"
SUMMARY="$EVIDENCE_DIR/ga_load_summary.json"
COMMAND_RESULTS="$EVIDENCE_DIR/command-results.md"
EXPLAIN_OUTPUT="$EVIDENCE_DIR/explain-ga-plans.txt"

if [[ ! -f "$SCENARIO" ]]; then
  echo "scenario not found: $SCENARIO" >&2
  exit 2
fi

GIT_COMMIT_SHA="$(git -C "$ROOT" rev-parse HEAD)"
GIT_BRANCH="$(git -C "$ROOT" branch --show-current)"
GIT_SOURCE_STATUS="$(git -C "$ROOT" status --short)"
MACHINE_DESCRIPTOR="$(uname -a)"
COMMAND_LINE="$(printf '%q ' "$0" "$@")"

if [[ "$RESET_STACK" -eq 1 ]]; then
  echo "[ga-load] resetting demo stack"
  make -C "$ROOT/deploy/demo" demo-down >/dev/null || true
fi

echo "[ga-load] starting real compose stack"
SPENDGUARD_SIDECAR_RUN_COST_PROJECTOR_URL=http://run-cost-projector:50055 \
SPENDGUARD_SIDECAR_ALLOW_UNTRUSTED_BUDGET_METADATA=true \
  "${COMPOSE[@]}" up -d --build \
    postgres pki-init pricing-seed-init bundles-init canonical-seed-init \
    manifest-init endpoint-catalog ledger canonical-ingest tokenizer \
    output-predictor run-cost-projector stats-aggregator sidecar \
    webhook-receiver outbox-forwarder

echo "[ga-load] building demo adapter image for in-network load driver"
"${COMPOSE[@]}" build demo >/dev/null

wait_for_http "http://127.0.0.1:9093/metrics" "sidecar metrics"
wait_for_http "http://127.0.0.1:9099/healthz" "tokenizer health"
wait_for_http "http://127.0.0.1:9100/healthz" "output predictor health"
wait_for_http "http://127.0.0.1:9102/healthz" "run cost projector health"
wait_for_container_http canonical-ingest "http://127.0.0.1:9091/metrics" "canonical metrics"

CANONICAL_BEFORE="$(psql_db spendguard_canonical "SELECT count(*)::int FROM canonical_events WHERE tenant_id = '$TENANT_ID';" | tail -n1)"
CANONICAL_DECISION_BEFORE="$(psql_db spendguard_canonical "SELECT count(*)::int FROM canonical_events WHERE tenant_id = '$TENANT_ID' AND event_type = 'spendguard.audit.decision';" | tail -n1)"
CANONICAL_OUTCOME_BEFORE="$(psql_db spendguard_canonical "SELECT count(*)::int FROM canonical_events WHERE tenant_id = '$TENANT_ID' AND event_type = 'spendguard.audit.outcome';" | tail -n1)"
LEDGER_OUTBOX_BEFORE="$(psql_db spendguard_ledger "SELECT count(*)::int FROM audit_outbox WHERE tenant_id = '$TENANT_ID';" | tail -n1)"
LEDGER_OUTBOX_DECISION_BEFORE="$(psql_db spendguard_ledger "SELECT count(*)::int FROM audit_outbox WHERE tenant_id = '$TENANT_ID' AND event_type = 'spendguard.audit.decision';" | tail -n1)"
LEDGER_OUTBOX_OUTCOME_BEFORE="$(psql_db spendguard_ledger "SELECT count(*)::int FROM audit_outbox WHERE tenant_id = '$TENANT_ID' AND event_type = 'spendguard.audit.outcome';" | tail -n1)"

echo "[ga-load] running load driver in demo container"
set +e
"${COMPOSE[@]}" run --rm --no-deps \
  --volume "$ROOT:/workspace:ro" \
  --volume "$EVIDENCE_DIR:/evidence" \
  --entrypoint /bin/sh \
  demo \
  -c "set -a && . /var/lib/spendguard/bundles/runtime.env && set +a && exec python /workspace/benchmarks/ga-load/driver.py --scenario /workspace/${SCENARIO#"$ROOT"/} --output /evidence/load-results.json --proto-root /workspace/proto"
DRIVER_STATUS=$?
set -e

if [[ ! -f "$LOAD_RESULTS" ]]; then
  python3 - "$LOAD_RESULTS" "$SCENARIO" "$DRIVER_STATUS" <<'PY'
import json, sys
path, scenario_path, status = sys.argv[1], sys.argv[2], int(sys.argv[3])
payload = {
    "result": "fail",
    "scenario": {"path": scenario_path},
    "expected_operations": 0,
    "completed_operations": 0,
    "error_count": 1,
    "errors": [{"error": "load driver did not write load-results.json"}],
    "latency": {},
    "cardinality": {},
    "service_metrics": {},
    "failures": [f"load driver exited {status} before writing results"],
}
open(path, "w", encoding="utf-8").write(json.dumps(payload, indent=2, sort_keys=True) + "\n")
PY
fi

EXPECTED_OPS="$(parse_json_field "$LOAD_RESULTS" expected_operations)"
MAX_DRAIN_WAIT="$(parse_json_field "$LOAD_RESULTS" scenario.slos.max_outbox_drain_wait_seconds)"

echo "[ga-load] waiting for outbox drain"
set +e
wait_for_outbox_drain "$MAX_DRAIN_WAIT"
OUTBOX_STATUS=$?
set -e

echo "[ga-load] running audit-column integrity probe"
set +e
python3 "$ROOT/tests/e2e/verify_audit_columns.py" --tenant "$TENANT_ID" >"$EVIDENCE_DIR/verify-audit-columns.txt" 2>&1
VERIFY_STATUS=$?
set -e

echo "[ga-load] running DB plan gate"
set +e
"${COMPOSE[@]}" exec -T postgres \
  psql -U spendguard -d spendguard_canonical -v ON_ERROR_STOP=1 \
  < "$ROOT/scripts/db/explain-ga-plans.sql" >"$EXPLAIN_OUTPUT" 2>&1
PLAN_STATUS=$?
set -e

CANONICAL_AFTER="$(psql_db spendguard_canonical "SELECT count(*)::int FROM canonical_events WHERE tenant_id = '$TENANT_ID';" | tail -n1)"
CANONICAL_DECISION_AFTER="$(psql_db spendguard_canonical "SELECT count(*)::int FROM canonical_events WHERE tenant_id = '$TENANT_ID' AND event_type = 'spendguard.audit.decision';" | tail -n1)"
CANONICAL_OUTCOME_AFTER="$(psql_db spendguard_canonical "SELECT count(*)::int FROM canonical_events WHERE tenant_id = '$TENANT_ID' AND event_type = 'spendguard.audit.outcome';" | tail -n1)"
LEDGER_OUTBOX_AFTER="$(psql_db spendguard_ledger "SELECT count(*)::int FROM audit_outbox WHERE tenant_id = '$TENANT_ID';" | tail -n1)"
LEDGER_OUTBOX_DECISION_AFTER="$(psql_db spendguard_ledger "SELECT count(*)::int FROM audit_outbox WHERE tenant_id = '$TENANT_ID' AND event_type = 'spendguard.audit.decision';" | tail -n1)"
LEDGER_OUTBOX_OUTCOME_AFTER="$(psql_db spendguard_ledger "SELECT count(*)::int FROM audit_outbox WHERE tenant_id = '$TENANT_ID' AND event_type = 'spendguard.audit.outcome';" | tail -n1)"
PENDING_AFTER="$(psql_db spendguard_ledger "SELECT count(*)::int FROM audit_outbox WHERE pending_forward = TRUE;" | tail -n1)"
OLDEST_LAG_AFTER="$(psql_db spendguard_ledger "SELECT COALESCE(EXTRACT(EPOCH FROM (clock_timestamp() - min(recorded_at)))::bigint, 0) FROM audit_outbox WHERE pending_forward = TRUE;" | tail -n1)"
CANONICAL_DELTA=$((CANONICAL_AFTER - CANONICAL_BEFORE))
CANONICAL_DECISION_DELTA=$((CANONICAL_DECISION_AFTER - CANONICAL_DECISION_BEFORE))
CANONICAL_OUTCOME_DELTA=$((CANONICAL_OUTCOME_AFTER - CANONICAL_OUTCOME_BEFORE))
LEDGER_OUTBOX_DELTA=$((LEDGER_OUTBOX_AFTER - LEDGER_OUTBOX_BEFORE))
LEDGER_OUTBOX_DECISION_DELTA=$((LEDGER_OUTBOX_DECISION_AFTER - LEDGER_OUTBOX_DECISION_BEFORE))
LEDGER_OUTBOX_OUTCOME_DELTA=$((LEDGER_OUTBOX_OUTCOME_AFTER - LEDGER_OUTBOX_OUTCOME_BEFORE))
export EXPECTED_OPS CANONICAL_DELTA CANONICAL_DECISION_DELTA CANONICAL_OUTCOME_DELTA
export LEDGER_OUTBOX_DELTA LEDGER_OUTBOX_DECISION_DELTA LEDGER_OUTBOX_OUTCOME_DELTA
export PENDING_AFTER OLDEST_LAG_AFTER
export DRIVER_STATUS OUTBOX_STATUS VERIFY_STATUS PLAN_STATUS
export GIT_BRANCH GIT_COMMIT_SHA GIT_SOURCE_STATUS COMMAND_LINE MACHINE_DESCRIPTOR
export SCENARIO TENANT_ID
export CANONICAL_BEFORE CANONICAL_AFTER CANONICAL_DECISION_BEFORE CANONICAL_DECISION_AFTER CANONICAL_OUTCOME_BEFORE CANONICAL_OUTCOME_AFTER
export LEDGER_OUTBOX_BEFORE LEDGER_OUTBOX_AFTER LEDGER_OUTBOX_DECISION_BEFORE LEDGER_OUTBOX_DECISION_AFTER LEDGER_OUTBOX_OUTCOME_BEFORE LEDGER_OUTBOX_OUTCOME_AFTER

python3 - "$LOAD_RESULTS" "$SUMMARY" "$COMMAND_RESULTS" <<'PY'
import json
import os
import sys
from datetime import datetime, timezone

load_path, summary_path, command_path = sys.argv[1:4]
load = json.load(open(load_path))
expected = int(os.environ["EXPECTED_OPS"])
canonical_delta = int(os.environ["CANONICAL_DELTA"])
canonical_decision_delta = int(os.environ["CANONICAL_DECISION_DELTA"])
canonical_outcome_delta = int(os.environ["CANONICAL_OUTCOME_DELTA"])
ledger_delta = int(os.environ["LEDGER_OUTBOX_DELTA"])
ledger_decision_delta = int(os.environ["LEDGER_OUTBOX_DECISION_DELTA"])
ledger_outcome_delta = int(os.environ["LEDGER_OUTBOX_OUTCOME_DELTA"])
pending_after = int(os.environ["PENDING_AFTER"])
oldest_lag_after = int(os.environ["OLDEST_LAG_AFTER"])
driver_status = int(os.environ["DRIVER_STATUS"])
outbox_status = int(os.environ["OUTBOX_STATUS"])
verify_status = int(os.environ["VERIFY_STATUS"])
plan_status = int(os.environ["PLAN_STATUS"])

failures = list(load.get("failures", []))
if driver_status != 0:
    failures.append(f"load driver exited {driver_status}")
if outbox_status != 0:
    failures.append("outbox drain timed out")
if verify_status != 0:
    failures.append(f"verify_audit_columns exited {verify_status}")
if plan_status != 0:
    failures.append(f"explain-ga-plans exited {plan_status}")
if canonical_delta < expected * 2:
    failures.append(f"canonical_events delta {canonical_delta} below expected decision+outcome rows {expected * 2}")
if canonical_decision_delta < expected:
    failures.append(f"canonical_events decision delta {canonical_decision_delta} below expected operations {expected}")
if canonical_outcome_delta < expected:
    failures.append(f"canonical_events outcome delta {canonical_outcome_delta} below expected operations {expected}")
if ledger_delta < expected * 2:
    failures.append(f"audit_outbox delta {ledger_delta} below expected decision+outcome rows {expected * 2}")
if ledger_decision_delta < expected:
    failures.append(f"audit_outbox decision delta {ledger_decision_delta} below expected operations {expected}")
if ledger_outcome_delta < expected:
    failures.append(f"audit_outbox outcome delta {ledger_outcome_delta} below expected operations {expected}")
if pending_after != 0:
    failures.append(f"pending audit_outbox rows after load: {pending_after}")

summary = {
    "result": "pass" if not failures else "fail",
    "date": datetime.now(timezone.utc).date().isoformat(),
    "finished_at": datetime.now(timezone.utc).isoformat(),
    "branch": os.environ["GIT_BRANCH"],
    "commit_sha": os.environ["GIT_COMMIT_SHA"],
    "git_dirty": bool(os.environ["GIT_SOURCE_STATUS"].strip()),
    "git_status": os.environ["GIT_SOURCE_STATUS"].splitlines(),
    "command_line": os.environ["COMMAND_LINE"].strip(),
    "machine_descriptor": os.environ["MACHINE_DESCRIPTOR"],
    "scenario_path": os.environ["SCENARIO"],
    "tenant_id": os.environ["TENANT_ID"],
    "load": load,
    "audit": {
        "canonical_before": int(os.environ["CANONICAL_BEFORE"]),
        "canonical_after": int(os.environ["CANONICAL_AFTER"]),
        "canonical_delta": canonical_delta,
        "canonical_decision_before": int(os.environ["CANONICAL_DECISION_BEFORE"]),
        "canonical_decision_after": int(os.environ["CANONICAL_DECISION_AFTER"]),
        "canonical_decision_delta": canonical_decision_delta,
        "canonical_outcome_before": int(os.environ["CANONICAL_OUTCOME_BEFORE"]),
        "canonical_outcome_after": int(os.environ["CANONICAL_OUTCOME_AFTER"]),
        "canonical_outcome_delta": canonical_outcome_delta,
        "ledger_outbox_before": int(os.environ["LEDGER_OUTBOX_BEFORE"]),
        "ledger_outbox_after": int(os.environ["LEDGER_OUTBOX_AFTER"]),
        "ledger_outbox_delta": ledger_delta,
        "ledger_outbox_decision_before": int(os.environ["LEDGER_OUTBOX_DECISION_BEFORE"]),
        "ledger_outbox_decision_after": int(os.environ["LEDGER_OUTBOX_DECISION_AFTER"]),
        "ledger_outbox_decision_delta": ledger_decision_delta,
        "ledger_outbox_outcome_before": int(os.environ["LEDGER_OUTBOX_OUTCOME_BEFORE"]),
        "ledger_outbox_outcome_after": int(os.environ["LEDGER_OUTBOX_OUTCOME_AFTER"]),
        "ledger_outbox_outcome_delta": ledger_outcome_delta,
        "pending_after": pending_after,
        "oldest_pending_lag_after_seconds": oldest_lag_after,
        "verify_audit_columns_status": verify_status,
        "outbox_drain_status": outbox_status,
    },
    "db_plan_status": plan_status,
    "failures": failures,
}
with open(summary_path, "w", encoding="utf-8") as fh:
    json.dump(summary, fh, indent=2, sort_keys=True)
    fh.write("\n")

lat = load.get("latency", {})
card = load.get("cardinality", {})
with open(command_path, "w", encoding="utf-8") as fh:
    fh.write("# GA_08 Command Results\n\n")
    fh.write(f"Date: {summary['date']}\n\n")
    fh.write("| Gate | Result | Evidence |\n|---|---|---|\n")
    fh.write(
        f"| `benchmarks/ga-load/run.sh --scenario benchmarks/ga-load/scenarios/local-100-tenants.yaml` | {summary['result'].upper()} | "
        f"ops {load.get('completed_operations')}/{expected}; logical tenants {card.get('logical_tenants')}; "
        f"providers {card.get('providers')}; canonical decision/outcome {canonical_decision_delta}/{canonical_outcome_delta}; "
        f"ledger decision/outcome {ledger_decision_delta}/{ledger_outcome_delta}; pending {pending_after}; failures {len(failures)} |\n"
    )
    for name in ["tokenizer", "output_predictor", "run_cost_projector", "sidecar_decision", "sidecar_confirm_publish_outcome", "sidecar_emit_trace_events", "end_to_end"]:
        item = lat.get(name, {})
        fh.write(
            f"| latency `{name}` | {'PASS' if item else 'FAIL'} | "
            f"count {item.get('count', 0)}, p50 {item.get('p50_ms', 0)}ms, "
            f"p95 {item.get('p95_ms', 0)}ms, p99 {item.get('p99_ms', 0)}ms, max {item.get('max_ms', 0)}ms |\n"
        )
    fh.write(
        f"| `python3 tests/e2e/verify_audit_columns.py --tenant {summary['tenant_id']}` | {'PASS' if verify_status == 0 else 'FAIL'} | "
        "`verify-audit-columns.txt` |\n"
    )
    fh.write(
        f"| `psql -d spendguard_canonical -f scripts/db/explain-ga-plans.sql` | {'PASS' if plan_status == 0 else 'FAIL'} | "
        "`explain-ga-plans.txt` |\n"
    )
PY

cat "$SUMMARY"
if [[ "$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["result"])' "$SUMMARY")" != "pass" ]]; then
  echo "[ga-load] FAIL: see $SUMMARY" >&2
  exit 1
fi

echo "[ga-load] PASS: evidence written to $SUMMARY"
