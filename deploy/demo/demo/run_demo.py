"""End-to-end smoke test for the SpendGuard demo compose topology.

Wires the Pydantic-AI adapter against a Mock LLM model so the demo can
run offline without consuming real provider tokens. The Mock model
returns a fixed completion + a `Usage` object with `total_tokens=42` so
that the sidecar's LLM_CALL_POST → ledger commit lifecycle has a real
provider amount to record.

Boot order: docker compose up brings up postgres → migrations →
pki/bundles/manifest init → ledger / canonical-ingest (mTLS) →
endpoint-catalog (HTTPS) → sidecar (UDS) → demo.

Demo container's depends_on waits for sidecar's /readyz, but UDS-bind
ordering can still race with first-handshake — the script retries
handshake until success or DEMO_HANDSHAKE_TIMEOUT_S elapses.
"""

from __future__ import annotations

import asyncio
import os
import sys
import time
from collections.abc import Sequence
from dataclasses import dataclass
from typing import Any

# Pydantic-AI imports are deliberately deferred so the script also runs
# as a thin handshake/Decision smoke test without needing to instantiate
# an Agent. Mode select via the SPENDGUARD_DEMO_MODE env var:
#   "agent"      → run a Pydantic-AI Agent (default)
#   "decision"   → just call client.request_decision once and exit
DEMO_MODE = os.environ.get("SPENDGUARD_DEMO_MODE", "agent")
HANDSHAKE_TIMEOUT_S = float(os.environ.get("DEMO_HANDSHAKE_TIMEOUT_S", "30"))


def _env(name: str) -> str:
    val = os.environ.get(name)
    if not val:
        print(f"FATAL: env var {name} required", file=sys.stderr)
        sys.exit(2)
    return val


# ---------------------------------------------------------------------------
# Mock LLM Model
# ---------------------------------------------------------------------------


class _MockUsage:
    """Stub `pydantic_ai.usage.Usage` for the demo Mock model."""

    def __init__(self, total: int = 42, request: int = 12, response: int = 30) -> None:
        self.total_tokens = total
        self.request_tokens = request
        self.response_tokens = response


@dataclass
class _MockResponse:
    """Stub `pydantic_ai.messages.ModelResponse`-like.

    Pydantic-AI consumes the response via `.usage()` (callable),
    `.model_name`, and the message parts. For a simple Agent.run() that
    returns a string, returning an object with a `parts` field carrying
    a single TextPart-like is usually enough.
    """

    parts: list[Any]
    model_name: str = "mock-llm"

    def usage(self) -> _MockUsage:
        return _MockUsage()


class MockLLM:
    """Minimal pydantic-ai Model duck-type for the demo.

    Only implements `request()` (non-streaming). Modern Pydantic-AI's
    Model.request returns the tuple `(ModelResponse, Usage)`, not the
    bare ModelResponse — the framework unpacks both for usage limits
    accounting. The MockLLM mirrors that contract.
    """

    @property
    def model_name(self) -> str:
        return "mock-llm"

    @property
    def system(self) -> str:
        return "spendguard-demo"

    async def request(
        self,
        messages: Sequence[Any],
        model_settings: Any | None,
        model_request_parameters: Any,
        run_context: Any | None = None,
    ) -> Any:
        from pydantic_ai.messages import ModelResponse, TextPart
        from pydantic_ai.usage import Usage

        response = ModelResponse(
            parts=[TextPart(content="hello from the spendguard demo")],
            model_name=self.model_name,
        )
        usage = Usage(
            request_tokens=12,
            response_tokens=30,
            total_tokens=42,
        )
        return response, usage


# ---------------------------------------------------------------------------
# Decision-only mode (skip Agent; just verify handshake + RequestDecision)
# ---------------------------------------------------------------------------


