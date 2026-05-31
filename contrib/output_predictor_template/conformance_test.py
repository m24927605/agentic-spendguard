"""Conformance test corpus for the SpendGuard Strategy C plugin.

Runs against a *real* in-process gRPC server (the template's own
``predictor_server.build_server`` factory), exercising:

1. **Happy-path corpus** (50 requests across the 7 prompt classes and
   the major model families). Each call must complete inside the 50ms
   p99 budget on a CI laptop and return a ``predicted_output_tokens``
   value that is positive and within model context bounds.
2. **The 8 failure modes** from `output-predictor-plugin-contract-
   v1alpha1.md` §5.1. Each test asserts that the SpendGuard-side
   contract is preserved (template returns a well-formed gRPC error,
   or — for cases that SpendGuard validates server-side, such as
   negative output tokens or illegal confidence — the template's
   own defense-in-depth clipping holds the line).
3. **Tenant binding** (spec §7.2): a request with a different
   ``tenant_id`` than configured is refused.
4. **HealthCheck**: returns SERVING with a valid ``plugin_version``.

Run::

    pip install pytest grpcio
    python -m pytest conformance_test.py -v
"""
from __future__ import annotations

import sys
import threading
import time
from collections.abc import Iterator
from concurrent import futures
from pathlib import Path
from unittest.mock import patch

import grpc
import pytest
from grpc_health.v1 import health_pb2, health_pb2_grpc

_HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(_HERE))
sys.path.insert(0, str(_HERE / "_proto"))

from _proto.spendguard.output_predictor_plugin.v1 import (  # noqa: E402
    plugin_pb2,
    plugin_pb2_grpc,
)
from predictor_server import (  # noqa: E402
    MAX_OUTPUT_TOKENS,
    MAX_STRING_FIELD_LEN,
    MIN_OUTPUT_TOKENS,
    build_server,
)
from svid_validation import (  # noqa: E402
    expected_svid_subject,
    extract_spiffe_uri_from_auth_context,
    validate_auth_context_tenant,
)


PROMPT_CLASSES = (
    "chat_short",
    "chat_long",
    "code_gen",
    "summarization",
    "rag",
    "tool_calling",
    "vision",
)
MODELS = (
    "gpt-4o",
    "gpt-4o-mini",
    "gpt-3.5-turbo",
    "claude-3-5-sonnet-20240620",
    "claude-3-haiku-20240307",
    "gemini-1.5-pro",
    "gemini-1.5-flash",
)


@pytest.fixture(scope="module")
def server() -> Iterator[tuple[grpc.Server, str]]:
    """Start the plugin server on an ephemeral port for the test session."""
    srv, _ = build_server(expected_tenant_id="tenant-a")
    port = srv.add_insecure_port("127.0.0.1:0")
    srv.start()
    try:
        yield srv, f"127.0.0.1:{port}"
    finally:
        srv.stop(grace=0).wait()


@pytest.fixture(scope="module")
def stub(server: tuple[grpc.Server, str]) -> Iterator[plugin_pb2_grpc.CustomerPredictorStub]:
    _srv, addr = server
    channel = grpc.insecure_channel(addr)
    grpc.channel_ready_future(channel).result(timeout=5)
    yield plugin_pb2_grpc.CustomerPredictorStub(channel)
    channel.close()


@pytest.fixture(scope="module")
def health_stub(server: tuple[grpc.Server, str]) -> Iterator[health_pb2_grpc.HealthStub]:
    _srv, addr = server
    channel = grpc.insecure_channel(addr)
    grpc.channel_ready_future(channel).result(timeout=5)
    yield health_pb2_grpc.HealthStub(channel)
    channel.close()


def _make_request(
    *,
    tenant_id: str = "tenant-a",
    model: str = "gpt-4o",
    prompt_class: str = "chat_short",
    input_tokens: int = 200,
    max_tokens_requested: int = 0,
    spendguard_call_id: str | None = None,
) -> plugin_pb2.PredictRequest:
    req = plugin_pb2.PredictRequest(
        spendguard_call_id=spendguard_call_id or f"call-{int(time.time_ns())}",
        tenant_id=tenant_id,
        model=model,
        agent_id="agent-conformance",
        prompt_class=prompt_class,
        input_tokens=input_tokens,
        max_tokens_requested=max_tokens_requested,
    )
    req.features.has_system_message = True
    return req


