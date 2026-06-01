#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
COMPOSE=(docker compose -f "$ROOT/deploy/demo/compose.yaml")
TENANT_ID="${TENANT_ID:-00000000-0000-4000-8000-000000000001}"

DURATION="30m"
INTERVAL="60s"
PROFILE="local"
DEMO_MODE="default"
EVIDENCE_DIR="$ROOT/docs/reviews/ga-readiness/GA_07_soak_harness"
MAX_OUTBOX_LAG_SECONDS="${MAX_OUTBOX_LAG_SECONDS:-30}"
MAX_MEMORY_GROWTH_BYTES="${MAX_MEMORY_GROWTH_BYTES:-268435456}"
RESET_STACK=1

usage() {
  cat <<'USAGE'
Usage: scripts/soak/ga-soak.sh [options]

Options:
  --duration <Ns|Nm|Nh>      Soak duration. Default: 30m. Release gate: 24h.
  --interval <Ns|Nm|Nh>      Snapshot interval. Default: 60s.
  --profile <local>          Scenario profile. Default: local.
  --demo-mode <mode>         Demo mode used for initial runtime traffic. Default: default.
  --evidence-dir <path>      Evidence directory. Default: docs/reviews/ga-readiness/GA_07_soak_harness.
  --no-reset                 Reuse the current compose stack and skip replaying demo traffic.
  -h, --help                 Show this help.
USAGE
}

parse_duration() {
  local raw="$1"
  local n unit
  unit="${raw: -1}"
  case "$unit" in
    s|m|h) n="${raw%?}" ;;
    *) unit="s"; n="$raw" ;;
  esac
  if ! [[ "$n" =~ ^[0-9]+$ ]]; then
    echo "invalid duration: $raw" >&2
    exit 2
  fi
  case "$unit" in
    s) echo "$n" ;;
    m) echo $((n * 60)) ;;
    h) echo $((n * 3600)) ;;
  esac
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --duration) DURATION="$2"; shift 2 ;;
    --interval) INTERVAL="$2"; shift 2 ;;
    --profile) PROFILE="$2"; shift 2 ;;
    --demo-mode) DEMO_MODE="$2"; shift 2 ;;
    --evidence-dir) EVIDENCE_DIR="$2"; shift 2 ;;
    --no-reset) RESET_STACK=0; shift ;;
    -h|--help) usage; exit 0 ;;
    *) echo "unknown argument: $1" >&2; usage >&2; exit 2 ;;
  esac
done

if [[ "$PROFILE" != "local" ]]; then
  echo "unsupported profile: $PROFILE" >&2
  exit 2
fi

DURATION_SECONDS="$(parse_duration "$DURATION")"
INTERVAL_SECONDS="$(parse_duration "$INTERVAL")"
if [[ "$DURATION_SECONDS" -lt "$INTERVAL_SECONDS" ]]; then
  echo "--duration must be >= --interval" >&2
  exit 2
fi

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

metric_sum() {
  local url="$1"
  local metric="$2"
  curl -fsS "$url" | awk -v metric="$metric" '
    $1 ~ ("^" metric "(\\{|$)") { sum += $2 }
    END { printf "%.0f\n", sum + 0 }
  '
}

metric_gauge() {
  local url="$1"
  local metric="$2"
  curl -fsS "$url" | awk -v metric="$metric" '
    $1 ~ ("^" metric "(\\{|$)") { value = $2; found = 1 }
    END { if (found) printf "%.0f\n", value; else print 0 }
  '
}

canonical_metric_sum() {
  local metric="$1"
  "${COMPOSE[@]}" exec -T canonical-ingest \
    wget -q -O - http://127.0.0.1:9091/metrics | awk -v metric="$metric" '
      $1 ~ ("^" metric "(\\{|$)") { sum += $2 }
      END { printf "%.0f\n", sum + 0 }
    '
}

wait_for_http() {
  local url="$1"
  local label="$2"
  for _ in $(seq 1 90); do
    if curl -fsS "$url" >/dev/null 2>&1; then
      return 0
    fi
    sleep 2
  done
  echo "timed out waiting for $label at $url" >&2
  return 1
}

