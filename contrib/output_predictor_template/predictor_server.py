"""Strategy C output predictor plugin — reference gRPC server.

Per `output-predictor-plugin-contract-v1alpha1.md` §2 + §10.

This is a *customer template*. SpendGuard does not operate this binary;
customers do. The wire surface (proto messages, error semantics, mTLS
expectations) is the public ABI — keep it stable. The model logic
inside (``model_predictor_stub.py``) is yours to replace.

Highlights
----------

- Implements ``CustomerPredictor.Predict`` + ``HealthCheck`` from
  ``proto/spendguard/output_predictor_plugin/v1/plugin.proto``.
- Validates request fields up-front (tenant_id format, model length,
  ``input_tokens >= 0``) before calling the model — invalid requests
  receive ``INVALID_ARGUMENT`` (per spec §5.1 plugin-side hygiene).
- Enforces the SLICE_07 §7 tenant binding: when ``--tenant-id`` is
  configured, requests whose ``tenant_id`` differs are rejected.
- mTLS server cert + client cert validation when ``--tls-server-cert``
  and ``--tls-client-ca`` are supplied (per spec §3).
- ``grpc.health.v1.Health`` registered on the same port so
  ``grpc_health_probe`` works for Docker / K8s probes.

Customer guidance
-----------------

1. Run this without TLS for local development:
   ``python predictor_server.py --insecure --port 50054``
2. In production, ALWAYS supply ``--tls-server-cert`` + ``--tls-server-key``
   and ``--tls-client-ca`` (the SpendGuard CA bundle). The server will
   refuse to start otherwise unless ``--insecure`` is set.
3. Bind to a port and EXPOSE it from your Dockerfile. The plugin's
   default is ``50054``; SpendGuard's control plane registration
   stores ``endpoint_url`` per tenant so the port number is opaque
   to SpendGuard.
4. Replace ``model_predictor_stub.StubModel`` with your trained model
   that exposes the same surface (``predict``, ``confidence``,
   ``MODEL_VERSION``, ``sample_size``).
"""
from __future__ import annotations

import argparse
import logging
import os
import signal
import sys
import time
from concurrent import futures
from pathlib import Path
from typing import TYPE_CHECKING

import grpc
from google.protobuf.timestamp_pb2 import Timestamp
from grpc_health.v1 import health, health_pb2, health_pb2_grpc

# Generated proto stubs. ``gen_proto.sh`` writes these under ``_proto/``.
# We extend ``sys.path`` so ``python predictor_server.py`` works without
# a full package install (matches the README quickstart).
_HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(_HERE))
sys.path.insert(0, str(_HERE / "_proto"))

from _proto.spendguard.output_predictor_plugin.v1 import (  # noqa: E402
    plugin_pb2,
    plugin_pb2_grpc,
)
from feature_extractor import SCHEMA, vectorize  # noqa: E402
from model_predictor_stub import StubModel  # noqa: E402

if TYPE_CHECKING:  # pragma: no cover - typing only
    from collections.abc import Callable


LOGGER = logging.getLogger("spendguard.predictor_template")

# Bounds applied to the model output before returning to SpendGuard.
# These exist to prevent obviously-broken model artifacts from poisoning
# SpendGuard's downstream invariants (per spec §5.1 SpendGuard ALSO
# validates, but customer-side defense in depth is good practice).
MIN_OUTPUT_TOKENS = 1
MAX_OUTPUT_TOKENS = 4_000_000  # SpendGuard re-clips against model context window

# Maximum byte length we accept on request strings. Beyond this we
# return INVALID_ARGUMENT rather than wasting model compute.
MAX_STRING_FIELD_LEN = 1024


