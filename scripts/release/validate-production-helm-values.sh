#!/usr/bin/env bash
set -euo pipefail

values_file="charts/spendguard/values-production.example.yaml"
skip_negative_tests=false
rendered_manifest=""

usage() {
  cat <<'USAGE'
Usage:
  scripts/release/validate-production-helm-values.sh [VALUES_FILE] [--skip-negative-tests] [--rendered-manifest FILE]

Validates the SpendGuard production Helm values example and rendered manifest.
USAGE
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --skip-negative-tests)
      skip_negative_tests=true
      shift
      ;;
    --rendered-manifest)
      rendered_manifest="${2:-}"
      shift 2
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      values_file="$1"
      shift
      ;;
  esac
done

if [[ ! -f "$values_file" ]]; then
  echo "values file does not exist: $values_file" >&2
  exit 1
fi

render_file=""
if [[ -n "$rendered_manifest" ]]; then
  if [[ ! -f "$rendered_manifest" ]]; then
    echo "rendered manifest does not exist: $rendered_manifest" >&2
    exit 1
  fi
  render_file="$rendered_manifest"
else
  render_file="$(mktemp)"
  trap "rm -f '$render_file'" EXIT
  helm template spendguard charts/spendguard -f "$values_file" > "$render_file"
fi

python3 - "$values_file" "$render_file" <<'PY'
from pathlib import Path
import re
import sys

import yaml

values_path = Path(sys.argv[1])
render_path = Path(sys.argv[2])

raw_values = values_path.read_text(encoding="utf-8")
values = yaml.safe_load(raw_values) or {}
docs = [doc for doc in yaml.safe_load_all(render_path.read_text(encoding="utf-8")) if doc]


def fail(message: str) -> None:
    print(message, file=sys.stderr)
    sys.exit(1)


def get(mapping, path, default=None):
    current = mapping
    for part in path.split("."):
        if not isinstance(current, dict) or part not in current:
            return default
        current = current[part]
    return current


def require_string(path: str) -> str:
    value = get(values, path)
    if not isinstance(value, str) or not value.strip():
        fail(f"{path} must be a non-empty string")
    if re.search(r"(?i)\b(postgres(?:ql)?|mysql|mongodb)://", value):
        fail(f"{path} must be a Secret name/key, not a plaintext URL")
    return value


def require_sha256_hex(path: str) -> str:
    value = require_string(path)
    if not re.fullmatch(r"[0-9a-f]{64}", value):
        fail(f"{path} must be a lowercase 64-character SHA-256 hex value")
    if value == "0" * 64:
        fail(f"{path} must not be the all-zero placeholder")
    return value


if get(values, "chart.profile") != "production":
    fail("chart.profile must be production")

if re.search(r"(?i)\b(postgres(?:ql)?|mysql|mongodb)://", raw_values):
    fail("production values must not contain plaintext database URLs")

required_value_refs = [
    "secrets.tls.existingSecret",
    "secrets.bundles.existingSecret",
    "postgres.existingSecret",
    "postgres.ledgerUrlKey",
    "postgres.canonicalUrlKey",
    "postgres.controlPlaneUrlKey",
    "postgres.controlPlaneAuditForwarderUrlKey",
    "postgres.tokenizerShadowUrlKey",
    "postgres.tokenizerUrlKey",
    "postgres.outputPredictorUrlKey",
    "postgres.outputPredictorPluginEndpointUrlKey",
    "postgres.runCostProjectorUrlKey",
    "postgres.statsAggregatorUrlKey",
    "sidecar.trustSecret.name",
    "sidecar.trustSecret.caPemKey",
    "sidecar.mtlsBootstrapTokenSecret.name",
    "sidecar.mtlsBootstrapTokenSecret.tokenKey",
    "sidecar.manifestVerifyKey.existingSecret",
    "sidecar.manifestVerifyKey.pemKey",
    "signing.existingSecret",
    "webhookReceiver.hmacSecretName",
]
for path in required_value_refs:
    require_string(path)

for path in [
    "sidecar.contractBundleHashHex",
    "sidecar.trustRootSpkiSha256Hex",
    "controlPlane.auditSchemaBundleHashHex",
    "outboxForwarder.schemaBundleHashHex",
    "statsAggregator.schemaBundleHashHex",
]:
    require_sha256_hex(path)

if get(values, "tokenizer.shadowEnabled") is True:
    require_sha256_hex("tokenizer.schemaBundleHashHex")

if get(values, "networkPolicy.enabled") is not True:
    fail("networkPolicy.enabled must be true in the production example")

if get(values, "tokenizer.shadowEnabled") is True:
    for path in [
        "tokenizer.providerSecretName",
        "tokenizer.sinkMtlsSecretName",
        "tokenizer.signingKeySecretName",
        "tokenizer.signingKeyPath",
        "tokenizer.canonicalIngestUrl",
    ]:
        require_string(path)

