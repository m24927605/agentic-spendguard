#!/usr/bin/env bash
# Regenerate Python bindings for the Strategy C plugin contract.
#
# Source proto: ../../proto/spendguard/output_predictor_plugin/v1/plugin.proto
# Output:       _proto/spendguard/output_predictor_plugin/v1/plugin_pb2{,_grpc,.pyi}.py
#
# The generated code is intentionally placed under _proto/ so that the
# template surface (predictor_server.py / feature_extractor.py / etc.)
# imports it as `from _proto.spendguard.output_predictor_plugin.v1 import plugin_pb2`
# without polluting the top-level namespace.
#
# Run this once after cloning; commit the generated files OR regenerate
# in CI. The template ships pre-generated stubs so a customer can
# `docker build` without grpcio-tools installed first.
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${HERE}/../.." && pwd)"
PROTO_ROOT="${REPO_ROOT}/proto"
OUT_DIR="${HERE}/_proto"

if [[ ! -d "${PROTO_ROOT}/spendguard/output_predictor_plugin" ]]; then
  echo "ERROR: ${PROTO_ROOT}/spendguard/output_predictor_plugin not found." >&2
  echo "Run this script from inside the SpendGuard monorepo OR vendor" >&2
  echo "the proto files into ./proto/ before running." >&2
  exit 1
fi

mkdir -p "${OUT_DIR}"

python -m grpc_tools.protoc \
  --proto_path="${PROTO_ROOT}" \
  --python_out="${OUT_DIR}" \
  --grpc_python_out="${OUT_DIR}" \
  --pyi_out="${OUT_DIR}" \
  "${PROTO_ROOT}/spendguard/common/v1/common.proto" \
  "${PROTO_ROOT}/spendguard/output_predictor_plugin/v1/plugin.proto"

# Ensure every generated directory is a proper Python package.
find "${OUT_DIR}" -type d -exec sh -c 'test -f "$1/__init__.py" || : > "$1/__init__.py"' _ {} \;

# Rewrite absolute spendguard.* imports inside the generated code to
# the namespaced location so they resolve under _proto/. Mirrors the
# SDK Makefile rewrite (sdk/python/Makefile:proto).
find "${OUT_DIR}" \( -name '*.py' -o -name '*.pyi' \) \
  -exec sed -i.bak \
    -e 's|^from spendguard\.|from _proto.spendguard.|g' \
    -e 's|^import spendguard\.|import _proto.spendguard.|g' \
    {} \;
find "${OUT_DIR}" -name '*.bak' -delete

echo "Generated Python bindings under ${OUT_DIR}/"