async def run_decision_mode() -> int:
    from spendguard_pydantic_ai import (
        SpendGuardClient,
        derive_idempotency_key,
        new_uuid7,
    )
    from spendguard_pydantic_ai._proto.spendguard.common.v1 import common_pb2

    socket_path = _env("SPENDGUARD_SIDECAR_UDS")
    tenant_id = _env("SPENDGUARD_TENANT_ID")
    budget_id = _env("SPENDGUARD_BUDGET_ID")
    window_id = _env("SPENDGUARD_WINDOW_INSTANCE_ID")
    unit_id = _env("SPENDGUARD_UNIT_ID")
    pricing_version = _env("SPENDGUARD_PRICING_VERSION")
    fx = _env("SPENDGUARD_FX_RATE_VERSION")
    unit_conv = _env("SPENDGUARD_UNIT_CONVERSION_VERSION")
    snapshot_hash_hex = _env("SPENDGUARD_PRICE_SNAPSHOT_HASH_HEX")

    print(f"[demo] connecting to sidecar at {socket_path}")
    deadline = time.monotonic() + HANDSHAKE_TIMEOUT_S
    client: SpendGuardClient | None = None
    last_err: BaseException | None = None
    while time.monotonic() < deadline:
        try:
            c = SpendGuardClient(
                socket_path=socket_path,
                tenant_id=tenant_id,
            )
            await c.connect()
            await c.handshake()
            client = c
            break
        except Exception as e:  # noqa: BLE001 — we retry
            last_err = e
            await asyncio.sleep(1)
    if client is None:
        print(f"[demo] FATAL: handshake timeout: {last_err}", file=sys.stderr)
        return 3
    print(f"[demo] handshake ok session_id={client.session_id}")

    run_id = str(new_uuid7())
    step_id = f"{run_id}:step0"
    llm_call_id = str(new_uuid7())
    decision_id = str(new_uuid7())

    # Phase 2B Step 7: reserve 100 (out of seeded 500) so the commit path
    # can release a meaningful residual back to available_budget. After
    # commit 42 → available 458 (= 500 - 42), reserved_hold 0, committed 42.
    claims = [
        common_pb2.BudgetClaim(
            budget_id=budget_id,
            unit=common_pb2.UnitRef(
                unit_id=unit_id,
                token_kind="output_token",
                model_family="gpt-4",
            ),
            amount_atomic="100",
            direction=common_pb2.BudgetClaim.DEBIT,
            window_instance_id=window_id,
        ),
    ]
    pricing = common_pb2.PricingFreeze(
        pricing_version=pricing_version,
        price_snapshot_hash=bytes.fromhex(snapshot_hash_hex),
        fx_rate_version=fx,
        unit_conversion_version=unit_conv,
    )
    idempotency_key = derive_idempotency_key(
        tenant_id=tenant_id,
        session_id=client.session_id,
        run_id=run_id,
        step_id=step_id,
        llm_call_id=llm_call_id,
        trigger="LLM_CALL_PRE",
    )
    outcome = await client.request_decision(
        trigger="LLM_CALL_PRE",
        run_id=run_id,
        step_id=step_id,
        llm_call_id=llm_call_id,
        tool_call_id="",
        decision_id=decision_id,
        route="llm.call",
        projected_claims=claims,
        idempotency_key=idempotency_key,
    )
    print(
        f"[demo] decision OK decision_id={outcome.decision_id} "
        f"decision={outcome.decision} "
        f"ledger_transaction_id={outcome.ledger_transaction_id} "
        f"reservation_ids={list(outcome.reservation_ids)}"
    )

    await client.confirm_publish_outcome(
        decision_id=outcome.decision_id,
        effect_hash=outcome.effect_hash,
        outcome="APPLIED_NOOP",
    )
    print("[demo] confirm_publish_outcome ok")

    # Phase 2B Step 7: drive Stage 7 commit lane. Mock LLM returns 42
    # tokens; sidecar routes LLM_CALL_POST.SUCCESS with
    # estimated_amount_atomic="42" to Ledger.CommitEstimated, which
    # commits 42 + releases residual 58 back to available_budget.
    # Per-account assertions are run from `make demo-verify-step7` via
    # psql against the postgres container; the demo runner deliberately
    # does NOT carry mTLS material to talk to ledger directly (that is
    # the sidecar's role).
    if not outcome.reservation_ids:
        print("[demo] FATAL: no reservation_id returned", file=sys.stderr)
        return 4
    reservation_id = list(outcome.reservation_ids)[0]
    await client.emit_llm_call_post(
        run_id=run_id,
        step_id=step_id,
        llm_call_id=llm_call_id,
        decision_id=outcome.decision_id,
        reservation_id=reservation_id,
        provider_reported_amount_atomic="",
        estimated_amount_atomic="42",
        unit=claims[0].unit,
        pricing=pricing,
        provider_event_id="mock-llm-1",
        outcome="SUCCESS",
    )
    print(f"[demo] emit_llm_call_post ok (estimated=42 reservation={reservation_id})")
    await client.close()

    # Phase 2B Step 8: simulate webhook receiver Provider Report.
    # Per spec D9 + §8.2.3, ProviderReport is webhook-originated; demo
    # simulates by talking directly to ledger gRPC over mTLS. Production
    # webhook receiver would do the same with provider event signature
    # verification + Redis dedupe.
    # Codex Step 8 challenge P2.3: derive provider_event_id from
    # reservation_id so reruns produce a fresh idempotency key (avoids
    # request-hash conflict on second `make demo-up`).
    rc = await simulate_webhook_provider_report(
        tenant_id=tenant_id,
        budget_id=budget_id,
        window_id=window_id,
        unit_id=unit_id,
        pricing=pricing,
        reservation_id=reservation_id,
        provider_amount_atomic="38",
        provider="mock-llm",
        provider_account="demo-tenant",
        provider_event_id=f"evt-{reservation_id[:8]}",
    )
    if rc != 0:
        return rc
    return 0


