#!/usr/bin/env bash
# SLICE_14: CI conformance harness for the customer-side output
# predictor plugin template.
#
# This script is the source-of-truth gate that proves the template can:
#
#   1. Build into a Docker image without grpcio-tools at runtime.
#   2. Boot the gRPC server in a container with the standard health
#      probe wired up.
#   3. Pass the in-process pytest conformance suite (50 happy-path
#      requests, 8 failure modes, tenant binding, concurrency).
#   4. Produce a backtest report against the bundled sample dataset
#      (informational; the stub model deliberately recommends retrain
#      and that does NOT fail the build).
#
# Run locally to mimic CI exactly:
#
#     bash ci/template_conformance.sh
#
# Required tools: docker (with buildx), python3.11, pip. The script
# creates a venv under contrib/output_predictor_template/.venv-ci/
# so it never pollutes the developer's working venv.
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${HERE}/.." && pwd)"
TEMPLATE_DIR="${REPO_ROOT}/contrib/output_predictor_template"
VENV_DIR="${TEMPLATE_DIR}/.venv-ci"
IMAGE_TAG="${IMAGE_TAG:-spendguard-plugin-template:ci}"
CONTAINER_NAME="spendguard-plugin-template-ci"
HOST_PORT="${HOST_PORT:-50054}"

log() { printf '[template-conformance] %s\n' "$*" >&2; }
require() {
  command -v "$1" >/dev/null 2>&1 || {
    log "ERROR: required tool $1 is not on PATH"
    exit 1
  }
}

cleanup() {
  if docker ps -a --format '{{.Names}}' | grep -qx "${CONTAINER_NAME}"; then
    log "removing container ${CONTAINER_NAME}"
    docker rm -f "${CONTAINER_NAME}" >/dev/null 2>&1 || true
  fi
}
trap cleanup EXIT INT TERM

# ------------------------------------------------------------------
# 0. Sanity: required tools are present.
# ------------------------------------------------------------------
require python3.11
require docker
log "tool versions:"
python3.11 --version >&2
python3.11 -m pip --version >&2
docker --version >&2

# ------------------------------------------------------------------
# 1. Local pytest conformance suite.
# ------------------------------------------------------------------
log "creating CI virtualenv at ${VENV_DIR}"
rm -rf "${VENV_DIR}"
python3.11 -m venv "${VENV_DIR}"
# shellcheck disable=SC1091
source "${VENV_DIR}/bin/activate"
pip install --quiet --upgrade pip setuptools wheel
pip install --quiet -r "${TEMPLATE_DIR}/requirements.txt"
# grpcio-tools <1.70 keeps gencode at 5.29.x — matches the runtime
# pin in requirements.txt. See the comment in requirements.txt for
# the upgrade path.
pip install --quiet "grpcio-tools>=1.62,<1.70" "pytest>=8" "pytest-asyncio>=0.23" "cryptography>=42"

log "regenerating proto bindings (sanity check gen_proto.sh)"
( cd "${TEMPLATE_DIR}" && bash gen_proto.sh ) >&2

log "running conformance pytest suite"
( cd "${TEMPLATE_DIR}" && \
  PYTHONPATH=".:_proto" python -m pytest conformance_test.py -v --tb=short )

# ------------------------------------------------------------------
# 2. Backtest harness against the sample dataset (informational).
# ------------------------------------------------------------------
log "running backtest harness against sample dataset"
set +e
( cd "${TEMPLATE_DIR}" && \
  PYTHONPATH=".:_proto" python backtest_harness.py \
    --csv data/sample_audit_data.csv \
    --json-out /tmp/backtest_report.json )
BT_RC=$?
set -e
log "backtest exit code: ${BT_RC} (0=well-calibrated, 3=retrain recommended)"
if [[ ${BT_RC} -ne 0 && ${BT_RC} -ne 3 ]]; then
  log "ERROR: backtest harness failed with unexpected code ${BT_RC}"
  exit ${BT_RC}
fi

# ------------------------------------------------------------------
# 3. Docker build + container smoke test.
# ------------------------------------------------------------------
log "building Docker image ${IMAGE_TAG}"
docker build -t "${IMAGE_TAG}" "${TEMPLATE_DIR}"

log "image size:"
docker images --format 'table {{.Repository}}:{{.Tag}}\t{{.Size}}' "${IMAGE_TAG}" >&2

log "booting container ${CONTAINER_NAME} on host port ${HOST_PORT}"
docker run --rm -d \
  --name "${CONTAINER_NAME}" \
  -e PREDICTOR_TENANT_ID="tenant-a" \
  -p "${HOST_PORT}:50054" \
  "${IMAGE_TAG}" \
  --insecure --port 50054 --tenant-id tenant-a >/dev/null

log "waiting for container to report HEALTHY (max 60s)"
for i in $(seq 1 30); do
  if docker exec "${CONTAINER_NAME}" grpc_health_probe \
       -addr=localhost:50054 \
       -service=spendguard.output_predictor_plugin.v1.CustomerPredictor \
       >/dev/null 2>&1; then
    log "container ready after ~$((i * 2))s"
    break
  fi
  if [[ $i -eq 30 ]]; then
    log "ERROR: container failed to report HEALTHY"
    docker logs "${CONTAINER_NAME}" >&2 || true
    exit 1
  fi
  sleep 2
done

log "running mock SpendGuard round-trip against the container"
( cd "${TEMPLATE_DIR}" && \
  PYTHONPATH=".:_proto" python - <<'PY'
"""Mock SpendGuard client: send 50 Predicts and a HealthCheck."""
import os
import sys
import time

import grpc

sys.path.insert(0, ".")
sys.path.insert(0, "_proto")
from _proto.spendguard.output_predictor_plugin.v1 import plugin_pb2, plugin_pb2_grpc

PORT = int(os.environ.get("HOST_PORT", "50054"))
channel = grpc.insecure_channel(f"127.0.0.1:{PORT}")
grpc.channel_ready_future(channel).result(timeout=10)
stub = plugin_pb2_grpc.CustomerPredictorStub(channel)

PROMPT_CLASSES = ("chat_short", "chat_long", "code_gen", "summarization",
                  "rag", "tool_calling", "vision")
MODELS = ("gpt-4o", "claude-3-5-sonnet-20240620", "gemini-1.5-pro")

ok = 0
for i in range(50):
    req = plugin_pb2.PredictRequest(
        spendguard_call_id=f"ci-roundtrip-{i}",
        tenant_id="tenant-a",
        model=MODELS[i % len(MODELS)],
        agent_id="ci-agent",
        prompt_class=PROMPT_CLASSES[i % len(PROMPT_CLASSES)],
        input_tokens=512,
    )
    req.features.has_system_message = True
    t0 = time.perf_counter()
    resp = stub.Predict(req, timeout=2.0)
    elapsed_ms = (time.perf_counter() - t0) * 1000
    assert resp.predicted_output_tokens > 0
    assert 0.0 <= resp.confidence <= 1.0
    ok += 1
    if i < 3:
        print(f"  call {i}: {resp.predicted_output_tokens} tokens, "
              f"conf={resp.confidence:.2f}, {elapsed_ms:.1f}ms", flush=True)

hc = stub.HealthCheck(plugin_pb2.HealthCheckRequest(), timeout=1.0)
assert hc.status == plugin_pb2.HealthCheckResponse.SERVING
print(f"50/50 round-trips OK; HealthCheck={hc.status} version={hc.plugin_version}")
PY
)

log "ALL GREEN — template conformance gate passed"
