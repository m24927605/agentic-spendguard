#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"

python3 - <<'PY'
import json
import pathlib
import re
import sys

root = pathlib.Path.cwd()
dashboard_path = root / "deploy/observability/grafana-dashboard.json"
inventory_path = root / "docs/operations/metrics-inventory.md"

errors: list[str] = []

try:
    dashboard = json.loads(dashboard_path.read_text())
except Exception as exc:  # noqa: BLE001
    print(f"dashboard JSON parse failed: {exc}", file=sys.stderr)
    sys.exit(1)

inventory_rows: list[tuple[str, list[str]]] = []
for line_no, raw in enumerate(inventory_path.read_text().splitlines(), start=1):
    if not raw.startswith("| `"):
        continue
    cells = [cell.strip() for cell in raw.strip().strip("|").split("|")]
    if len(cells) != 7:
        errors.append(
            f"metrics inventory row {line_no} has {len(cells)} cells; expected 7. "
            "Avoid raw pipe characters inside cells."
        )
        continue
    inventory_rows.append((raw, cells))

inventory: dict[str, pathlib.Path] = {}
inventory_endpoints: dict[str, str] = {}
inventory_labels: dict[str, str] = {}
for raw, cells in inventory_rows:
    metric_match = re.fullmatch(r"`([^`]+)`", cells[0])
    source_match = re.fullmatch(r"`([^`]+)`", cells[3])
    if metric_match and source_match:
        metric = metric_match.group(1)
        inventory[metric] = root / source_match.group(1)
        endpoint_match = re.fullmatch(r"`([^`]+)`", cells[2])
        if endpoint_match:
            inventory_endpoints[metric] = endpoint_match.group(1)
        inventory_labels[metric] = cells[4].lower()

if not inventory:
    errors.append("metrics inventory did not yield any metric/source rows")

expressions: list[str] = []

def collect_exprs(value) -> None:
    if isinstance(value, dict):
        for key, child in value.items():
            if key == "expr" and isinstance(child, str):
                expressions.append(child)
            else:
                collect_exprs(child)
    elif isinstance(value, list):
        for child in value:
            collect_exprs(child)

collect_exprs(dashboard)

if not expressions:
    errors.append("dashboard contains no PromQL expressions")

metric_pattern = re.compile(r"\b(?:spendguard|customer)_[A-Za-z0-9_:]*\b")
dashboard_metrics = set()
for expr in expressions:
    if "vector(0)" in expr or "or vector(0)" in expr:
        errors.append(f"placeholder vector detected in dashboard expression: {expr}")
    dashboard_metrics.update(metric_pattern.findall(expr))

for metric in sorted(dashboard_metrics):
    source = inventory.get(metric)
    if source is None:
        errors.append(f"dashboard metric `{metric}` missing from docs/operations/metrics-inventory.md")
        continue
    if not source.exists():
        errors.append(f"source path for `{metric}` does not exist: {source.relative_to(root)}")
        continue
    if metric not in source.read_text():
        errors.append(f"source path for `{metric}` does not contain metric string: {source.relative_to(root)}")

for metric, labels in sorted(inventory_labels.items()):
    for forbidden in [
        "tenant_id",
        "run_id",
        "decision_id",
        "agent_id",
        "model",
        "prompt",
        "prompt_text",
    ]:
        if re.search(rf"\b{re.escape(forbidden)}\b", labels):
            errors.append(
                f"forbidden high-cardinality/PII label documented for `{metric}`: {forbidden}"
            )

expected_endpoints = {
    "canonical_ingest": ":9091/metrics",
    "ledger": ":9092/metrics",
    "control_plane": ":9094/metrics",
    "outbox_forwarder": ":9096/metrics",
    "tokenizer": ":9099/metrics",
    "output_predictor": ":9100/metrics",
    "stats_aggregator": ":9101/metrics",
    "run_cost_projector": ":9102/metrics",
}
for _raw, cells in inventory_rows:
    metric_match = re.fullmatch(r"`([^`]+)`", cells[0])
    endpoint_match = re.fullmatch(r"`([^`]+)`", cells[2])
    if not metric_match or not endpoint_match:
        continue
    metric = metric_match.group(1)
    service = cells[1]
    expected = expected_endpoints.get(service)
    if expected is None:
        errors.append(f"`{metric}` has unknown service in inventory: {service}")
        continue
    got = endpoint_match.group(1)
    if got != expected:
        errors.append(
            f"`{metric}` endpoint is {got}, expected {expected} for service {service}"
        )

expr_text = "\n".join(expressions)
required_expr_fragments = [
    "histogram_quantile(0.99, sum(rate(spendguard_output_predictor_predict_latency_seconds_bucket[5m])) by (le))",
    "histogram_quantile(0.99, sum(rate(spendguard_run_cost_projector_project_latency_seconds_bucket[5m])) by (le))",
    "spendguard_outbox_pending_oldest_age_seconds",
    "spendguard_ingest_events_deduped_total",
    "customer_predictor_failure_mode_total{mode=\"tls_error\"}",
]
for fragment in required_expr_fragments:
    if fragment not in expr_text:
        errors.append(f"required GA dashboard expression fragment missing: {fragment}")

placeholder_checks = [
    ("services/output_predictor/src/main.rs", "spendguard_output_predictor_predict_total 0"),
    ("services/output_predictor/src/main.rs", "spendguard_output_predictor_cache_hit_rate 0"),
    ("services/run_cost_projector/src/main.rs", "spendguard_run_cost_projector_project_total 0"),
    ("services/run_cost_projector/src/main.rs", "spendguard_run_cost_projector_terminate_run_total 0"),
]
for rel, needle in placeholder_checks:
    if needle in (root / rel).read_text():
        errors.append(f"legacy placeholder metric still present in {rel}: {needle}")

if errors:
    for err in errors:
        print(f"ERROR: {err}", file=sys.stderr)
    sys.exit(1)

print(
    f"dashboard metrics validated: {len(dashboard_metrics)} metrics, {len(expressions)} expressions"
)
PY