async def simulate_webhook_provider_report(
    *,
    tenant_id: str,
    budget_id: str,
    window_id: str,
    unit_id: str,
    pricing: "common_pb2.PricingFreeze",  # type: ignore[name-defined]
    reservation_id: str,
    provider_amount_atomic: str,
    provider: str,
    provider_account: str,
    provider_event_id: str,
) -> int:
    """Step 8 demo: open ledger mTLS gRPC client and call ProviderReport
    with webhook-namespaced decision_id + idempotency_key derived from
    sha256("provider_report:{provider}:{provider_account}:{provider_event_id}").
    """
    import hashlib
    import grpc.aio
    from google.protobuf import timestamp_pb2
    from spendguard_pydantic_ai import new_uuid7
    from spendguard_pydantic_ai._proto.spendguard.common.v1 import common_pb2 as cpb
    from spendguard_pydantic_ai._proto.spendguard.ledger.v1 import (
        ledger_pb2 as lpb,
        ledger_pb2_grpc as lpb_grpc,
    )

    ledger_url = _env("SPENDGUARD_LEDGER_URL")
    cert_path = _env("SPENDGUARD_DEMO_TLS_CERT_PEM")
    key_path = _env("SPENDGUARD_DEMO_TLS_KEY_PEM")
    ca_path = _env("SPENDGUARD_DEMO_TLS_CA_PEM")
    fencing_scope_id = _env("SPENDGUARD_DEMO_WEBHOOK_FENCING_SCOPE_ID")
    workload_instance_id = _env("SPENDGUARD_DEMO_WEBHOOK_WORKLOAD_INSTANCE_ID")

    # Strip "https://" for grpc target.
    target = ledger_url.removeprefix("https://").removeprefix("http://")

    with open(cert_path, "rb") as f:
        cert_bytes = f.read()
    with open(key_path, "rb") as f:
        key_bytes = f.read()
    with open(ca_path, "rb") as f:
        ca_bytes = f.read()
    creds = grpc.ssl_channel_credentials(
        root_certificates=ca_bytes,
        private_key=key_bytes,
        certificate_chain=cert_bytes,
    )
    options = [("grpc.ssl_target_name_override", "ledger.spendguard.internal")]

    # Webhook-namespaced identity (Codex round 2 M2.1).
    namespaced = f"provider_report:{provider}:{provider_account}:{provider_event_id}"
    digest = hashlib.sha256(namespaced.encode()).digest()
    decision_id_bytes = bytearray(digest[:16])
    decision_id_bytes[6] = (decision_id_bytes[6] & 0x0F) | 0x40  # v4
    decision_id_bytes[8] = (decision_id_bytes[8] & 0x3F) | 0x80
    decision_id = (
        decision_id_bytes[0:4].hex()
        + "-"
        + decision_id_bytes[4:6].hex()
        + "-"
        + decision_id_bytes[6:8].hex()
        + "-"
        + decision_id_bytes[8:10].hex()
        + "-"
        + decision_id_bytes[10:16].hex()
    )
    audit_outbox_id = str(new_uuid7())

    ts = timestamp_pb2.Timestamp()
    ts.GetCurrentTime()

    cloud_event = cpb.CloudEvent(
        specversion="1.0",
        type="spendguard.audit.decision",
        source=f"webhook-receiver://{provider}/{provider_account}",
        id=audit_outbox_id,
        time=ts,
        datacontenttype="application/json",
        data=(
            '{"kind":"provider_report","provider":"' + provider + '",'
            '"provider_account":"' + provider_account + '",'
            '"provider_event_id":"' + provider_event_id + '",'
            '"provider_amount_atomic":"' + provider_amount_atomic + '"}'
        ).encode(),
        tenant_id=tenant_id,
        decision_id=decision_id,
        producer_id=f"demo-webhook-receiver:{workload_instance_id}",
        producer_sequence=1,
    )

    request = lpb.ProviderReportRequest(
        tenant_id=tenant_id,
        reservation_id=reservation_id,
        provider_reported_amount_atomic=provider_amount_atomic,
        unit=cpb.UnitRef(unit_id=unit_id),
        provider_response_metadata=namespaced,
        idempotency=cpb.Idempotency(key=namespaced, request_hash=b""),
        fencing=cpb.Fencing(
            epoch=1,
            scope_id=fencing_scope_id,
            workload_instance_id=workload_instance_id,
        ),
        pricing=pricing,
        audit_event=cloud_event,
        producer_sequence=1,
        decision_id=decision_id,
    )

    print(f"[demo] webhook simulator -> ledger ProviderReport target={target}")
    async with grpc.aio.secure_channel(target, creds, options=options) as channel:
        stub = lpb_grpc.LedgerStub(channel)
        try:
            resp = await stub.ProviderReport(request, timeout=5.0)
        except grpc.aio.AioRpcError as e:
            print(
                f"[demo] FATAL: ProviderReport RPC failed: code={e.code()} details={e.details()}",
                file=sys.stderr,
            )
            return 6
    outcome = resp.WhichOneof("outcome")
    if outcome == "success":
        s = resp.success
        print(
            f"[demo] ProviderReport success ledger_tx={s.ledger_transaction_id} "
            f"reservation={s.reservation_id} delta_to_reserved={s.delta_to_reserved_atomic}"
        )
        return 0
    if outcome == "replay":
        r = resp.replay
        print(f"[demo] ProviderReport replay ledger_tx={r.ledger_transaction_id}")
        return 0
    err = resp.error
    print(
        f"[demo] FATAL: ProviderReport returned Error code={err.code} message={err.message}",
        file=sys.stderr,
    )
    return 7