class PredictorServicer(plugin_pb2_grpc.CustomerPredictorServicer):
    """Reference implementation of the SpendGuard Strategy C plugin."""

    def __init__(
        self,
        *,
        model: StubModel,
        expected_tenant_id: str | None,
        health_servicer: health.HealthServicer,
    ) -> None:
        self._model = model
        self._expected_tenant_id = (expected_tenant_id or "").strip() or None
        self._health = health_servicer
        # Register the CustomerPredictor service under the standard
        # gRPC health probe so an operator can verify both the
        # streaming health endpoint and the contract's HealthCheck RPC.
        self._health.set("", health_pb2.HealthCheckResponse.SERVING)
        self._health.set(
            "spendguard.output_predictor_plugin.v1.CustomerPredictor",
            health_pb2.HealthCheckResponse.SERVING,
        )

    # -- helpers -----------------------------------------------------

    def _validate_request(
        self,
        request: plugin_pb2.PredictRequest,
        context: grpc.ServicerContext,
    ) -> bool:
        """Return False (after aborting context) when the request is malformed.

        Validation rules:
        - All string fields must be <= ``MAX_STRING_FIELD_LEN`` bytes.
        - ``tenant_id`` must be non-empty (SpendGuard's contract requires
          it; an empty string indicates a SpendGuard misconfiguration).
        - ``input_tokens`` must be non-negative (proto3 ``int64``; a
          negative value is a SpendGuard bug or wire tampering).
        - When ``--tenant-id`` is configured on the server, the request
          tenant_id MUST match — per spec §7.2.
        """
        if len(request.tenant_id.encode("utf-8")) > MAX_STRING_FIELD_LEN:
            context.abort(grpc.StatusCode.INVALID_ARGUMENT, "tenant_id too long")
            return False
        if len(request.model.encode("utf-8")) > MAX_STRING_FIELD_LEN:
            context.abort(grpc.StatusCode.INVALID_ARGUMENT, "model too long")
            return False
        if len(request.agent_id.encode("utf-8")) > MAX_STRING_FIELD_LEN:
            context.abort(grpc.StatusCode.INVALID_ARGUMENT, "agent_id too long")
            return False
        if not request.tenant_id:
            context.abort(grpc.StatusCode.INVALID_ARGUMENT, "tenant_id is required")
            return False
        if request.input_tokens < 0:
            context.abort(
                grpc.StatusCode.INVALID_ARGUMENT,
                "input_tokens must be non-negative",
            )
            return False
        if self._expected_tenant_id and request.tenant_id != self._expected_tenant_id:
            # Spec §7.2 — plugin SHOULD verify tenant_id matches its
            # configured expected tenant; mismatch is a SpendGuard
            # misconfiguration. Returning INVALID_ARGUMENT here makes
            # SpendGuard fall back to Strategy B with a clear metric.
            LOGGER.warning(
                "tenant_id mismatch: got %s, expected %s — refusing",
                request.tenant_id,
                self._expected_tenant_id,
            )
            context.abort(
                grpc.StatusCode.INVALID_ARGUMENT,
                "tenant_id does not match the plugin's configured tenant",
            )
            return False
        return True

    # -- RPCs --------------------------------------------------------

    def Predict(  # noqa: N802 (proto-mandated CamelCase)
        self,
        request: plugin_pb2.PredictRequest,
        context: grpc.ServicerContext,
    ) -> plugin_pb2.PredictResponse:
        start_ns = time.perf_counter_ns()
        if not self._validate_request(request, context):
            return plugin_pb2.PredictResponse()  # unreachable; context.abort raises

        # Probe calls (per spec §6.4) carry a `probe-` prefix on
        # spendguard_call_id. We could fast-path them with a known-good
        # response; for the template we run the model normally so the
        # half-open transition observes real latency.
        is_probe = request.spendguard_call_id.startswith("probe-")
        try:
            feats = vectorize(request)
            prediction = self._model.predict_one(feats)
        except Exception as exc:  # noqa: BLE001 — last line of defense
            LOGGER.exception("model.predict_one raised: %s", exc)
            # Returning INTERNAL maps to SpendGuard's
            # `customer_predictor_grpc_error` fallback bucket.
            context.abort(grpc.StatusCode.INTERNAL, "model failure")
            return plugin_pb2.PredictResponse()  # unreachable

        # Defense-in-depth bounds: the stub clips to >= 1, but a
        # future replacement model might produce NaN / overflow.
        predicted = max(MIN_OUTPUT_TOKENS, min(MAX_OUTPUT_TOKENS, prediction.predicted_tokens))
        confidence = max(0.0, min(1.0, prediction.confidence))

        elapsed_us = (time.perf_counter_ns() - start_ns) / 1_000.0
        # The 50ms cap is enforced *by SpendGuard* (per spec §4.1). The
        # template still logs latency so an operator notices when their
        # replacement model creeps past the budget on their hot path.
        if elapsed_us >= 40_000.0:  # 40ms — 80% of the budget
            LOGGER.warning(
                "Predict latency %.2fms approaches the 50ms hard cap (probe=%s)",
                elapsed_us / 1_000.0,
                is_probe,
            )

        return plugin_pb2.PredictResponse(
            predicted_output_tokens=int(predicted),
            confidence=float(confidence),
            sample_size=int(self._model.sample_size),
            plugin_version=self._model.MODEL_VERSION,
            feature_hash=SCHEMA.feature_hash,
        )

    def HealthCheck(  # noqa: N802 (proto-mandated CamelCase)
        self,
        request: plugin_pb2.HealthCheckRequest,  # noqa: ARG002 — empty per spec §2.1
        context: grpc.ServicerContext,  # noqa: ARG002 — no abort path here
    ) -> plugin_pb2.HealthCheckResponse:
        # The plugin contract's HealthCheck (per spec §2.1) is a
        # separate RPC from the standard ``grpc.health.v1`` probe.
        # SpendGuard's circuit breaker (§6.3) polls THIS one.
        ts = Timestamp()
        ts.GetCurrentTime()
        return plugin_pb2.HealthCheckResponse(
            status=plugin_pb2.HealthCheckResponse.SERVING,
            plugin_version=self._model.MODEL_VERSION,
            checked_at=ts,
        )