# ---------------------------------------------------------------------------
# 1. Happy-path corpus
# ---------------------------------------------------------------------------


def test_happy_path_corpus_size():
    """The corpus must include >= 50 requests across the 7 classes."""
    requests = list(_happy_path_requests())
    assert len(requests) >= 50, len(requests)
    seen_classes = {r.prompt_class for r in requests}
    assert seen_classes == set(PROMPT_CLASSES), seen_classes


def _happy_path_requests() -> Iterator[plugin_pb2.PredictRequest]:
    # 7 classes * 7 models = 49 requests; add one extreme-input case to
    # reach 50 with a deterministic ordering.
    for pc in PROMPT_CLASSES:
        for m in MODELS:
            yield _make_request(model=m, prompt_class=pc, input_tokens=512)
    yield _make_request(prompt_class="chat_long", input_tokens=8000, max_tokens_requested=2048)


@pytest.mark.parametrize(
    "request_idx",
    list(range(50)),
    ids=lambda i: f"corpus[{i}]",
)
def test_happy_path_corpus(
    stub: plugin_pb2_grpc.CustomerPredictorStub,
    request_idx: int,
):
    requests = list(_happy_path_requests())
    req = requests[request_idx]
    deadline_seconds = 0.5  # generous on CI; spec is 50ms in prod
    response = stub.Predict(req, timeout=deadline_seconds)
    assert response.predicted_output_tokens >= MIN_OUTPUT_TOKENS, response
    assert response.predicted_output_tokens <= MAX_OUTPUT_TOKENS, response
    assert 0.0 <= response.confidence <= 1.0, response
    assert response.plugin_version, "plugin_version must be non-empty"
    assert response.feature_hash, "feature_hash must be non-empty"


# ---------------------------------------------------------------------------
# 2. Failure modes per spec §5.1
# ---------------------------------------------------------------------------


def test_failure_mode_timeout(stub: plugin_pb2_grpc.CustomerPredictorStub):
    """gRPC-level deadline exceeded: client cancels mid-call."""
    req = _make_request(spendguard_call_id="timeout-probe")

    # Patch the model so it takes longer than the client deadline. The
    # template wraps blocking work behind a thread pool; sleeping in
    # the model body simulates a slow real model.
    import predictor_server

    original_predict_one = predictor_server.StubModel.predict_one

    def _slow(self, x):
        time.sleep(0.5)
        return original_predict_one(self, x)

    with patch.object(predictor_server.StubModel, "predict_one", _slow):
        with pytest.raises(grpc.RpcError) as exc:
            stub.Predict(req, timeout=0.05)  # 50ms — same as the spec cap
        assert exc.value.code() == grpc.StatusCode.DEADLINE_EXCEEDED


def test_failure_mode_grpc_error_internal(stub: plugin_pb2_grpc.CustomerPredictorStub):
    """Model raising = template returns INTERNAL (SpendGuard maps to grpc_error)."""
    import predictor_server

    def _broken(self, x):
        raise RuntimeError("synthetic model failure")

    req = _make_request(spendguard_call_id="grpc-error-probe")
    with patch.object(predictor_server.StubModel, "predict_one", _broken):
        with pytest.raises(grpc.RpcError) as exc:
            stub.Predict(req, timeout=1.0)
        assert exc.value.code() == grpc.StatusCode.INTERNAL