# ---------------------------------------------------------------------------
# Agent mode (full Pydantic-AI Agent.run() with Mock LLM)
# ---------------------------------------------------------------------------


async def run_agent_mode() -> int:
    from pydantic_ai import Agent

    from spendguard_pydantic_ai import (
        RunContext,
        SpendGuardClient,
        SpendGuardModel,
        new_uuid7,
        run_context,
    )
    from spendguard_pydantic_ai._proto.spendguard.common.v1 import common_pb2

    socket_path = _env("SPENDGUARD_SIDECAR_UDS")
    tenant_id = _env("SPENDGUARD_TENANT_ID")
    budget_id = _env("SPENDGUARD_BUDGET_ID")
    window_id = _env("SPENDGUARD_WINDOW_INSTANCE_ID")
    unit_id = _env("SPENDGUARD_UNIT_ID")
    pricing_version = _env("SPENDGUARD_PRICING_VERSION")
    fx = _env("SPENDGUARD_FX_RATE_VERSION")
    unit_conv = _env("SPENDGUARD_UNIT_CONVERSION_VERSION")
    snapshot_hash_hex = _env("SPENDGUARD_PRICE_SNAPSHOT_HASH_HEX")

    def estimate_claims(
        messages: Sequence[Any],
        model_settings: Any | None,
    ) -> list[common_pb2.BudgetClaim]:
        return [
            common_pb2.BudgetClaim(
                budget_id=budget_id,
                unit=common_pb2.UnitRef(
                    unit_id=unit_id,
                    token_kind="output_token",
                    model_family="gpt-4",
                ),
                amount_atomic="500",
                direction=common_pb2.BudgetClaim.DEBIT,
                window_instance_id=window_id,
            ),
        ]

    print(f"[demo] connecting to sidecar at {socket_path}")
    async with SpendGuardClient(
        socket_path=socket_path,
        tenant_id=tenant_id,
    ) as client:
        # Retry handshake during compose race window.
        deadline = time.monotonic() + HANDSHAKE_TIMEOUT_S
        last_err: BaseException | None = None
        while time.monotonic() < deadline:
            try:
                await client.handshake()
                break
            except Exception as e:  # noqa: BLE001
                last_err = e
                await asyncio.sleep(1)
        else:
            print(f"[demo] FATAL: handshake timeout: {last_err}", file=sys.stderr)
            return 3
        print(f"[demo] handshake ok session_id={client.session_id}")

        guarded = SpendGuardModel(
            inner=MockLLM(),
            client=client,
            budget_id=budget_id,
            window_instance_id=window_id,
            unit=common_pb2.UnitRef(
                unit_id=unit_id,
                token_kind="output_token",
                model_family="gpt-4",
            ),
            pricing=common_pb2.PricingFreeze(
                pricing_version=pricing_version,
                price_snapshot_hash=bytes.fromhex(snapshot_hash_hex),
                fx_rate_version=fx,
                unit_conversion_version=unit_conv,
            ),
            claim_estimator=estimate_claims,
        )

        agent = Agent(model=guarded)
        run_id = str(new_uuid7())
        async with run_context(RunContext(run_id=run_id)):
            result = await agent.run("Say hello in three words.")
        # AgentRunResult attribute name shifted across pydantic-ai
        # versions ("output" / "data"); resolve at runtime.
        output = getattr(result, "output", None)
        if output is None:
            output = getattr(result, "data", None)
        print(f"[demo] agent.run() OK output={output!r} run_id={run_id}")

    return 0


