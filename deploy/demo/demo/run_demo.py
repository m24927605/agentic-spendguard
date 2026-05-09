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


async def run_decision_mode(also_invoice: bool = False) -> int:
    from spendguard import (
        SpendGuardClient,
        derive_idempotency_key,
        new_uuid7,
    )
    from spendguard._proto.spendguard.common.v1 import common_pb2

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
    if also_invoice:
        # Phase 2B Step 9: simulate webhook receiver InvoiceReconcile.
        # Chain: reserve(100)→commit(42)→provider_report(38)→invoice(40).
        # delta = invoice - provider_reported = 40 - 38 = +2.
        # Final per-account: available=460, committed=40, reserved=0.
        # Webhook simulator allocates 2 contiguous seqs (decision=2, outcome=3)
        # in same workload-instance space as provider_report (seq=1).
        rc = await simulate_webhook_invoice_reconcile(
            tenant_id=tenant_id,
            budget_id=budget_id,
            window_id=window_id,
            unit_id=unit_id,
            pricing=pricing,
            reservation_id=reservation_id,
            invoice_amount_atomic="40",
            provider="mock-llm",
            provider_account="demo-tenant",
            provider_invoice_id=f"inv-{reservation_id[:8]}",
            outcome_producer_seq=3,
        )
        if rc != 0:
            return rc
    return 0


async def _post_webhook_event(
    *,
    event_kind: str,
    tenant_id: str,
    reservation_id: str,
    unit_id: str,
    pricing: "common_pb2.PricingFreeze",  # type: ignore[name-defined]
    amount_atomic: str,
    provider: str,
    provider_account: str,
    provider_event_id: str,
) -> int:
    """Phase 2B Step 11: POST webhook event to receiver service.

    Replaces the prior in-process simulator that called Ledger gRPC
    directly. Receiver verifies HMAC, dedupes, then invokes Ledger gRPC
    on caller's behalf (Stage 2 §8.2.3 + §11).
    """
    import hashlib
    import hmac
    import json
    import httpx

    receiver_url = _env("SPENDGUARD_DEMO_WEBHOOK_RECEIVER_URL")
    secret = _env("SPENDGUARD_WEBHOOK_SECRET_MOCK_LLM")
    ca_path = _env("SPENDGUARD_DEMO_TLS_CA_PEM")

    body_obj = {
        "event_kind": event_kind,
        "tenant_id": tenant_id,
        "provider_account": provider_account,
        "provider_event_id": provider_event_id,
        "reservation_id": reservation_id,
        "amount_atomic": amount_atomic,
        "unit_id": unit_id,
        "pricing": {
            "pricing_version": pricing.pricing_version,
            "price_snapshot_hash_hex": pricing.price_snapshot_hash.hex(),
            "fx_rate_version": pricing.fx_rate_version,
            "unit_conversion_version": pricing.unit_conversion_version,
        },
    }
    # Pre-serialize to bytes; HMAC over EXACT bytes the receiver sees.
    body_bytes = json.dumps(body_obj, separators=(",", ":"), sort_keys=True).encode()
    sig = hmac.new(secret.encode(), body_bytes, hashlib.sha256).hexdigest()

    url = f"{receiver_url}/v1/webhook/{provider}"
    print(f"[demo] webhook -> receiver {event_kind} url={url}")
    try:
        async with httpx.AsyncClient(verify=ca_path, timeout=10.0) as client:
            resp = await client.post(
                url,
                content=body_bytes,
                headers={
                    "Content-Type": "application/json",
                    "X-SpendGuard-Signature": sig,
                },
            )
    except httpx.HTTPError as e:
        print(f"[demo] FATAL: webhook POST failed: {e}", file=sys.stderr)
        return 6

    if resp.status_code != 200:
        print(
            f"[demo] FATAL: webhook {event_kind} returned {resp.status_code}: {resp.text}",
            file=sys.stderr,
        )
        return 7
    body = resp.json()
    print(
        f"[demo] webhook {event_kind} {body.get('outcome')} ledger_tx={body.get('ledger_transaction_id')}"
    )
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
    """Phase 2B Step 11: routes through HTTPS webhook receiver service.

    Was Step 8 in-process simulator; now thin wrapper that POSTs to the
    real receiver. Receiver handles signature, dedupe, sequence allocation,
    CloudEvent construction, and Ledger gRPC call.
    """
    return await _post_webhook_event(
        event_kind="provider_report",
        tenant_id=tenant_id,
        reservation_id=reservation_id,
        unit_id=unit_id,
        pricing=pricing,
        amount_atomic=provider_amount_atomic,
        provider=provider,
        provider_account=provider_account,
        provider_event_id=provider_event_id,
    )