# ---------------------------------------------------------------------------
# Server bootstrap
# ---------------------------------------------------------------------------


def _load_credentials(
    *,
    server_cert_path: str,
    server_key_path: str,
    client_ca_path: str | None,
) -> grpc.ServerCredentials:
    """Load mTLS server credentials per spec §3.1.

    - ``server_cert_path`` / ``server_key_path``: the plugin's own TLS
      identity. The customer is responsible for renewing these (e.g.
      via cert-manager); see ``mtls_setup.md``.
    - ``client_ca_path``: the CA bundle that signs the SpendGuard
      ``predictor-client/<tenant_id>`` SVIDs. Per spec §3.1 the
      plugin MUST require + verify the client cert.
    """
    with open(server_cert_path, "rb") as f:
        server_cert = f.read()
    with open(server_key_path, "rb") as f:
        server_key = f.read()
    if client_ca_path:
        with open(client_ca_path, "rb") as f:
            client_ca = f.read()
        return grpc.ssl_server_credentials(
            [(server_key, server_cert)],
            root_certificates=client_ca,
            require_client_auth=True,
        )
    LOGGER.warning(
        "Starting WITHOUT client-cert verification. This is acceptable"
        " for local development but violates spec §3.1 — production must"
        " supply --tls-client-ca."
    )
    return grpc.ssl_server_credentials([(server_key, server_cert)])


def build_server(
    *,
    expected_tenant_id: str | None = None,
    max_workers: int = 8,
    model: StubModel | None = None,
) -> tuple[grpc.Server, PredictorServicer]:
    """Create a configured ``grpc.Server`` and return it alongside the servicer.

    Exposed as a factory so the conformance test and Docker entrypoint
    share construction logic.
    """
    server = grpc.server(
        futures.ThreadPoolExecutor(max_workers=max_workers),
        options=[
            # Match SpendGuard's connection-keep-alive expectations
            # (spec §4.3 — pooled connections).
            ("grpc.keepalive_time_ms", 30_000),
            ("grpc.keepalive_timeout_ms", 10_000),
            ("grpc.keepalive_permit_without_calls", 1),
        ],
    )
    health_servicer = health.HealthServicer()
    health_pb2_grpc.add_HealthServicer_to_server(health_servicer, server)

    model = model or StubModel()
    servicer = PredictorServicer(
        model=model,
        expected_tenant_id=expected_tenant_id,
        health_servicer=health_servicer,
    )
    plugin_pb2_grpc.add_CustomerPredictorServicer_to_server(servicer, server)
    return server, servicer


