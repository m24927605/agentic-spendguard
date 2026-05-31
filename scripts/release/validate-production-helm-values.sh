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


def validate_image_tag(name: str, tag: str) -> None:
    if not re.fullmatch(r"(@sha256:[0-9a-f]{64}|v?[0-9]+\.[0-9]+\.[0-9]+(-[a-z0-9.]+)?)", str(tag)):
        fail(f"{name}.image.tag must be semver or @sha256 digest under production values")


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

for image_name in [
    "ledger",
    "canonicalIngest",
    "controlPlane",
    "sidecar",
    "tokenizer",
    "outputPredictor",
    "runCostProjector",
    "egressProxy",
    "statsAggregator",
    "webhookReceiver",
    "outboxForwarder",
    "ttlSweeper",
]:
    validate_image_tag(image_name, get(values, f"{image_name}.image.tag", ""))

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

migration_image = require_string("migrations.ledgerImage")
if not re.fullmatch(r"(localhost(:[0-9]+)?|[A-Za-z0-9.-]+\.[A-Za-z0-9.-]+|[A-Za-z0-9.-]+:[0-9]+)/.+", migration_image):
    fail("migrations.ledgerImage must include an explicit registry")
if re.search(r":(latest|dev|edge|main|master|snapshot)$", migration_image):
    fail("migrations.ledgerImage must not use mutable tags like latest/dev/edge/main/master/snapshot")
if "canonicalImage" in (get(values, "migrations", {}) or {}):
    fail("migrations.canonicalImage is not used by the chart; remove the dead value")

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
    has_sidecar_uds = False
    for volume in pod_spec.get("volumes", []) or []:
        host_path = volume.get("hostPath") or {}
        if host_path.get("path") != "/var/run/spendguard":
            continue
        has_sidecar_uds = True
        if host_path.get("type") != "Directory":
            fail(f"{resource_name} UDS hostPath must use type=Directory in production so node prep owns write permissions for UID/GID 65532")
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
        first_segment = image.split("/", 1)[0]
        if "/" not in image or ("." not in first_segment and ":" not in first_segment and first_segment != "localhost"):
            fail(f"{resource_name}/{name} rendered image {image!r} without an explicit registry")
        if ":@" in image:
            fail(f"{resource_name}/{name} rendered invalid digest image reference {image!r}")

        for env in container.get("env", []) or []:
            env_name = env.get("name", "")
            if env_name == "SPENDGUARD_PROXY_OUTPUT_PREDICTOR_ENDPOINT" and str(env.get("value", "")).startswith("https://"):
                fail("egress-proxy must not render https output_predictor endpoint until mTLS client support lands")
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
  tmp_https_predictor="$(mktemp)"
  tmp_bad_hostpath="$(mktemp)"
  tmp_bad_migration_image="$(mktemp)"
  tmp_latest_image="$(mktemp)"
  tmp_empty_issuer="$(mktemp)"
  tmp_dead_canonical_image="$(mktemp)"
  tmp_render_bad="$(mktemp)"
  cleanup() {
    rm -f "$tmp_plaintext" "$tmp_svid" "$tmp_zero_hash" "$tmp_https_predictor" "$tmp_bad_hostpath" "$tmp_bad_migration_image" "$tmp_latest_image" "$tmp_empty_issuer" "$tmp_dead_canonical_image" "$tmp_render_bad"
    if [[ -z "$rendered_manifest" ]]; then
      rm -f "$render_file"
    fi
  }
  trap cleanup EXIT

  python3 - "$values_file" "$tmp_plaintext" "$tmp_svid" "$tmp_zero_hash" "$tmp_https_predictor" "$tmp_bad_hostpath" "$tmp_bad_migration_image" "$tmp_latest_image" "$tmp_empty_issuer" "$tmp_dead_canonical_image" <<'PY'
from pathlib import Path
import sys

import yaml

source = Path(sys.argv[1])
plaintext = Path(sys.argv[2])
missing_svid = Path(sys.argv[3])
zero_hash = Path(sys.argv[4])
https_predictor = Path(sys.argv[5])
bad_hostpath = Path(sys.argv[6])
bad_migration_image = Path(sys.argv[7])
latest_image = Path(sys.argv[8])
empty_issuer = Path(sys.argv[9])
dead_canonical_image = Path(sys.argv[10])
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

bad = yaml.safe_load(source.read_text(encoding="utf-8"))
bad["egressProxy"]["outputPredictorEndpoint"] = "https://spendguard-spendguard-output-predictor:50054"
https_predictor.write_text(yaml.safe_dump(bad, sort_keys=False), encoding="utf-8")

bad = yaml.safe_load(source.read_text(encoding="utf-8"))
bad["egressProxy"]["sidecarUdsHostPathType"] = "DirectoryOrCreate"
bad_hostpath.write_text(yaml.safe_dump(bad, sort_keys=False), encoding="utf-8")

bad = yaml.safe_load(source.read_text(encoding="utf-8"))
bad["migrations"]["ledgerImage"] = "postgres:16-alpine"
bad_migration_image.write_text(yaml.safe_dump(bad, sort_keys=False), encoding="utf-8")

bad = yaml.safe_load(source.read_text(encoding="utf-8"))
bad["ledger"]["image"]["tag"] = "latest"
latest_image.write_text(yaml.safe_dump(bad, sort_keys=False), encoding="utf-8")

bad = yaml.safe_load(source.read_text(encoding="utf-8"))
bad["outputPredictor"]["pluginClientSvid"]["issuer"]["name"] = ""
empty_issuer.write_text(yaml.safe_dump(bad, sort_keys=False), encoding="utf-8")

bad = yaml.safe_load(source.read_text(encoding="utf-8"))
bad["migrations"]["canonicalImage"] = "docker.io/library/postgres:16-alpine"
dead_canonical_image.write_text(yaml.safe_dump(bad, sort_keys=False), encoding="utf-8")
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
  if "$0" "$tmp_https_predictor" --skip-negative-tests >/tmp/spendguard-ga03-https-predictor.out 2>/tmp/spendguard-ga03-https-predictor.err; then
    echo "https egress-proxy predictor endpoint negative test unexpectedly passed" >&2
    exit 1
  fi
  if "$0" "$tmp_bad_hostpath" --skip-negative-tests >/tmp/spendguard-ga03-hostpath.out 2>/tmp/spendguard-ga03-hostpath.err; then
    echo "egress-proxy DirectoryOrCreate hostPath negative test unexpectedly passed" >&2
    exit 1
  fi
  if "$0" "$tmp_bad_migration_image" --skip-negative-tests >/tmp/spendguard-ga03-migration-image.out 2>/tmp/spendguard-ga03-migration-image.err; then
    echo "unqualified migration image negative test unexpectedly passed" >&2
    exit 1
  fi
  if "$0" "$tmp_latest_image" --skip-negative-tests >/tmp/spendguard-ga03-latest-image.out 2>/tmp/spendguard-ga03-latest-image.err; then
    echo "mutable first-party image tag negative test unexpectedly passed" >&2
    exit 1
  fi
  if "$0" "$tmp_empty_issuer" --skip-negative-tests >/tmp/spendguard-ga03-empty-issuer.out 2>/tmp/spendguard-ga03-empty-issuer.err; then
    echo "empty SVID issuer negative test unexpectedly passed" >&2
    exit 1
  fi
  if "$0" "$tmp_dead_canonical_image" --skip-negative-tests >/tmp/spendguard-ga03-dead-canonical-image.out 2>/tmp/spendguard-ga03-dead-canonical-image.err; then
    echo "dead canonicalImage value negative test unexpectedly passed" >&2
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