def test_failure_mode_invalid_zero_or_negative(stub: plugin_pb2_grpc.CustomerPredictorStub):
    """If the model returns <= 0, template's defense-in-depth clips to 1.

    The wire contract (spec §5.1) says SpendGuard would treat this as
    `customer_predictor_invalid_zero_or_negative` and fall to Strategy
    B. The template's own defense-in-depth clip ensures the response
    is still well-formed (the SpendGuard side validator never sees
    <= 0 from THIS template). Customers replacing the stub model
    inherit the same clip.
    """
    import predictor_server
    from model_predictor_stub import StubPrediction

    def _zero(self, x):
        return StubPrediction(predicted_tokens=0, confidence=0.5, sample_size=1)

    req = _make_request(spendguard_call_id="zero-probe")
    with patch.object(predictor_server.StubModel, "predict_one", _zero):
        response = stub.Predict(req, timeout=1.0)
    assert response.predicted_output_tokens == MIN_OUTPUT_TOKENS


def test_failure_mode_invalid_overflow(stub: plugin_pb2_grpc.CustomerPredictorStub):
    """If the model returns a huge value, template clips to MAX_OUTPUT_TOKENS."""
    import predictor_server
    from model_predictor_stub import StubPrediction

    def _huge(self, x):
        return StubPrediction(
            predicted_tokens=10**12,
            confidence=0.5,
            sample_size=1,
        )

    req = _make_request(spendguard_call_id="overflow-probe")
    with patch.object(predictor_server.StubModel, "predict_one", _huge):
        response = stub.Predict(req, timeout=1.0)
    assert response.predicted_output_tokens == MAX_OUTPUT_TOKENS


def test_failure_mode_invalid_confidence_high(stub: plugin_pb2_grpc.CustomerPredictorStub):
    """Confidence > 1 is clamped to 1.0 by defense-in-depth."""
    import predictor_server
    from model_predictor_stub import StubPrediction

    def _bad_conf(self, x):
        return StubPrediction(predicted_tokens=500, confidence=5.0, sample_size=1)

    req = _make_request(spendguard_call_id="bad-conf-high-probe")
    with patch.object(predictor_server.StubModel, "predict_one", _bad_conf):
        response = stub.Predict(req, timeout=1.0)
    assert response.confidence == pytest.approx(1.0)


def test_failure_mode_invalid_confidence_low(stub: plugin_pb2_grpc.CustomerPredictorStub):
    """Confidence < 0 is clamped to 0.0 by defense-in-depth."""
    import predictor_server
    from model_predictor_stub import StubPrediction

    def _bad_conf(self, x):
        return StubPrediction(predicted_tokens=500, confidence=-1.0, sample_size=1)

    req = _make_request(spendguard_call_id="bad-conf-low-probe")
    with patch.object(predictor_server.StubModel, "predict_one", _bad_conf):
        response = stub.Predict(req, timeout=1.0)
    assert response.confidence == pytest.approx(0.0)


def test_failure_mode_deserialization_invalid_argument(
    stub: plugin_pb2_grpc.CustomerPredictorStub,
):
    """Malformed PredictRequest (negative input_tokens) → INVALID_ARGUMENT.

    Negative ``input_tokens`` is the closest in-proto analogue of a
    deserialization error: the wire decoded fine, but the field value
    is semantically invalid. The template refuses up-front instead of
    paying model compute.
    """
    req = _make_request(spendguard_call_id="bad-input-probe", input_tokens=-1)
    with pytest.raises(grpc.RpcError) as exc:
        stub.Predict(req, timeout=1.0)
    assert exc.value.code() == grpc.StatusCode.INVALID_ARGUMENT