wait_for_canonical_metrics() {
  for _ in $(seq 1 90); do
    if "${COMPOSE[@]}" exec -T canonical-ingest \
      wget -q -O - http://127.0.0.1:9091/metrics >/dev/null 2>&1; then
      return 0
    fi
    sleep 2
  done
  echo "timed out waiting for canonical-ingest metrics inside compose" >&2
  return 1
}

wait_for_metric_positive() {
  local url="$1"
  local metric="$2"
  local label="$3"
  local value
  for _ in $(seq 1 120); do
    value="$(metric_sum "$url" "$metric" || echo 0)"
    if [[ "$value" -gt 0 ]]; then
      return 0
    fi
    sleep 2
  done
  echo "timed out waiting for $label metric $metric to become positive" >&2
  return 1
}

run_svid_probe() {
  python3 - "$ROOT" "$TENANT_ID" <<'PY'
import sys
root, tenant = sys.argv[1], sys.argv[2]
sys.path.insert(0, f"{root}/contrib/output_predictor_template")
from svid_validation import expected_svid_subject, tenant_from_svid_subject, validate_auth_context_tenant

subject = expected_svid_subject(tenant)
assert tenant_from_svid_subject(subject) == tenant
validate_auth_context_tenant(
    auth_context={"x509_subject_alternative_name": [f"URI:{subject}".encode()]},
    tenant_id=tenant,
    require_svid=True,
)
try:
    validate_auth_context_tenant(
        auth_context={"x509_subject_alternative_name": [b"URI:spiffe://spendguard.platform/predictor-client/018fcf9a-3d2d-7b37-9f21-0f27de0b20c1"]},
        tenant_id=tenant,
        require_svid=True,
    )
except ValueError:
    pass
else:
    raise AssertionError("mismatched tenant SVID did not fail closed")
print(subject)
PY
}

run_verify_audit_columns() {
  python3 "$ROOT/tests/e2e/verify_audit_columns.py" --tenant "$TENANT_ID"
}