if get(values, "outputPredictor.pluginEndpointDatabaseEnabled") is True:
    if get(values, "outputPredictor.pluginClientSvid.enabled") is not True:
        fail("Strategy C production example requires outputPredictor.pluginClientSvid.enabled=true")
    bindings = get(values, "outputPredictor.pluginClientSvid.bindings", [])
    if not isinstance(bindings, list) or not bindings:
        fail("Strategy C production example requires at least one per-tenant SVID binding")
else:
    bindings = []

for path in [
    "outputPredictor.mtlsSecretName",
    "runCostProjector.mtlsSecretName",
    "statsAggregator.sinkMtlsSecretName",
    "statsAggregator.signingKeySecretName",
    "statsAggregator.signingKeyPath",
    "statsAggregator.schemaBundleHashHex",
]:
    require_string(path)

components = {
    str((doc.get("metadata") or {}).get("labels", {}).get("app.kubernetes.io/component"))
    for doc in docs
}
expected_components = {
    "ledger",
    "canonical-ingest",
    "control-plane",
    "sidecar",
    "tokenizer",
    "output-predictor",
    "run-cost-projector",
    "egress-proxy",
    "stats-aggregator",
    "webhook-receiver",
    "outbox-forwarder",
    "ttl-sweeper",
    "migrations",
    "networkpolicy",
}
missing_components = sorted(expected_components - components)
if missing_components:
    fail(f"production render missing expected components: {', '.join(missing_components)}")

network_policies = [doc for doc in docs if doc.get("kind") == "NetworkPolicy"]
if len(network_policies) < 3:
    fail("production render must include NetworkPolicy resources")

for policy in network_policies:
    if str((policy.get("metadata") or {}).get("name", "")).endswith("allow-sidecar-internal"):
        expected_ports = {
            "ledger": 50051,
            "canonical-ingest": 50052,
            "tokenizer": 50053,
            "output-predictor": 50054,
            "run-cost-projector": 50055,
        }
        egress_rules = ((policy.get("spec") or {}).get("egress") or [])
        for component, port in expected_ports.items():
            matched = False
            for rule in egress_rules:
                selectors = [
                    (target.get("podSelector") or {}).get("matchLabels") or {}
                    for target in (rule.get("to") or [])
                ]
                ports = [entry.get("port") for entry in (rule.get("ports") or [])]
                if any(selector.get("app.kubernetes.io/component") == component for selector in selectors) and port in ports:
                    matched = True
                    break
            if not matched:
                fail(f"allow-sidecar-internal NetworkPolicy must allow {component} on TCP {port}")
        break
else:
    fail("production render missing allow-sidecar-internal NetworkPolicy")

certificates = [doc for doc in docs if doc.get("kind") == "Certificate"]
cert_uris = {
    uri
    for doc in certificates
    for uri in ((doc.get("spec") or {}).get("uris") or [])
}
for binding in bindings:
    tenant_id = str(binding.get("tenantId", ""))
    client_cert_id = str(binding.get("clientCertId", ""))
    if not tenant_id or not client_cert_id:
        fail("each SVID binding must include tenantId and clientCertId")
    expected_uri = f"spiffe://spendguard.platform/predictor-client/{tenant_id}"
    if expected_uri not in cert_uris:
        fail(f"missing Certificate URI SAN for tenant binding: {expected_uri}")


def pod_spec_for(doc):
    kind = doc.get("kind")
    spec = doc.get("spec") or {}
    if kind in {"Deployment", "DaemonSet", "StatefulSet"}:
        return ((spec.get("template") or {}).get("spec") or {})
    if kind == "Job":
        return ((spec.get("template") or {}).get("spec") or {})
    if kind == "CronJob":
        return ((((spec.get("jobTemplate") or {}).get("spec") or {}).get("template") or {}).get("spec") or {})
    return None


for doc in docs:
    pod_spec = pod_spec_for(doc)
    if pod_spec is None:
        continue
    resource_name = (doc.get("metadata") or {}).get("name", doc.get("kind", "<unknown>"))
    if doc.get("kind") == "DaemonSet" and str(resource_name).endswith("sidecar"):
        for volume in pod_spec.get("volumes", []) or []:
            if volume.get("name") != "uds":
                continue
            host_path = volume.get("hostPath") or {}
            if host_path.get("type") != "Directory":
                fail("sidecar UDS hostPath must use type=Directory in production so node prep owns write permissions for UID/GID 65532")
            break
        else:
            fail("sidecar DaemonSet must mount the UDS hostPath volume")
    pod_sc = pod_spec.get("securityContext") or {}
    for container in pod_spec.get("containers", []):
        name = container.get("name", "<unnamed>")
        c_sc = container.get("securityContext") or {}
        effective_run_as_non_root = c_sc.get("runAsNonRoot", pod_sc.get("runAsNonRoot"))
        effective_run_as_user = c_sc.get("runAsUser", pod_sc.get("runAsUser"))
        if effective_run_as_non_root is not True:
            fail(f"{resource_name}/{name} must set runAsNonRoot=true")
        if effective_run_as_user != 65532:
            fail(f"{resource_name}/{name} must run as UID 65532")
        if c_sc.get("readOnlyRootFilesystem") is not True:
            fail(f"{resource_name}/{name} must set readOnlyRootFilesystem=true")
        if c_sc.get("allowPrivilegeEscalation") is not False:
            fail(f"{resource_name}/{name} must set allowPrivilegeEscalation=false")
        drops = (((c_sc.get("capabilities") or {}).get("drop")) or [])
        if "ALL" not in drops:
            fail(f"{resource_name}/{name} must drop ALL Linux capabilities")

        image = str(container.get("image", ""))
        if image.startswith("spendguard/"):
            fail(f"{resource_name}/{name} rendered unqualified image {image!r}; global.imageRegistry was not applied")

        for env in container.get("env", []) or []:
            env_name = env.get("name", "")
            if "DATABASE_URL" not in env_name:
                continue
            if "value" in env:
                fail(f"{resource_name}/{name}:{env_name} must use valueFrom.secretKeyRef, not literal value")
            ref = (((env.get("valueFrom") or {}).get("secretKeyRef")) or {})
            if not ref.get("name") or not ref.get("key"):
                fail(f"{resource_name}/{name}:{env_name} must include secretKeyRef name and key")