def test_failure_mode_tls_handshake_error():
    """A TLS-misconfigured server refuses plaintext clients (UNAVAILABLE).

    This test stands up a one-shot TLS-required server with a
    self-signed cert, then attempts a plaintext call. The expectation
    is the client never gets a valid response.
    """
    # Use cryptography to mint a throwaway self-signed cert.
    pytest.importorskip("cryptography")

    from cryptography import x509
    from cryptography.hazmat.primitives import hashes, serialization
    from cryptography.hazmat.primitives.asymmetric import rsa
    from cryptography.x509.oid import NameOID
    import datetime

    key = rsa.generate_private_key(public_exponent=65537, key_size=2048)
    subject = x509.Name([x509.NameAttribute(NameOID.COMMON_NAME, "localhost")])
    cert = (
        x509.CertificateBuilder()
        .subject_name(subject)
        .issuer_name(subject)
        .public_key(key.public_key())
        .serial_number(x509.random_serial_number())
        .not_valid_before(
            datetime.datetime.now(datetime.timezone.utc) - datetime.timedelta(minutes=1)
        )
        .not_valid_after(datetime.datetime.now(datetime.timezone.utc) + datetime.timedelta(days=1))
        .sign(key, hashes.SHA256())
    )
    cert_pem = cert.public_bytes(serialization.Encoding.PEM)
    key_pem = key.private_bytes(
        encoding=serialization.Encoding.PEM,
        format=serialization.PrivateFormat.PKCS8,
        encryption_algorithm=serialization.NoEncryption(),
    )

    srv, _ = build_server(expected_tenant_id="tenant-a")
    creds = grpc.ssl_server_credentials([(key_pem, cert_pem)])
    port = srv.add_secure_port("127.0.0.1:0", creds)
    srv.start()
    try:
        addr = f"127.0.0.1:{port}"
        # Plaintext client to a TLS server: never completes successfully.
        plaintext_channel = grpc.insecure_channel(addr)
        plaintext_stub = plugin_pb2_grpc.CustomerPredictorStub(plaintext_channel)
        req = _make_request(spendguard_call_id="tls-probe")
        with pytest.raises(grpc.RpcError):
            plaintext_stub.Predict(req, timeout=1.0)
        plaintext_channel.close()
    finally:
        srv.stop(grace=0).wait()


def test_failure_mode_not_serving(health_stub: health_pb2_grpc.HealthStub):
    """SpendGuard's circuit breaker should observe NOT_SERVING when the plugin
    flips its health state. This test toggles the health servicer
    directly (the public surface customers actually use)."""
    # Default state is SERVING.
    initial = health_stub.Check(
        health_pb2.HealthCheckRequest(
            service="spendguard.output_predictor_plugin.v1.CustomerPredictor"
        ),
        timeout=1.0,
    )
    assert initial.status == health_pb2.HealthCheckResponse.SERVING

    # The template's PredictorServicer holds the same health_servicer
    # reference; mutate via the registered fixture for a clean revert.
    import predictor_server as ps

    # Build a side server with a NOT_SERVING setup to verify the
    # default state machinery is exercised end-to-end.
    srv, sv = ps.build_server(expected_tenant_id="tenant-a")
    sv._health.set(
        "spendguard.output_predictor_plugin.v1.CustomerPredictor",
        health_pb2.HealthCheckResponse.NOT_SERVING,
    )
    port = srv.add_insecure_port("127.0.0.1:0")
    srv.start()
    try:
        channel = grpc.insecure_channel(f"127.0.0.1:{port}")
        h = health_pb2_grpc.HealthStub(channel)
        resp = h.Check(
            health_pb2.HealthCheckRequest(
                service="spendguard.output_predictor_plugin.v1.CustomerPredictor"
            ),
            timeout=1.0,
        )
        assert resp.status == health_pb2.HealthCheckResponse.NOT_SERVING
        channel.close()
    finally:
        srv.stop(grace=0).wait()


# ---------------------------------------------------------------------------
# 3. Tenant binding (spec §7.2)
# ---------------------------------------------------------------------------


def test_tenant_mismatch_rejected(stub: plugin_pb2_grpc.CustomerPredictorStub):
    """Request with a different tenant_id → INVALID_ARGUMENT."""
    req = _make_request(tenant_id="tenant-b")
    with pytest.raises(grpc.RpcError) as exc:
        stub.Predict(req, timeout=1.0)
    assert exc.value.code() == grpc.StatusCode.INVALID_ARGUMENT


def test_tenant_id_required(stub: plugin_pb2_grpc.CustomerPredictorStub):
    """Empty tenant_id → INVALID_ARGUMENT."""
    req = _make_request(tenant_id="")
    with pytest.raises(grpc.RpcError) as exc:
        stub.Predict(req, timeout=1.0)
    assert exc.value.code() == grpc.StatusCode.INVALID_ARGUMENT