append_snapshot() {
  local index="$1"
  local started_unix="$2"
  local baseline_file="$3"
  local now_unix
  local elapsed
  local ledger_out
  local canonical_out
  local cache_out
  local run_cache_out
  local outbox_lag
  local leader_count
  local dedup_total
  local stats_cycles
  local stats_errors
  local stats_last_cycle
  local tokenizer_escalations
  local svid_output
  local svid_status=0
  local verify_output
  local verify_status=0
  local inspect_output
  local stats_output

  now_unix="$(date -u +%s)"
  elapsed=$((now_unix - started_unix))

  ledger_out="$(psql_db spendguard_ledger "SELECT count(*) FILTER (WHERE pending_forward = TRUE)::int, COALESCE(EXTRACT(EPOCH FROM (clock_timestamp() - min(recorded_at)))::bigint, 0) FROM audit_outbox WHERE pending_forward = TRUE;")"
  canonical_out="$(psql_db spendguard_canonical "SELECT count(*)::int, COALESCE(EXTRACT(EPOCH FROM (clock_timestamp() - max(ingest_at)))::bigint, 999999) FROM canonical_events WHERE tenant_id = '$TENANT_ID';")"
  cache_out="$(psql_db spendguard_canonical "SELECT set_config('app.current_tenant_id', '$TENANT_ID', true); SELECT count(*)::int, COALESCE(EXTRACT(EPOCH FROM (clock_timestamp() - max(computed_at)))::bigint, 999999) FROM output_distribution_cache WHERE tenant_id = '$TENANT_ID';" | tail -n1)"
  run_cache_out="$(psql_db spendguard_canonical "SELECT set_config('app.current_tenant_id', '$TENANT_ID', true); SELECT count(*)::int, COALESCE(EXTRACT(EPOCH FROM (clock_timestamp() - max(computed_at)))::bigint, 999999) FROM run_length_distribution_cache WHERE tenant_id = '$TENANT_ID';" | tail -n1)"

  outbox_lag="$(metric_gauge "http://127.0.0.1:9096/metrics" "spendguard_outbox_pending_oldest_age_seconds")"
  leader_count="$(metric_sum "http://127.0.0.1:9096/metrics" "spendguard_outbox_forwarder_is_leader")"
  dedup_total="$(canonical_metric_sum "spendguard_ingest_events_deduped_total")"
  stats_cycles="$(metric_sum "http://127.0.0.1:9101/metrics" "spendguard_stats_aggregator_cycles_total")"
  stats_errors="$(metric_sum "http://127.0.0.1:9101/metrics" "spendguard_stats_aggregator_cycle_error_total")"
  stats_last_cycle="$(metric_gauge "http://127.0.0.1:9101/metrics" "spendguard_stats_aggregator_last_cycle_start_unix_secs")"
  tokenizer_escalations="$(metric_sum "http://127.0.0.1:9099/metrics" "spendguard_tokenizer_drift_alert_oncall_escalation_total")"

  if svid_output="$(run_svid_probe 2>&1)"; then
    svid_status=0
  else
    svid_status=$?
  fi

  if verify_output="$(run_verify_audit_columns 2>&1)"; then
    verify_status=0
  else
    verify_status=$?
  fi

  inspect_output="$(docker inspect --format '{{json .}}' \
    spendguard-postgres spendguard-ledger spendguard-canonical-ingest spendguard-sidecar \
    spendguard-outbox-forwarder spendguard-stats-aggregator spendguard-output-predictor \
    spendguard-run-cost-projector spendguard-tokenizer spendguard-control-plane)"
  stats_output="$(docker stats --no-stream --format '{{json .}}' \
    spendguard-postgres spendguard-ledger spendguard-canonical-ingest spendguard-sidecar \
    spendguard-outbox-forwarder spendguard-stats-aggregator spendguard-output-predictor \
    spendguard-run-cost-projector spendguard-tokenizer spendguard-control-plane)"

  SNAPSHOT_INDEX="$index" \
  SNAPSHOT_UNIX="$now_unix" \
  ELAPSED_SECONDS="$elapsed" \
  LEDGER_OUT="$ledger_out" \
  CANONICAL_OUT="$canonical_out" \
  CACHE_OUT="$cache_out" \
  RUN_CACHE_OUT="$run_cache_out" \
  OUTBOX_LAG="$outbox_lag" \
  LEADER_COUNT="$leader_count" \
  DEDUP_TOTAL="$dedup_total" \
  STATS_CYCLES="$stats_cycles" \
  STATS_ERRORS="$stats_errors" \
  STATS_LAST_CYCLE="$stats_last_cycle" \
  TOKENIZER_ESCALATIONS="$tokenizer_escalations" \
  SVID_STATUS="$svid_status" \
  SVID_OUTPUT="$svid_output" \
  VERIFY_STATUS="$verify_status" \
  VERIFY_OUTPUT="$verify_output" \
  DOCKER_INSPECT="$inspect_output" \
  DOCKER_STATS="$stats_output" \
  BASELINE_FILE="$baseline_file" \
  SNAPSHOTS_FILE="$SNAPSHOTS_FILE" \
  MAX_OUTBOX_LAG_SECONDS="$MAX_OUTBOX_LAG_SECONDS" \
  MAX_MEMORY_GROWTH_BYTES="$MAX_MEMORY_GROWTH_BYTES" \
  python3 - <<'PY'
import json
import os
import re
import sys
from datetime import datetime, timezone
from pathlib import Path


def split_pair(value: str) -> tuple[int, int]:
    left, right = (value.strip().split("|", 1) + ["0"])[:2]
    return int(left or 0), int(right or 0)


def parse_bytes(value: str) -> int:
    number, unit = re.match(r"\s*([0-9.]+)\s*([A-Za-z]+)", value).groups()
    unit = unit.lower()
    multipliers = {
        "b": 1,
        "kb": 1000,
        "kib": 1024,
        "mb": 1000**2,
        "mib": 1024**2,
        "gb": 1000**3,
        "gib": 1024**3,
    }
    return int(float(number) * multipliers[unit])


def parse_stats(raw: str) -> dict[str, dict[str, object]]:
    out = {}
    for line in raw.splitlines():
        item = json.loads(line)
        name = item["Name"]
        usage = item["MemUsage"]
        current = usage.split("/", 1)[0].strip()
        out[name] = {"memory_raw": usage, "memory_bytes": parse_bytes(current)}
    return out


def parse_inspect(raw: str) -> dict[str, dict[str, object]]:
    out = {}
    for line in raw.splitlines():
        item = json.loads(line)
        state = item.get("State") or {}
        health_state = state.get("Health") or {}
        out[item["Name"].lstrip("/")] = {
            "restart_count": int(item.get("RestartCount") or 0),
            "status": state.get("Status") or "unknown",
            "health": health_state.get("Status") or "none",
        }
    return out


snapshot_unix = int(os.environ["SNAPSHOT_UNIX"])
pending, pending_age = split_pair(os.environ["LEDGER_OUT"])
canonical_count, canonical_freshness = split_pair(os.environ["CANONICAL_OUT"])
output_cache_count, output_cache_age = split_pair(os.environ["CACHE_OUT"])
run_cache_count, run_cache_age = split_pair(os.environ["RUN_CACHE_OUT"])
docker_stats = parse_stats(os.environ["DOCKER_STATS"])
docker_inspect = parse_inspect(os.environ["DOCKER_INSPECT"])

baseline_path = Path(os.environ["BASELINE_FILE"])
if baseline_path.exists():
    baseline = json.loads(baseline_path.read_text(encoding="utf-8"))
else:
    baseline = {
        "created_at": datetime.fromtimestamp(snapshot_unix, timezone.utc).isoformat(),
        "memory_bytes": {name: item["memory_bytes"] for name, item in docker_stats.items()},
    }
    baseline_path.write_text(json.dumps(baseline, indent=2, sort_keys=True) + "\n", encoding="utf-8")

failures = []
max_outbox_lag = int(os.environ["MAX_OUTBOX_LAG_SECONDS"])
max_memory_growth = int(os.environ["MAX_MEMORY_GROWTH_BYTES"])

if pending != 0:
    failures.append(f"audit_outbox pending rows not drained: {pending}")
if int(os.environ["OUTBOX_LAG"]) > max_outbox_lag:
    failures.append(f"outbox lag {os.environ['OUTBOX_LAG']}s exceeds {max_outbox_lag}s")
if int(os.environ["LEADER_COUNT"]) != 1:
    failures.append(f"outbox leader count is {os.environ['LEADER_COUNT']}, expected 1")
if canonical_count <= 0:
    failures.append("canonical_events count is zero")
if canonical_freshness > 600:
    failures.append(f"canonical_events freshest row age {canonical_freshness}s exceeds 600s")
if int(os.environ["STATS_CYCLES"]) <= 0:
    failures.append("stats_aggregator has not completed a cycle")
if int(os.environ["STATS_ERRORS"]) != 0:
    failures.append(f"stats_aggregator cycle errors = {os.environ['STATS_ERRORS']}")
last_cycle = int(os.environ["STATS_LAST_CYCLE"])
if last_cycle <= 0 or snapshot_unix - last_cycle > 180:
    failures.append(f"stats_aggregator last cycle is stale: {snapshot_unix - last_cycle if last_cycle else 'never'}s")
if int(os.environ["SVID_STATUS"]) != 0:
    failures.append("SVID subject probe failed")
if int(os.environ["VERIFY_STATUS"]) != 0:
    failures.append("verify_audit_columns/verify-chain probe failed")

for name, info in docker_inspect.items():
    if info["status"] != "running":
        failures.append(f"{name} status is {info['status']}")
    if info["health"] not in {"healthy", "none"}:
        failures.append(f"{name} health is {info['health']}")

for name, current in docker_stats.items():
    start = baseline["memory_bytes"].get(name)
    if start is None:
        continue
    growth = current["memory_bytes"] - int(start)
    current["growth_bytes"] = growth
    if growth > max_memory_growth:
        failures.append(f"{name} memory grew by {growth} bytes")

snapshot = {
    "index": int(os.environ["SNAPSHOT_INDEX"]),
    "timestamp": datetime.fromtimestamp(snapshot_unix, timezone.utc).isoformat(),
    "elapsed_seconds": int(os.environ["ELAPSED_SECONDS"]),
    "audit": {
        "pending_forward_rows": pending,
        "pending_oldest_age_db_seconds": pending_age,
        "outbox_lag_metric_seconds": int(os.environ["OUTBOX_LAG"]),
        "outbox_leader_count": int(os.environ["LEADER_COUNT"]),
        "canonical_events": canonical_count,
        "canonical_freshest_age_seconds": canonical_freshness,
        "dedup_total": int(os.environ["DEDUP_TOTAL"]),
        "verify_status": int(os.environ["VERIFY_STATUS"]),
    },
    "stats": {
        "cycles_total": int(os.environ["STATS_CYCLES"]),
        "cycle_errors_total": int(os.environ["STATS_ERRORS"]),
        "last_cycle_age_seconds": snapshot_unix - last_cycle if last_cycle else None,
        "output_distribution_cache_rows": output_cache_count,
        "output_distribution_cache_freshness_seconds": output_cache_age,
        "run_length_distribution_cache_rows": run_cache_count,
        "run_length_distribution_cache_freshness_seconds": run_cache_age,
    },
    "plugin_cert": {
        "svid_probe_status": int(os.environ["SVID_STATUS"]),
        "svid_subject": os.environ["SVID_OUTPUT"].strip().splitlines()[-1] if os.environ["SVID_OUTPUT"].strip() else "",
        "tokenizer_drift_escalations_total": int(os.environ["TOKENIZER_ESCALATIONS"]),
    },
    "containers": {
        name: {**docker_inspect.get(name, {}), **docker_stats.get(name, {})}
        for name in sorted(set(docker_inspect) | set(docker_stats))
    },
    "failures": failures,
}

with Path(os.environ["SNAPSHOTS_FILE"]).open("a", encoding="utf-8") as fh:
    fh.write(json.dumps(snapshot, sort_keys=True) + "\n")

if failures:
    print(json.dumps({"snapshot": snapshot["index"], "failures": failures}, indent=2), file=sys.stderr)
    if os.environ["VERIFY_OUTPUT"].strip():
        print(os.environ["VERIFY_OUTPUT"], file=sys.stderr)
    if os.environ["SVID_OUTPUT"].strip():
        print(os.environ["SVID_OUTPUT"], file=sys.stderr)
    sys.exit(1)

print(
    f"[soak] snapshot {snapshot['index']} ok: elapsed={snapshot['elapsed_seconds']}s "
    f"canonical={canonical_count} pending={pending} lag={os.environ['OUTBOX_LAG']}s "
    f"stats_cycles={os.environ['STATS_CYCLES']}"
)
PY
}