def _parse_args(argv: list[str] | None = None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0] if __doc__ else "")
    parser.add_argument(
        "--port",
        type=int,
        default=int(os.environ.get("PREDICTOR_PORT", "50054")),
        help="gRPC port (default: 50054 or PREDICTOR_PORT env var)",
    )
    parser.add_argument(
        "--tenant-id",
        type=str,
        default=os.environ.get("PREDICTOR_TENANT_ID"),
        help=(
            "Configured tenant_id. When set, requests with a different"
            " tenant_id are rejected with INVALID_ARGUMENT per spec §7.2."
        ),
    )
    parser.add_argument(
        "--insecure",
        action="store_true",
        help="Skip mTLS. Local development only — violates spec §3.",
    )
    parser.add_argument(
        "--tls-server-cert",
        type=str,
        default=os.environ.get("PREDICTOR_TLS_SERVER_CERT"),
        help="Path to PEM-encoded server certificate (mTLS).",
    )
    parser.add_argument(
        "--tls-server-key",
        type=str,
        default=os.environ.get("PREDICTOR_TLS_SERVER_KEY"),
        help="Path to PEM-encoded server private key (mTLS).",
    )
    parser.add_argument(
        "--tls-client-ca",
        type=str,
        default=os.environ.get("PREDICTOR_TLS_CLIENT_CA"),
        help=(
            "Path to PEM bundle of CAs that sign SpendGuard's"
            " predictor-client SVIDs. Required in production."
        ),
    )
    parser.add_argument(
        "--max-workers",
        type=int,
        default=int(os.environ.get("PREDICTOR_MAX_WORKERS", "8")),
        help="ThreadPoolExecutor max workers (default: 8 or PREDICTOR_MAX_WORKERS).",
    )
    parser.add_argument(
        "--log-level",
        default=os.environ.get("PREDICTOR_LOG_LEVEL", "INFO"),
        choices=["DEBUG", "INFO", "WARNING", "ERROR"],
    )
    return parser.parse_args(argv)


def main(argv: list[str] | None = None) -> int:
    args = _parse_args(argv)
    logging.basicConfig(
        level=args.log_level,
        format="%(asctime)s %(levelname)s %(name)s %(message)s",
    )

    if not args.insecure:
        missing: list[str] = []
        if not args.tls_server_cert:
            missing.append("--tls-server-cert")
        if not args.tls_server_key:
            missing.append("--tls-server-key")
        if missing:
            LOGGER.error(
                "mTLS misconfigured: %s required (or pass --insecure for local dev)",
                ", ".join(missing),
            )
            return 2

    server, _ = build_server(
        expected_tenant_id=args.tenant_id,
        max_workers=args.max_workers,
    )

    bind_addr = f"0.0.0.0:{args.port}"  # noqa: S104 — server binds 0.0.0.0 deliberately
    if args.insecure:
        server.add_insecure_port(bind_addr)
        LOGGER.info("Listening INSECURE on %s (development mode)", bind_addr)
    else:
        creds = _load_credentials(
            server_cert_path=args.tls_server_cert,
            server_key_path=args.tls_server_key,
            client_ca_path=args.tls_client_ca,
        )
        server.add_secure_port(bind_addr, creds)
        LOGGER.info("Listening MTLS on %s", bind_addr)

    server.start()

    stop_event = futures.Future()  # type: ignore[var-annotated]

    def _shutdown(signum: int, _frame: object) -> None:
        LOGGER.info("Received signal %s — draining…", signum)
        server.stop(grace=5).wait()
        if not stop_event.done():
            stop_event.set_result(None)

    signal.signal(signal.SIGTERM, _shutdown)
    signal.signal(signal.SIGINT, _shutdown)

    try:
        server.wait_for_termination()
    finally:
        if not stop_event.done():
            stop_event.set_result(None)
    return 0


if __name__ == "__main__":  # pragma: no cover - CLI entrypoint
    sys.exit(main())