async def simulate_webhook_invoice_reconcile(
    *,
    tenant_id: str,
    budget_id: str,
    window_id: str,
    unit_id: str,
    pricing: "common_pb2.PricingFreeze",  # type: ignore[name-defined]
    reservation_id: str,
    invoice_amount_atomic: str,
    provider: str,
    provider_account: str,
    provider_invoice_id: str,
    outcome_producer_seq: int,  # POC: kept for caller compat; unused now
) -> int:
    """Phase 2B Step 11: routes through HTTPS webhook receiver service.

    Receiver allocates producer_sequence(s) internally via cold-path DB
    recovery; caller-supplied outcome_producer_seq is no longer needed
    (kept in signature for caller compat — Step 9 demo flow).
    """
    _ = (budget_id, window_id, outcome_producer_seq)  # unused in receiver mode
    return await _post_webhook_event(
        event_kind="invoice_reconcile",
        tenant_id=tenant_id,
        reservation_id=reservation_id,
        unit_id=unit_id,
        pricing=pricing,
        amount_atomic=invoice_amount_atomic,
        provider=provider,
        provider_account=provider_account,
        provider_event_id=provider_invoice_id,  # provider_invoice_id is the event_id namespace
    )


# ---------------------------------------------------------------------------
# Agent mode (full Pydantic-AI Agent.run() with Mock LLM)
# ---------------------------------------------------------------------------