write_summary() {
  local started_at="$1"
  local finished_at="$2"
  local result="$3"
  STARTED_AT="$started_at" \
  FINISHED_AT="$finished_at" \
  BOOT_STARTED_AT="$BOOT_STARTED_AT" \
  RESULT="$result" \
  DURATION_SECONDS="$DURATION_SECONDS" \
  INTERVAL_SECONDS="$INTERVAL_SECONDS" \
  PROFILE="$PROFILE" \
  DEMO_MODE="$DEMO_MODE" \
  SNAPSHOTS_FILE="$SNAPSHOTS_FILE" \
  SUMMARY_FILE="$SUMMARY_FILE" \
  python3 - <<'PY'
import json
import os
from pathlib import Path

snapshots_path = Path(os.environ["SNAPSHOTS_FILE"])
snapshots = [
    json.loads(line)
    for line in snapshots_path.read_text(encoding="utf-8").splitlines()
    if line.strip()
]
failures = [f for snapshot in snapshots for f in snapshot.get("failures", [])]
summary = {
    "result": os.environ["RESULT"],
    "boot_started_at": os.environ["BOOT_STARTED_AT"],
    "started_at": os.environ["STARTED_AT"],
    "finished_at": os.environ["FINISHED_AT"],
    "duration_seconds": int(os.environ["DURATION_SECONDS"]),
    "interval_seconds": int(os.environ["INTERVAL_SECONDS"]),
    "profile": os.environ["PROFILE"],
    "demo_mode": os.environ["DEMO_MODE"],
    "snapshot_count": len(snapshots),
    "failures": failures,
    "first_snapshot": snapshots[0] if snapshots else None,
    "last_snapshot": snapshots[-1] if snapshots else None,
}
Path(os.environ["SUMMARY_FILE"]).write_text(
    json.dumps(summary, indent=2, sort_keys=True) + "\n",
    encoding="utf-8",
)
print(os.environ["SUMMARY_FILE"])
PY
}