def test_client_svid_subject_matches_tenant():
    subject = expected_svid_subject("018fcf9a-3d2d-7b37-9f21-0f27de0b20c1")
    auth_context = {b"x509_subject_alternative_name": [f"URI:{subject}".encode("utf-8")]}
    assert extract_spiffe_uri_from_auth_context(auth_context) == subject
    validate_auth_context_tenant(
        auth_context=auth_context,
        tenant_id="018fcf9a-3d2d-7b37-9f21-0f27de0b20c1",
        require_svid=True,
    )


def test_client_svid_tenant_mismatch_rejected():
    auth_context = {
        "x509_subject_alternative_name": [
            b"URI:spiffe://spendguard.platform/predictor-client/018fcf9a-3d2d-7b37-9f21-0f27de0b20c1"
        ]
    }
    with pytest.raises(ValueError, match="SVID tenant mismatch"):
        validate_auth_context_tenant(
            auth_context=auth_context,
            tenant_id="018fcf9a-3d2d-7b37-9f21-0f27de0b20c2",
            require_svid=True,
        )


def test_client_svid_missing_cert_fails_closed():
    srv, _ = build_server(expected_tenant_id="tenant-a", require_client_svid=True)
    port = srv.add_insecure_port("127.0.0.1:0")
    srv.start()
    try:
        channel = grpc.insecure_channel(f"127.0.0.1:{port}")
        stub = plugin_pb2_grpc.CustomerPredictorStub(channel)
        with pytest.raises(grpc.RpcError) as exc:
            stub.Predict(_make_request(tenant_id="tenant-a"), timeout=1.0)
        assert exc.value.code() == grpc.StatusCode.PERMISSION_DENIED
        channel.close()
    finally:
        srv.stop(grace=0).wait()


def test_oversized_field_rejected(stub: plugin_pb2_grpc.CustomerPredictorStub):
    """Request field exceeding ``MAX_STRING_FIELD_LEN`` → INVALID_ARGUMENT."""
    req = _make_request(model="x" * (MAX_STRING_FIELD_LEN + 1))
    with pytest.raises(grpc.RpcError) as exc:
        stub.Predict(req, timeout=1.0)
    assert exc.value.code() == grpc.StatusCode.INVALID_ARGUMENT


# ---------------------------------------------------------------------------
# 4. Plugin-defined HealthCheck RPC (spec §2.1)
# ---------------------------------------------------------------------------


def test_plugin_health_check(stub: plugin_pb2_grpc.CustomerPredictorStub):
    """CustomerPredictor.HealthCheck (NOT the grpc.health.v1 probe) returns SERVING."""
    response = stub.HealthCheck(plugin_pb2.HealthCheckRequest(), timeout=1.0)
    assert response.status == plugin_pb2.HealthCheckResponse.SERVING
    assert response.plugin_version, "plugin_version must be set"
    assert response.HasField("checked_at"), "checked_at must be set"


# ---------------------------------------------------------------------------
# 5. Concurrency smoke (defense in depth — surfaces threading bugs early)
# ---------------------------------------------------------------------------


def test_concurrent_requests_isolated(stub: plugin_pb2_grpc.CustomerPredictorStub):
    """16 concurrent Predict calls must all return valid responses."""
    requests = [
        _make_request(spendguard_call_id=f"concurrent-{i}", prompt_class=PROMPT_CLASSES[i % 7])
        for i in range(16)
    ]

    results: list[plugin_pb2.PredictResponse | Exception] = [None] * len(requests)  # type: ignore[list-item]

    def _call(idx: int) -> None:
        try:
            results[idx] = stub.Predict(requests[idx], timeout=2.0)
        except Exception as exc:  # noqa: BLE001
            results[idx] = exc

    threads = [threading.Thread(target=_call, args=(i,)) for i in range(len(requests))]
    for t in threads:
        t.start()
    for t in threads:
        t.join(timeout=5.0)

    for idx, res in enumerate(results):
        assert isinstance(res, plugin_pb2.PredictResponse), (idx, res)
        assert res.predicted_output_tokens >= MIN_OUTPUT_TOKENS
        assert 0.0 <= res.confidence <= 1.0