async def run_agent_mode(
    use_real_openai: bool = False,
    use_real_anthropic: bool = False,
) -> int:
    from pydantic_ai import Agent

    from spendguard import SpendGuardClient, new_uuid7
    from spendguard.integrations.pydantic_ai import (
        RunContext,
        SpendGuardModel,
        run_context,
    )
    from spendguard._proto.spendguard.common.v1 import common_pb2

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

        # Phase 3 follow-up: wire real OpenAI model when DEMO_MODE=agent_real.
        # Confirms SpendGuard's LLM_CALL_PRE/POST + commit_estimated lifecycle
        # works against a real provider's `usage.total_tokens`, not just MockLLM.
        if use_real_openai:
            from pydantic_ai.models.openai import OpenAIModel

            if not os.environ.get("OPENAI_API_KEY"):
                print("[demo] FATAL: OPENAI_API_KEY required for agent_real mode", file=sys.stderr)
                return 8
            # OpenAI SDK reads OPENAI_API_KEY from env automatically;
            # pydantic-ai 0.0.x OpenAIModel takes only the model name.
            inner_model: Any = OpenAIModel("gpt-4o-mini")
            print("[demo] using real OpenAI gpt-4o-mini")
        elif use_real_anthropic:
            from pydantic_ai.models.anthropic import AnthropicModel

            if not os.environ.get("ANTHROPIC_API_KEY"):
                print(
                    "[demo] FATAL: ANTHROPIC_API_KEY required for agent_real_anthropic mode",
                    file=sys.stderr,
                )
                return 9
            # Anthropic SDK reads ANTHROPIC_API_KEY from env automatically.
            # claude-3-5-haiku is the cheapest claude that returns usage in
            # the response (input_tokens + output_tokens).
            inner_model = AnthropicModel("claude-haiku-4-5-20251001")
            print("[demo] using real Anthropic claude-haiku-4-5-20251001")
        else:
            inner_model = MockLLM()

        guarded = SpendGuardModel(
            inner=inner_model,
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
    if DEMO_MODE == "invoice":
        return await run_decision_mode(also_invoice=True)
    if DEMO_MODE == "release":
        return await run_release_mode()
    if DEMO_MODE == "ttl_sweep":
        return await run_ttl_sweep_mode()
    if DEMO_MODE == "deny":
        return await run_deny_mode()
    if DEMO_MODE == "agent_real":
        return await run_agent_mode(use_real_openai=True)
    if DEMO_MODE == "agent_real_anthropic":
        return await run_agent_mode(use_real_anthropic=True)
    return await run_agent_mode()


# ---------------------------------------------------------------------------
# Deny mode (Phase 3 wedge): contract evaluator returns STOP for a claim
# above the bundle's hard-cap rule.
# ---------------------------------------------------------------------------


async def run_deny_mode() -> int:
    from spendguard import (
        SpendGuardClient,
        derive_idempotency_key,
        new_uuid7,
    )
    from spendguard._proto.spendguard.common.v1 import common_pb2

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

    # Phase 3 wedge demo: claim 2_000_000_000 atomic ($2000) — twice the
    # hard-cap rule threshold (1_000_000_000 / $1000) shipped in the
    # demo contract bundle. Sidecar's contract evaluator matches
    # `hard-cap-deny` and short-circuits Reserve, calling
    # Ledger.RecordDeniedDecision instead.
    claims = [
        common_pb2.BudgetClaim(
            budget_id=budget_id,
            unit=common_pb2.UnitRef(
                unit_id=unit_id,
                token_kind="output_token",
                model_family="gpt-4",
            ),
            amount_atomic="2000000000",
            direction=common_pb2.BudgetClaim.DEBIT,
            window_instance_id=window_id,
        ),
    ]
    _ = (pricing_version, fx, unit_conv, snapshot_hash_hex)
    idempotency_key = derive_idempotency_key(
        tenant_id=tenant_id,
        session_id=client.session_id,
        run_id=run_id,
        step_id=step_id,
        llm_call_id=llm_call_id,
        trigger="LLM_CALL_PRE",
    )

    # Adapter raises DecisionStopped on STOP (per
    # adapters/pydantic-ai/.../client.py:474). The exception carries
    # decision_id, reason_codes, audit_decision_event_id, and
    # matched_rule_ids — exactly what we want to assert in the wedge demo.
    from spendguard.errors import DecisionStopped

    try:
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
    except DecisionStopped as e:
        print(
            f"[demo] DENY raised decision_id={e.decision_id} "
            f"reason_codes={e.reason_codes} "
            f"matched_rule_ids={e.matched_rule_ids} "
            f"audit_decision_event_id={e.audit_decision_event_id}"
        )
        if "BUDGET_EXHAUSTED" not in e.reason_codes:
            print(
                f"[demo] FATAL: expected reason_code BUDGET_EXHAUSTED, got {e.reason_codes}",
                file=sys.stderr,
            )
            await client.close()
            return 5
        if not e.matched_rule_ids:
            print(
                "[demo] FATAL: DENY produced empty matched_rule_ids — audit forensics gap",
                file=sys.stderr,
            )
            await client.close()
            return 7
        print("[demo] DENY assertions PASS")
        await client.close()
        return 0

    # If we get here, sidecar returned CONTINUE/DEGRADE — wedge failure.
    print(
        f"[demo] FATAL: expected DecisionStopped (STOP) but got "
        f"decision={outcome.decision} reservation_ids={list(outcome.reservation_ids)}",
        file=sys.stderr,
    )
    await client.close()
    return 4


# ---------------------------------------------------------------------------
# Release mode (Phase 2B Step 7.5): reserve → emit_llm_call_post(RUN_ABORTED)
# → sidecar routes to Ledger.Release → reservation released, full refund.
# ---------------------------------------------------------------------------


async def run_release_mode() -> int:
    from spendguard import (
        SpendGuardClient,
        derive_idempotency_key,
        new_uuid7,
    )
    from spendguard._proto.spendguard.common.v1 import common_pb2

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


async def run_ttl_sweep_mode() -> int:
    """TTL Sweeper demo: reserve with short TTL (sidecar reads
    SPENDGUARD_SIDECAR_RESERVATION_TTL_SECONDS=5), sleep ~10s, the
    sweeper background worker auto-releases. verify_step_ttl_sweep.sql
    asserts the release happened.
    """
    from spendguard import (
        SpendGuardClient,
        derive_idempotency_key,
        new_uuid7,
    )
    from spendguard._proto.spendguard.common.v1 import common_pb2

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
        f"[demo] reserved decision_id={outcome.decision_id} "
        f"reservation_ids={list(outcome.reservation_ids)} (TTL=5s)"
    )
    await client.close()

    # Wait for TTL to expire + sweeper poll cycle to fire.
    # TTL=5s, sweeper polls every 2s, total wait 12s gives ~3 sweep cycles.
    print("[demo] waiting 12s for TTL expiry + sweeper poll cycles...")
    await asyncio.sleep(12)
    print("[demo] verifying TTL release happened via verify_step_ttl_sweep.sql")
    return 0


if __name__ == "__main__":
    sys.exit(asyncio.run(main()))