require_cmd docker
require_cmd curl
require_cmd awk
require_cmd python3

cd "$ROOT"
mkdir -p "$EVIDENCE_DIR"
SNAPSHOTS_FILE="$EVIDENCE_DIR/ga_soak_snapshots.jsonl"
SUMMARY_FILE="$EVIDENCE_DIR/ga_soak_summary.json"
BASELINE_FILE="$EVIDENCE_DIR/ga_soak_baseline.json"
rm -f "$SNAPSHOTS_FILE" "$SUMMARY_FILE" "$BASELINE_FILE"

BOOT_STARTED_AT="$(date -u +"%Y-%m-%dT%H:%M:%SZ")"

echo "[soak] profile=$PROFILE duration=${DURATION_SECONDS}s interval=${INTERVAL_SECONDS}s demo_mode=$DEMO_MODE"
if [[ "$RESET_STACK" -eq 1 ]]; then
  echo "[soak] resetting demo stack"
  make demo-down
fi

if [[ "$RESET_STACK" -eq 1 ]]; then
  echo "[soak] booting demo traffic path"
  make demo-up DEMO_MODE="$DEMO_MODE"
else
  echo "[soak] reusing current demo stack; skipping demo traffic replay"
fi

echo "[soak] starting GA predictor/ops services"
"${COMPOSE[@]}" up -d --build output-predictor run-cost-projector stats-aggregator tokenizer control-plane

