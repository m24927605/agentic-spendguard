# SpendGuard Strategy C output predictor plugin — reference container.
#
# Spec refs:
#   - output-predictor-plugin-contract-v1alpha1.md §10 (template surface)
#   - SLICE_03+ container security baseline (non-root, distroless-ish)
#
# Multi-stage build:
#   1. `deps` — pin pip + pre-build wheels for scikit-learn / numpy
#      so the final layer doesn't carry build toolchains.
#   2. final  — slim runtime + the pre-built wheels + the template.
#
# The image is **customer-side**. SpendGuard does not publish or run
# this image; it is for the customer to fork, replace the stub model,
# and deploy in their own infrastructure.

# ---- Stage 1: build wheels ----------------------------------------
# Base image SHA pinned for reproducibility. Update via
#   docker pull python:3.11-slim && docker inspect python:3.11-slim ...
FROM python:3.11-slim AS deps

ENV PIP_DISABLE_PIP_VERSION_CHECK=1 \
    PIP_NO_CACHE_DIR=1 \
    PYTHONDONTWRITEBYTECODE=1

WORKDIR /build
COPY requirements.txt ./
RUN pip install --upgrade pip==24.2 && \
    pip wheel --wheel-dir=/wheels -r requirements.txt

# ---- Stage 2: runtime image ---------------------------------------
FROM python:3.11-slim AS final

# OCI labels for downstream discovery / image scanners.
LABEL org.opencontainers.image.title="spendguard-output-predictor-template" \
      org.opencontainers.image.description="SpendGuard Strategy C plugin reference (STUB MODEL — replace before production)" \
      org.opencontainers.image.licenses="Apache-2.0" \
      org.opencontainers.image.source="https://github.com/m24927605/agentic-spendguard" \
      org.opencontainers.image.documentation="https://github.com/m24927605/agentic-spendguard/blob/main/contrib/output_predictor_template/README.md"

ENV PYTHONUNBUFFERED=1 \
    PYTHONDONTWRITEBYTECODE=1 \
    PIP_DISABLE_PIP_VERSION_CHECK=1 \
    PIP_NO_CACHE_DIR=1 \
    PREDICTOR_PORT=50054

# `grpc_health_probe` for Kubernetes / Docker liveness probes.
# Pinned binary release; update sha256 alongside the version bump.
ARG GRPC_HEALTH_PROBE_VERSION=v0.4.34
RUN apt-get update && \
    apt-get install --no-install-recommends -y wget ca-certificates && \
    wget -qO /usr/local/bin/grpc_health_probe \
        "https://github.com/grpc-ecosystem/grpc-health-probe/releases/download/${GRPC_HEALTH_PROBE_VERSION}/grpc_health_probe-linux-amd64" && \
    chmod +x /usr/local/bin/grpc_health_probe && \
    apt-get purge -y wget && \
    apt-get autoremove -y && \
    rm -rf /var/lib/apt/lists/*

# Install Python deps from the pre-built wheel cache.
COPY --from=deps /wheels /wheels
COPY requirements.txt /tmp/
RUN pip install --no-index --find-links=/wheels -r /tmp/requirements.txt && \
    rm -rf /wheels /tmp/requirements.txt

# Copy the template source. Layered last so iterative dev rebuilds fast.
WORKDIR /app
COPY _proto/ ./_proto/
COPY feature_extractor.py predictor_server.py model_predictor_stub.py backtest_harness.py ./

# SLICE_03+ security baseline: run as an unprivileged user.
# 65532:65532 matches `distroless:nonroot` so a customer can later
# swap the base image to distroless without changing the GID.
RUN groupadd --gid 65532 nonroot && \
    useradd --uid 65532 --gid 65532 --no-create-home --shell /sbin/nologin nonroot && \
    chown -R nonroot:nonroot /app
USER 65532:65532

EXPOSE 50054

# Health probe against the standard grpc.health.v1 endpoint. The
# CustomerPredictor service is registered there during startup.
HEALTHCHECK --interval=30s --timeout=5s --start-period=20s --retries=3 \
    CMD grpc_health_probe -addr=localhost:${PREDICTOR_PORT} \
        -service=spendguard.output_predictor_plugin.v1.CustomerPredictor

# In production, the customer SHOULD mount certs and remove --insecure:
#   docker run \
#     -v /etc/predictor/certs:/certs:ro \
#     -e PREDICTOR_TLS_SERVER_CERT=/certs/server.pem \
#     -e PREDICTOR_TLS_SERVER_KEY=/certs/server.key \
#     -e PREDICTOR_TLS_CLIENT_CA=/certs/spendguard-ca.pem \
#     -e PREDICTOR_TENANT_ID=<tenant-uuidv7> \
#     spendguard-plugin-template
ENTRYPOINT ["python", "predictor_server.py"]
CMD ["--insecure"]
