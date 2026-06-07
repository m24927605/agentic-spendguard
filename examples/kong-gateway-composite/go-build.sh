#!/usr/bin/env bash
# D09 SLICE 6 — Build the SpendGuard Kong plugin and bake it into a
# custom Kong DataPlane image. Reference recipe for operators.
#
# Spec ref: docs/specs/coverage/D09_kong_ai_gateway/implementation.md §10.
#
# Usage:
#   ./go-build.sh [TAG]
#
# Default TAG = `dev`. The output image is named
# `spendguard-kong-datadog-dataplane:${TAG}` (registry-less; operators
# tag + push to their own registry per `docker tag`).
#
# Prereqs:
#   - go 1.22+ (for `go build` of the plugin-server binary)
#   - docker (for the multi-stage image build)
#   - sufficient disk for the Kong base image (~1.5 GB)

set -euo pipefail

TAG="${1:-dev}"
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
PLUGIN_DIR="$REPO_ROOT/plugins/kong/spendguard-go"
OUT_BIN="$REPO_ROOT/target/kong/spendguard"

echo "[go-build] D09 SLICE 6 — building Kong plugin .so + custom Kong DataPlane image"
echo "[go-build] repo root:    $REPO_ROOT"
echo "[go-build] plugin dir:   $PLUGIN_DIR"
echo "[go-build] output binary: $OUT_BIN"
echo "[go-build] image tag:    spendguard-kong-dataplane:$TAG"

# 1. Build the plugin-server binary. Static link so the resulting
#    binary runs inside the Kong slim image without glibc surprises.
echo "[go-build] step 1: go build plugin-server binary"
( cd "$PLUGIN_DIR" && CGO_ENABLED=0 go build -o "$OUT_BIN" -ldflags="-s -w" ./... )

# 2. Build the Kong DataPlane image. The Dockerfile below is the
#    minimum recipe to drop the binary into the image. Operators
#    who already maintain a custom Kong image should copy the
#    `COPY ... /usr/local/bin/spendguard` line into their own
#    Dockerfile instead of using this one verbatim.
echo "[go-build] step 2: docker build custom Kong DataPlane image"
cat > /tmp/Dockerfile.kong-spendguard <<'EOF'
# Reference Kong DataPlane image with the SpendGuard plugin baked in.
# Kong 3.7 is the floor for stable go-plugin-server v0.6.0.
FROM kong/kong-gateway:3.7

# Run as the kong user; the entry chmod is a one-shot fix-up so the
# go-plugin-server subprocess can exec the binary.
USER root
COPY target/kong/spendguard /usr/local/bin/spendguard
RUN chmod +x /usr/local/bin/spendguard
USER kong

# Kong's plugin-server protocol expects the binary path under
# KONG_PLUGINSERVER_<NAME>_START_CMD. The reference KongPlugin CRD
# in this directory wires the matching `plugins = bundled,spendguard`
# config; we surface it here as ENV so a quick `docker run` works
# without an operator config-map.
ENV KONG_PLUGINS="bundled,spendguard" \
    KONG_PLUGINSERVER_NAMES="spendguard" \
    KONG_PLUGINSERVER_SPENDGUARD_START_CMD="/usr/local/bin/spendguard" \
    KONG_PLUGINSERVER_SPENDGUARD_QUERY_CMD="/usr/local/bin/spendguard -dump"
EOF

docker build \
    -t "spendguard-kong-dataplane:$TAG" \
    -f /tmp/Dockerfile.kong-spendguard \
    "$REPO_ROOT"

echo "[go-build] PASS — image spendguard-kong-dataplane:$TAG built"
echo "[go-build] next steps:"
echo "[go-build]   docker tag spendguard-kong-dataplane:$TAG <your-registry>/...:$TAG"
echo "[go-build]   docker push <your-registry>/...:$TAG"
echo "[go-build]   helm upgrade ... --set image.repository=<your-registry>/..."