wait_for_canonical_metrics
wait_for_http "http://127.0.0.1:9096/metrics" "outbox-forwarder metrics"
wait_for_http "http://127.0.0.1:9100/healthz" "output-predictor healthz"
wait_for_http "http://127.0.0.1:9102/livez" "run-cost-projector livez"
wait_for_http "http://127.0.0.1:9101/healthz" "stats-aggregator healthz"
wait_for_http "http://127.0.0.1:9099/healthz" "tokenizer healthz"
wait_for_http "http://127.0.0.1:9094/metrics" "control-plane metrics"
wait_for_metric_positive "http://127.0.0.1:9101/metrics" "spendguard_stats_aggregator_cycles_total" "stats-aggregator cycle"

echo "[soak] validating real SVID/mTLS test once before sustained snapshots"
cargo test --manifest-path services/output_predictor/Cargo.toml --test plugin_svid_mtls -- --nocapture
python3 -m pytest contrib/output_predictor_template/conformance_test.py -q -k 'client_svid'

STARTED_AT="$(date -u +"%Y-%m-%dT%H:%M:%SZ")"
STARTED_UNIX="$(date -u +%s)"
INDEX=0
RESULT="pass"
while true; do
  append_snapshot "$INDEX" "$STARTED_UNIX" "$BASELINE_FILE" || RESULT="fail"
  if [[ "$RESULT" != "pass" ]]; then
    break
  fi
  NOW_UNIX="$(date -u +%s)"
  ELAPSED=$((NOW_UNIX - STARTED_UNIX))
  if [[ "$ELAPSED" -ge "$DURATION_SECONDS" ]]; then
    break
  fi
  INDEX=$((INDEX + 1))
  sleep "$INTERVAL_SECONDS"
done

FINISHED_AT="$(date -u +"%Y-%m-%dT%H:%M:%SZ")"
write_summary "$STARTED_AT" "$FINISHED_AT" "$RESULT"

if [[ "$RESULT" != "pass" ]]; then
  echo "[soak] FAIL: see $SUMMARY_FILE" >&2
  exit 1
fi

echo "[soak] PASS: evidence written to $SUMMARY_FILE and $SNAPSHOTS_FILE"