# ---------------------------------------------------------------------------
# Entry point
# ---------------------------------------------------------------------------


async def main() -> int:
    if DEMO_MODE == "decision":
        return await run_decision_mode()
    if DEMO_MODE == "release":
        return await run_release_mode()
    return await run_agent_mode()


# ---------------------------------------------------------------------------
# Release mode (Phase 2B Step 7.5): reserve → emit_llm_call_post(RUN_ABORTED)
# → sidecar routes to Ledger.Release → reservation released, full refund.
# ---------------------------------------------------------------------------


async def run_release_mode() -> int:
    from spendguard_pydantic_ai import (
        SpendGuardClient,
        derive_idempotency_key,
        new_uuid7,
    )
    from spendguard_pydantic_ai._proto.spendguard.common.v1 import common_pb2

    socket_path = _env("SPENDGUARD_SIDECAR_UDS")
    tenant_id = _env("SPENDGUARD_TENANT_ID")
    budget_id = _env("SPENDGUARD_BUDGET_ID")
    window_id = _env("SPENDGUARD_WINDOW_INSTANCE_ID")
    unit_id = _env("SPENDGUARD_UNIT_ID")
    pricing_version = _env("SPENDGUARD_PRICING_VERSION")
    fx = _env("SPENDGUARD_FX_RATE_VERSION")
    unit_conv = _env("SPENDGUARD_UNIT_CONVERSION_VERSION")
    snapshot_hash_hex = _env("SPENDGUARD_PRICE_SNAPSHOT_HASH_HEX")

    print(f"[demo] connecting to sidecar at {socket_path}")
    deadline = time.monotonic() + HANDSHAKE_TIMEOUT_S
    client: SpendGuardClient | None = None
    last_err: BaseException | None = None
    while time.monotonic() < deadline:
        try:
            c = SpendGuardClient(socket_path=socket_path, tenant_id=tenant_id)
            await c.connect()
            await c.handshake()
            client = c
            break
        except Exception as e:  # noqa: BLE001
            last_err = e
            await asyncio.sleep(1)
    if client is None:
        print(f"[demo] FATAL: handshake timeout: {last_err}", file=sys.stderr)
        return 3
    print(f"[demo] handshake ok session_id={client.session_id}")

    run_id = str(new_uuid7())
    step_id = f"{run_id}:step0"
    llm_call_id = str(new_uuid7())
    decision_id = str(new_uuid7())

    claims = [
        common_pb2.BudgetClaim(
            budget_id=budget_id,
            unit=common_pb2.UnitRef(
                unit_id=unit_id,
                token_kind="output_token",
                model_family="gpt-4",
            ),
            amount_atomic="100",
            direction=common_pb2.BudgetClaim.DEBIT,
            window_instance_id=window_id,
        ),
    ]
    pricing = common_pb2.PricingFreeze(
        pricing_version=pricing_version,
        price_snapshot_hash=bytes.fromhex(snapshot_hash_hex),
        fx_rate_version=fx,
        unit_conversion_version=unit_conv,
    )
    idempotency_key = derive_idempotency_key(
        tenant_id=tenant_id,
        session_id=client.session_id,
        run_id=run_id,
        step_id=step_id,
        llm_call_id=llm_call_id,
        trigger="LLM_CALL_PRE",
    )
    outcome = await client.request_decision(
        trigger="LLM_CALL_PRE",
        run_id=run_id,
        step_id=step_id,
        llm_call_id=llm_call_id,
        tool_call_id="",
        decision_id=decision_id,
        route="llm.call",
        projected_claims=claims,
        idempotency_key=idempotency_key,
    )
    print(
        f"[demo] decision OK decision_id={outcome.decision_id} "
        f"reservation_ids={list(outcome.reservation_ids)}"
    )

    if not outcome.reservation_ids:
        print("[demo] FATAL: no reservation_id returned", file=sys.stderr)
        return 4
    reservation_id = list(outcome.reservation_ids)[0]

    # Emit LLM_CALL_POST with outcome=RUN_ABORTED → sidecar routes to
    # Ledger.Release. estimated_amount_atomic + provider_reported_amount_atomic
    # are both empty (no commit; release path).
    await client.emit_llm_call_post(
        run_id=run_id,
        step_id=step_id,
        llm_call_id=llm_call_id,
        decision_id=outcome.decision_id,
        reservation_id=reservation_id,
        provider_reported_amount_atomic="",
        estimated_amount_atomic="",
        unit=claims[0].unit,
        pricing=pricing,
        provider_event_id=f"abort-{reservation_id[:8]}",
        outcome="RUN_ABORTED",
    )
    print(f"[demo] emit_llm_call_post(RUN_ABORTED) ok reservation={reservation_id}")
    await client.close()
    return 0


if __name__ == "__main__":
    sys.exit(asyncio.run(main()))
