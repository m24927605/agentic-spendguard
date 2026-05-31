#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
RULES_FILE="${1:-$ROOT/deploy/observability/prometheus-rules.yaml}"
INVENTORY_FILE="$ROOT/docs/operations/metrics-inventory.md"

python3 - "$ROOT" "$RULES_FILE" "$INVENTORY_FILE" <<'PY'
import pathlib
import re
import sys

try:
    import yaml
except ImportError as exc:
    raise SystemExit("PyYAML is required for alert/runbook validation") from exc

root = pathlib.Path(sys.argv[1])
rules_path = pathlib.Path(sys.argv[2])
inventory_path = pathlib.Path(sys.argv[3])

required_headings = [
    "## Detection",
    "## Diagnosis",
    "## Mitigation",
    "## Rollback",
    "## Evidence",
    "## Safety",
]
unsafe_phrases = [
    "delete from audit_outbox",
    "truncate audit_outbox",
    "drop table audit_outbox",
    "bypassrls",
    "disable rls",
    "turn off strict_signatures",
    "set strict_signatures=false",
]
metric_re = re.compile(r"\b(?:spendguard|customer)_[A-Za-z0-9_:]*[A-Za-z0-9_]\b")


def fail(message: str) -> None:
    raise SystemExit(f"alert-runbook validation failed: {message}")


def repo_path(value: str) -> pathlib.Path:
    if value.startswith("http://") or value.startswith("https://"):
        fail(f"runbook links must be repo-local during GA validation: {value}")
    path = pathlib.Path(value)
    return path if path.is_absolute() else root / path


if not rules_path.exists():
    fail(f"missing rules file {rules_path}")
if not inventory_path.exists():
    fail(f"missing metrics inventory {inventory_path}")

data = yaml.safe_load(rules_path.read_text(encoding="utf-8"))
if not isinstance(data, dict):
    fail("rules file did not parse as a YAML mapping")
if data.get("apiVersion") != "monitoring.coreos.com/v1":
    fail("rules file is not a PrometheusRule CRD")
if data.get("kind") != "PrometheusRule":
    fail("rules file kind must be PrometheusRule")

groups = data.get("spec", {}).get("groups")
if not isinstance(groups, list) or not groups:
    fail("PrometheusRule must contain spec.groups")

inventory = {}
for line in inventory_path.read_text(encoding="utf-8").splitlines():
    if not line.startswith("| `"):
        continue
    cells = [cell.strip() for cell in line.strip().strip("|").split("|")]
    if len(cells) < 4:
        continue
    metric = cells[0].strip("`")
    source = cells[3].strip("`")
    inventory[metric] = source

if not inventory:
    fail("metrics inventory did not expose any metric rows")

alerts = []
runbooks = set()
drills = set()
metrics_seen = set()
inventory_text = inventory_path.read_text(encoding="utf-8")

for group in groups:
    if not isinstance(group, dict) or not group.get("name"):
        fail("every alert group must be a named mapping")
    rules = group.get("rules")
    if not isinstance(rules, list) or not rules:
        fail(f"group {group.get('name')} has no rules")
    for rule in rules:
        if not isinstance(rule, dict) or "alert" not in rule:
            fail(f"group {group.get('name')} contains a non-alert rule")
        alert = rule["alert"]
        expr = rule.get("expr")
        if not isinstance(expr, str) or not expr.strip():
            fail(f"{alert} has no expression")
        if "vector(0)" in expr:
            fail(f"{alert} uses placeholder vector(0)")
        if "spendguard_outbox_pending_seconds_bucket" in expr:
            fail(f"{alert} uses stale outbox histogram placeholder")
        if alert == "SpendGuardOutboxNoLeader" and "absent(" not in expr:
            fail("SpendGuardOutboxNoLeader must alert when all leader metric series are absent")
        labels = rule.get("labels") or {}
        severity = labels.get("severity")
        if severity not in {"page", "warn", "info"}:
            fail(f"{alert} must set severity to page, warn, or info")
        annotations = rule.get("annotations") or {}
        runbook = annotations.get("runbook")
        if not runbook:
            fail(f"{alert} has no runbook annotation")
        runbook_path = repo_path(str(runbook))
        if not runbook_path.exists():
            fail(f"{alert} references missing runbook {runbook}")
        runbook_text = runbook_path.read_text(encoding="utf-8")
        for heading in required_headings:
            if heading not in runbook_text:
                fail(f"{runbook} is missing required heading {heading}")
        lower_runbook = runbook_text.lower()
        for phrase in unsafe_phrases:
            if phrase in lower_runbook:
                fail(f"{runbook} contains unsafe mitigation phrase: {phrase}")
        runbooks.add(runbook_path)

        drill = annotations.get("drill_runbook")
        if drill:
            drill_path = repo_path(str(drill))
            if not drill_path.exists():
                fail(f"{alert} references missing drill {drill}")
            drills.add(drill_path)

        expr_metrics = set(metric_re.findall(expr))
        if not expr_metrics:
            fail(f"{alert} expression does not reference a SpendGuard/customer metric")
        for metric in expr_metrics:
            if metric not in inventory:
                fail(f"{alert} references {metric}, which is not in metrics inventory")
            source = root / inventory[metric]
            if not source.exists():
                fail(f"{metric} inventory source is missing: {inventory[metric]}")
            source_text = source.read_text(encoding="utf-8", errors="replace")
            if metric not in source_text:
                fail(f"{metric} not found in inventory source {inventory[metric]}")
            if f"`{metric}`" not in inventory_text:
                fail(f"{metric} not listed in metrics inventory table")
            metrics_seen.add(metric)

        alerts.append(alert)

if len(alerts) != len(set(alerts)):
    fail("duplicate alert names found")

print(
    "alert runbooks validated: "
    f"{len(alerts)} alerts, {len(runbooks)} runbooks, {len(drills)} drills, "
    f"{len(metrics_seen)} metrics"
)
PY