rendered_text = render_path.read_text(encoding="utf-8")
if re.search(r"(?i)\bpostgres(?:ql)?://", rendered_text):
    fail("rendered manifest must not contain plaintext Postgres URLs")

print("production Helm values validated")
PY

if [[ "$skip_negative_tests" == "false" ]]; then
  tmp_plaintext="$(mktemp)"
  tmp_svid="$(mktemp)"
  tmp_zero_hash="$(mktemp)"
  tmp_render_bad="$(mktemp)"
  cleanup() {
    rm -f "$tmp_plaintext" "$tmp_svid" "$tmp_zero_hash" "$tmp_render_bad"
    if [[ -z "$rendered_manifest" ]]; then
      rm -f "$render_file"
    fi
  }
  trap cleanup EXIT

  python3 - "$values_file" "$tmp_plaintext" "$tmp_svid" "$tmp_zero_hash" <<'PY'
from pathlib import Path
import sys

import yaml

source = Path(sys.argv[1])
plaintext = Path(sys.argv[2])
missing_svid = Path(sys.argv[3])
zero_hash = Path(sys.argv[4])
values = yaml.safe_load(source.read_text(encoding="utf-8"))

bad = yaml.safe_load(source.read_text(encoding="utf-8"))
bad["postgres"]["existingSecret"] = "postgresql://spendguard:secret@db.example.invalid/spendguard"
plaintext.write_text(yaml.safe_dump(bad, sort_keys=False), encoding="utf-8")

bad = yaml.safe_load(source.read_text(encoding="utf-8"))
bad["outputPredictor"]["pluginClientSvid"]["bindings"] = []
missing_svid.write_text(yaml.safe_dump(bad, sort_keys=False), encoding="utf-8")

bad = yaml.safe_load(source.read_text(encoding="utf-8"))
bad["sidecar"]["contractBundleHashHex"] = "0" * 64
zero_hash.write_text(yaml.safe_dump(bad, sort_keys=False), encoding="utf-8")
PY

  if "$0" "$tmp_plaintext" --skip-negative-tests >/tmp/spendguard-ga03-plaintext.out 2>/tmp/spendguard-ga03-plaintext.err; then
    echo "plaintext database URL negative test unexpectedly passed" >&2
    exit 1
  fi
  if "$0" "$tmp_svid" --skip-negative-tests >/tmp/spendguard-ga03-svid.out 2>/tmp/spendguard-ga03-svid.err; then
    echo "missing SVID binding negative test unexpectedly passed" >&2
    exit 1
  fi
  if "$0" "$tmp_zero_hash" --skip-negative-tests >/tmp/spendguard-ga03-zero-hash.out 2>/tmp/spendguard-ga03-zero-hash.err; then
    echo "zero hash placeholder negative test unexpectedly passed" >&2
    exit 1
  fi

  python3 - "$render_file" "$tmp_render_bad" <<'PY'
from pathlib import Path
import sys

import yaml

docs = [doc for doc in yaml.safe_load_all(Path(sys.argv[1]).read_text(encoding="utf-8")) if doc]
changed = False

for doc in docs:
    if doc.get("kind") != "Deployment":
        continue
    metadata = doc.get("metadata") or {}
    if not str(metadata.get("name", "")).endswith("output-predictor"):
        continue
    container = ((doc.get("spec") or {}).get("template") or {}).get("spec", {}).get("containers", [])[0]
    container.get("securityContext", {}).pop("readOnlyRootFilesystem", None)
    changed = True
    break

if not changed:
    raise SystemExit("could not find output-predictor deployment to mutate")

Path(sys.argv[2]).write_text("---\n".join(yaml.safe_dump(doc, sort_keys=False) for doc in docs), encoding="utf-8")
PY

  if "$0" "$values_file" --skip-negative-tests --rendered-manifest "$tmp_render_bad" >/tmp/spendguard-ga03-security.out 2>/tmp/spendguard-ga03-security.err; then
    echo "securityContext negative test unexpectedly passed" >&2
    exit 1
  fi
  echo "negative production Helm values tests failed closed"
fi

echo "production Helm values gate passed: $values_file"
