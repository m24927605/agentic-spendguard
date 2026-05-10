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
    if DEMO_MODE == "approval":
        return await run_approval_mode()
    if DEMO_MODE == "agent_real":
        return await run_agent_mode(use_real_openai=True)
    if DEMO_MODE == "agent_real_anthropic":
        return await run_agent_mode(use_real_anthropic=True)
    if DEMO_MODE == "agent_real_langchain":
        return await run_langchain_mode()
    if DEMO_MODE == "agent_real_langgraph":
        return await run_langgraph_mode()
    if DEMO_MODE == "multi_provider_usd":
        return await run_multi_provider_usd_mode()
    if DEMO_MODE == "agent_real_openai_agents":
        return await run_openai_agents_mode()
    if DEMO_MODE == "agent_real_agt":
        return await run_agt_composite_mode()
    return await run_agent_mode()


# ---------------------------------------------------------------------------
# Microsoft AGT composite mode (Phase 4 O6 closure):
# Exercises all 3 paths of SpendGuardCompositeEvaluator:
#   (1) AGT-deny:                  tool_name='shell' → AGT denies
#   (2) AGT-allow + SG-allow:      tool_name='web_search', small claim → both allow
#   (3) AGT-allow + SG-deny:       tool_name='web_search', huge claim → SG hard-cap
# ---------------------------------------------------------------------------


async def run_agt_composite_mode() -> int:
    from agent_os.policies import (
        PolicyEvaluator, PolicyDocument, PolicyRule, PolicyCondition,
        PolicyAction, PolicyOperator, PolicyDefaults,
    )
    from spendguard import SpendGuardClient, new_uuid7
    from spendguard.integrations.agt import (
        SpendGuardCompositeEvaluator, RunContext as AgtRunContext, run_context as agt_run_context,
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

    unit = common_pb2.UnitRef(unit_id=unit_id, token_kind="output_token", model_family="gpt-4")
    pricing = common_pb2.PricingFreeze(
        pricing_version=pricing_version,
        price_snapshot_hash=bytes.fromhex(snapshot_hash_hex),
        fx_rate_version=fx,
        unit_conversion_version=unit_conv,
    )

    agt = PolicyEvaluator(policies=[PolicyDocument(
        name="demo-policy", version="1.0",
        defaults=PolicyDefaults(action=PolicyAction.ALLOW),
        rules=[PolicyRule(
            name="deny-dangerous",
            condition=PolicyCondition(field="tool_name", operator=PolicyOperator.IN,
                                      value=["shell", "delete_file"]),
            action=PolicyAction.DENY, priority=100,
        )],
    )])

    # Path-specific composite: each sub-test mints its own composite
    # so the claim_estimator can return different amounts (small vs
    # huge) without state leakage.
    def make_composite(claim_amount_atomic: str):
        def estimator(_payload):
            return [
                common_pb2.BudgetClaim(
                    budget_id=budget_id,
                    unit=unit,
                    amount_atomic=claim_amount_atomic,
                    direction=common_pb2.BudgetClaim.DEBIT,
                    window_instance_id=window_id,
                ),
            ]
        return SpendGuardCompositeEvaluator(
            agt_evaluator=agt,
            spendguard_client=client,
            budget_id=budget_id,
            window_instance_id=window_id,
            unit=unit,
            pricing=pricing,
            claim_estimator=estimator,
        )

    run_id = str(new_uuid7())

    async with agt_run_context(AgtRunContext(run_id=run_id)):
        # Path 1: AGT-deny (never reaches sidecar)
        c1 = make_composite("100")
        r1 = await c1.evaluate({"tool_name": "shell", "tool_call_id": str(new_uuid7())})
        print(f"[demo] (1) AGT-deny: allowed={r1.allowed} reason={r1.reason!r} matched={r1.matched_rule_ids}")
        if r1.allowed or "AGT_DENY" not in r1.reason:
            print("[demo] FATAL: expected AGT_DENY", file=sys.stderr)
            return 7

        # Path 2: AGT-allow + SG-allow (small claim, well below cap)
        c2 = make_composite("100")
        r2 = await c2.evaluate({"tool_name": "web_search", "tool_call_id": str(new_uuid7())})
        print(f"[demo] (2) AGT+SG allow: allowed={r2.allowed} reason={r2.reason!r}")
        if not r2.allowed:
            print(f"[demo] FATAL: expected ALLOW, got {r2.reason}", file=sys.stderr)
            return 7

        # Path 3: AGT-allow + SG-deny (huge claim above hard cap 1B)
        c3 = make_composite("2000000000")
        r3 = await c3.evaluate({"tool_name": "web_search", "tool_call_id": str(new_uuid7())})
        print(f"[demo] (3) AGT-allow+SG-deny: allowed={r3.allowed} reason={r3.reason!r}")
        if r3.allowed or "SPENDGUARD_DENY" not in r3.reason:
            print(f"[demo] FATAL: expected SPENDGUARD_DENY, got {r3.reason}", file=sys.stderr)
            return 7

    print("[demo] AGT composite all 3 paths PASS")
    await client.close()
    return 0


# ---------------------------------------------------------------------------
# OpenAI Agents SDK mode (Phase 4 O5 closure):
# Uses agents.Agent + Runner with a SpendGuardAgentsModel wrapping the
# built-in OpenAIChatCompletionsModel.
# ---------------------------------------------------------------------------


async def run_openai_agents_mode() -> int:
    if not os.environ.get("OPENAI_API_KEY"):
        print("[demo] FATAL: OPENAI_API_KEY required for agent_real_openai_agents", file=sys.stderr)
        return 8

    from agents import Agent, Runner
    from agents.models.openai_chatcompletions import OpenAIChatCompletionsModel
    from openai import AsyncOpenAI

    from spendguard import SpendGuardClient, new_uuid7
    from spendguard.integrations.openai_agents import (
        RunContext as OaiRunContext,
        SpendGuardAgentsModel,
        run_context as oai_run_context,
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
    print("[demo] using real OpenAI gpt-4o-mini via OpenAI Agents SDK")

    unit = common_pb2.UnitRef(
        unit_id=unit_id,
        token_kind="output_token",
        model_family="gpt-4",
    )
    pricing = common_pb2.PricingFreeze(
        pricing_version=pricing_version,
        price_snapshot_hash=bytes.fromhex(snapshot_hash_hex),
        fx_rate_version=fx,
        unit_conversion_version=unit_conv,
    )

    def estimate_claims(_input):
        return [
            common_pb2.BudgetClaim(
                budget_id=budget_id,
                unit=unit,
                amount_atomic="500",
                direction=common_pb2.BudgetClaim.DEBIT,
                window_instance_id=window_id,
            ),
        ]

    inner_model = OpenAIChatCompletionsModel(
        model="gpt-4o-mini",
        openai_client=AsyncOpenAI(),
    )
    guarded = SpendGuardAgentsModel(
        inner=inner_model,
        client=client,
        budget_id=budget_id,
        window_instance_id=window_id,
        unit=unit,
        pricing=pricing,
        claim_estimator=estimate_claims,
    )

    agent = Agent(name="spendguard-demo", instructions="Reply concisely.", model=guarded)

    run_id = str(new_uuid7())
    async with oai_run_context(OaiRunContext(run_id=run_id)):
        result = await Runner.run(agent, "Say hello in three words.")

    output = getattr(result, "final_output", None) or str(result)
    print(f"[demo] openai-agents Runner.run OK output={output!r} run_id={run_id}")
    await client.close()
    return 0


# ---------------------------------------------------------------------------
# Multi-provider USD mode (Phase 4 O4): single USD-denominated budget
# debited by both OpenAI and Anthropic in one session. Proves cross-
# provider netting works end-to-end (real LLM usage → token→USD
# conversion → ledger commit in µUSD).
# ---------------------------------------------------------------------------


async def run_multi_provider_usd_mode() -> int:
    if not os.environ.get("OPENAI_API_KEY") or not os.environ.get("ANTHROPIC_API_KEY"):
        print(
            "[demo] FATAL: multi_provider_usd needs both OPENAI_API_KEY + ANTHROPIC_API_KEY",
            file=sys.stderr,
        )
        return 8

    from openai import AsyncOpenAI
    from anthropic import AsyncAnthropic

    from spendguard import SpendGuardClient, derive_idempotency_key, new_uuid7
    from spendguard.pricing import PricingLookup, USD_MICROS_PER_USD
    from spendguard._proto.spendguard.common.v1 import common_pb2

    socket_path = _env("SPENDGUARD_SIDECAR_UDS")
    tenant_id = _env("SPENDGUARD_TENANT_ID")
    budget_id = _env("SPENDGUARD_BUDGET_ID")
    window_id = _env("SPENDGUARD_WINDOW_INSTANCE_ID")
    pricing_version = _env("SPENDGUARD_PRICING_VERSION")
    fx = _env("SPENDGUARD_FX_RATE_VERSION")
    unit_conv = _env("SPENDGUARD_UNIT_CONVERSION_VERSION")
    snapshot_hash_hex = _env("SPENDGUARD_PRICE_SNAPSHOT_HASH_HEX")
    # USD unit_id mirrors the demo seed (deploy/demo/init/migrations/30_seed_demo_state.sh).
    usd_unit_id = "88888888-8888-4888-8888-888888888888"

    # Pricing table (subset; mirrors deploy/demo/init/pricing/seed.yaml).
    # Production adapters would receive this via the sidecar handshake or
    # control plane API.
    pricing_table = PricingLookup({
        ("openai", "gpt-4o-mini", "input"):  0.15,
        ("openai", "gpt-4o-mini", "output"): 0.60,
        ("anthropic", "claude-haiku-4-5-20251001", "input"):  1.00,
        ("anthropic", "claude-haiku-4-5-20251001", "output"): 5.00,
    })

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

    usd_unit = common_pb2.UnitRef(
        unit_id=usd_unit_id,
        token_kind="usd_micros",
        model_family="n/a",
    )
    pricing = common_pb2.PricingFreeze(
        pricing_version=pricing_version,
        price_snapshot_hash=bytes.fromhex(snapshot_hash_hex),
        fx_rate_version=fx,
        unit_conversion_version=unit_conv,
    )

    async def _gated_provider_call(
        provider: str,
        model: str,
        prompt: str,
        run_id: str,
        step_label: str,
    ) -> tuple[int, int, int]:
        """Reserve → real LLM call → commit. Returns (input_t, output_t, charged_usd_micros)."""
        # Conservative pre-claim: 100 µUSD per call (covers up to ~17K
        # output tokens of gpt-4o-mini @ $0.60/1M, much more than
        # "Say hello in three words." would ever produce).
        projected_micros = 100
        claim = common_pb2.BudgetClaim(
            budget_id=budget_id,
            unit=usd_unit,
            amount_atomic=str(projected_micros),
            direction=common_pb2.BudgetClaim.DEBIT,
            window_instance_id=window_id,
        )
        decision_id = str(new_uuid7())
        llm_call_id = str(new_uuid7())
        step_id = f"{run_id}:{step_label}"
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
            projected_claims=[claim],
            idempotency_key=idempotency_key,
        )

        # Real provider call.
        if provider == "openai":
            oai = AsyncOpenAI()
            resp = await oai.chat.completions.create(
                model=model,
                messages=[{"role": "user", "content": prompt}],
                max_tokens=20,
            )
            in_t = resp.usage.prompt_tokens
            out_t = resp.usage.completion_tokens
        elif provider == "anthropic":
            ant = AsyncAnthropic()
            resp = await ant.messages.create(
                model=model,
                max_tokens=20,
                messages=[{"role": "user", "content": prompt}],
            )
            in_t = resp.usage.input_tokens
            out_t = resp.usage.output_tokens
        else:
            raise ValueError(f"unknown provider {provider}")

        actual_micros = pricing_table.usd_micros_for_call(
            provider=provider, model=model,
            input_tokens=in_t, output_tokens=out_t,
        )

        if outcome.reservation_ids:
            await client.emit_llm_call_post(
                run_id=run_id,
                step_id=step_id,
                llm_call_id=llm_call_id,
                decision_id=outcome.decision_id,
                reservation_id=outcome.reservation_ids[0],
                provider_reported_amount_atomic="",
                estimated_amount_atomic=str(actual_micros),
                unit=usd_unit,
                pricing=pricing,
                provider_event_id=getattr(resp, "id", "") or "",
                outcome="SUCCESS",
            )

        return (in_t, out_t, actual_micros)

    run_id = str(new_uuid7())

    oai_in, oai_out, oai_micros = await _gated_provider_call(
        "openai", "gpt-4o-mini",
        "Say hello in three words.",
        run_id, "openai-call",
    )
    print(
        f"[demo] OpenAI: input={oai_in} output={oai_out} "
        f"charged={oai_micros} µUSD (~${oai_micros / USD_MICROS_PER_USD:.6f})"
    )

    ant_in, ant_out, ant_micros = await _gated_provider_call(
        "anthropic", "claude-haiku-4-5-20251001",
        "Say hello in three words.",
        run_id, "anthropic-call",
    )
    print(
        f"[demo] Anthropic: input={ant_in} output={ant_out} "
        f"charged={ant_micros} µUSD (~${ant_micros / USD_MICROS_PER_USD:.6f})"
    )

    total_micros = oai_micros + ant_micros
    print(
        f"[demo] cross-provider total: {total_micros} µUSD "
        f"(~${total_micros / USD_MICROS_PER_USD:.6f}) "
        f"against single USD budget"
    )

    await client.close()
    return 0


# ---------------------------------------------------------------------------
# LangChain mode (Phase 4 O5): SpendGuardChatModel wraps ChatOpenAI.
# ---------------------------------------------------------------------------


async def run_langchain_mode() -> int:
    if not os.environ.get("OPENAI_API_KEY"):
        print("[demo] FATAL: OPENAI_API_KEY required for agent_real_langchain mode", file=sys.stderr)
        return 8

    from langchain_core.messages import HumanMessage
    from langchain_openai import ChatOpenAI

    from spendguard import SpendGuardClient, new_uuid7
    from spendguard.integrations.langchain import (
        RunContext as LcRunContext,
        SpendGuardChatModel,
        run_context as lc_run_context,
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
    print("[demo] using real OpenAI gpt-4o-mini via LangChain")

    unit = common_pb2.UnitRef(
        unit_id=unit_id,
        token_kind="output_token",
        model_family="gpt-4",
    )
    pricing = common_pb2.PricingFreeze(
        pricing_version=pricing_version,
        price_snapshot_hash=bytes.fromhex(snapshot_hash_hex),
        fx_rate_version=fx,
        unit_conversion_version=unit_conv,
    )

    def estimate_claims(messages):
        # Conservative: reserve 500 atomic per call (well above gpt-4o-mini's
        # ~30 token responses for short prompts; below the 1B contract cap).
        return [
            common_pb2.BudgetClaim(
                budget_id=budget_id,
                unit=unit,
                amount_atomic="500",
                direction=common_pb2.BudgetClaim.DEBIT,
                window_instance_id=window_id,
            ),
        ]

    guarded = SpendGuardChatModel(
        inner=ChatOpenAI(model="gpt-4o-mini"),
        client=client,
        budget_id=budget_id,
        window_instance_id=window_id,
        unit=unit,
        pricing=pricing,
        claim_estimator=estimate_claims,
    )

    run_id = str(new_uuid7())
    async with lc_run_context(LcRunContext(run_id=run_id)):
        result = await guarded.ainvoke([HumanMessage(content="Say hello in three words.")])

    output = getattr(result, "content", None) or str(result)
    print(f"[demo] langchain ainvoke OK output={output!r} run_id={run_id}")
    await client.close()
    return 0


# ---------------------------------------------------------------------------
# LangGraph mode (Phase 4 O5): same SpendGuardChatModel, driven through
# a LangGraph create_react_agent. Reservation lifecycle is identical to
# bare LangChain — LangGraph just orchestrates the model + tools graph.
# ---------------------------------------------------------------------------


async def run_langgraph_mode() -> int:
    if not os.environ.get("OPENAI_API_KEY"):
        print("[demo] FATAL: OPENAI_API_KEY required for agent_real_langgraph mode", file=sys.stderr)
        return 8

    from langchain_core.tools import tool
    from langchain_openai import ChatOpenAI
    from langgraph.prebuilt import create_react_agent

    from spendguard import SpendGuardClient, new_uuid7
    from spendguard.integrations.langchain import (
        RunContext as LcRunContext,
        SpendGuardChatModel,
        run_context as lc_run_context,
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
    print("[demo] using real OpenAI gpt-4o-mini via LangGraph")

    unit = common_pb2.UnitRef(
        unit_id=unit_id,
        token_kind="output_token",
        model_family="gpt-4",
    )
    pricing = common_pb2.PricingFreeze(
        pricing_version=pricing_version,
        price_snapshot_hash=bytes.fromhex(snapshot_hash_hex),
        fx_rate_version=fx,
        unit_conversion_version=unit_conv,
    )

    def estimate_claims(messages):
        return [
            common_pb2.BudgetClaim(
                budget_id=budget_id,
                unit=unit,
                amount_atomic="500",
                direction=common_pb2.BudgetClaim.DEBIT,
                window_instance_id=window_id,
            ),
        ]

    guarded = SpendGuardChatModel(
        inner=ChatOpenAI(model="gpt-4o-mini"),
        client=client,
        budget_id=budget_id,
        window_instance_id=window_id,
        unit=unit,
        pricing=pricing,
        claim_estimator=estimate_claims,
    )

    @tool
    def echo(msg: str) -> str:
        """Echo the input string back."""
        return msg

    agent = create_react_agent(guarded, tools=[echo])

    run_id = str(new_uuid7())
    async with lc_run_context(LcRunContext(run_id=run_id)):
        result = await agent.ainvoke(
            {"messages": [{"role": "user", "content": "Say hello in three words."}]}
        )

    final_msg = result["messages"][-1] if result.get("messages") else None
    output = getattr(final_msg, "content", None) if final_msg is not None else None
    print(f"[demo] langgraph ainvoke OK output={output!r} run_id={run_id}")
    await client.close()
    return 0


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


# ---------------------------------------------------------------------------
# DEMO_MODE=approval (Round-2 #9 part 2 PR 9e):
# Exercises the REQUIRE_APPROVAL → ApprovalRequired → e.resume(client) flow
# end-to-end. Drives the resume gRPC RPCs added in PR #37 (ledger handlers)
# + PR #38 (sidecar wiring) + PR #39 (Python SDK).
#
# Today the resume path surfaces ApprovalLapsedError with the
# [PRODUCER_SP_NOT_WIRED] tag because the producer-side
# post_approval_required_decision SP that writes
# approval_requests.{decision_context, requested_effect} JSON is a
# separate workstream. Once that SP lands the demo will assert a
# successful Continue with a fresh ledger transaction.
# ---------------------------------------------------------------------------


async def run_approval_mode() -> int:
    from spendguard import (
        ApprovalDeniedError,
        ApprovalLapsedError,
        ApprovalRequired,
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

    print(f"[demo] approval-mode connecting to sidecar at {socket_path}")
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

    # Claim 500_000_000 atomic ($500) — assumes the demo contract has
    # a REQUIRE_APPROVAL rule keyed on amount > $500. If the seeded
    # bundle doesn't yet ship such a rule, this DECISION returns
    # CONTINUE and we report that explicitly so the operator can
    # follow up with the bundle update.
    claims = [
        common_pb2.BudgetClaim(
            budget_id=budget_id,
            unit=common_pb2.UnitRef(
                unit_id=unit_id,
                token_kind="output_token",
                model_family="gpt-4",
            ),
            amount_atomic="500000000",
            direction=common_pb2.BudgetClaim.DEBIT,
            window_instance_id=window_id,
        ),
    ]
    idempotency_key = derive_idempotency_key(
        tenant_id=tenant_id,
        session_id=client.session_id,
        run_id=run_id,
        step_id=step_id,
        llm_call_id=llm_call_id,
        trigger="LLM_CALL_PRE",
    )

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
        print(
            f"[demo] DEMO_MODE=approval — decision returned CONTINUE without "
            f"REQUIRE_APPROVAL (decision_id={outcome.decision_id}). The seeded "
            f"contract bundle does not yet contain a REQUIRE_APPROVAL rule for "
            f"the demo claim shape. The resume flow surface (sidecar + SDK) is "
            f"still wired and exercised individually by unit tests in PR #37/#38/#39."
        )
        await client.close()
        return 0
    except ApprovalRequired as e:
        print(
            f"[demo] REQUIRE_APPROVAL raised approval_id={e.approval_request_id} "
            f"decision_id={e.decision_id}"
        )

        # Round-2 #9 part 2: in production the approver simulates a
        # decision via the control_plane REST API
        # (POST /v1/approvals/{id}/resolve). Here we go straight to
        # resume() and expect either:
        #   * ApprovalLapsedError(state='pending') — operator hasn't
        #     resolved yet (typical demo path until control_plane
        #     wiring is exercised)
        #   * ApprovalLapsedError(message containing
        #     'PRODUCER_SP_NOT_WIRED') — operator approved but the
        #     producer-side SP that captures decision_context +
        #     requested_effect hasn't shipped, so resume can't rebuild
        #     the ReserveSetRequest
        #   * Continue DecisionOutcome — full path lit up
        #   * ApprovalDeniedError — operator rejected
        try:
            resume_outcome = await e.resume(client)
            print(
                f"[demo] resume() returned CONTINUE: "
                f"decision_id={resume_outcome.decision_id} "
                f"ledger_transaction_id={resume_outcome.ledger_transaction_id}"
            )
        except ApprovalLapsedError as lapsed:
            print(
                f"[demo] resume() raised ApprovalLapsedError state={lapsed.state} "
                f"message={lapsed!s} — expected until producer-side SP lands"
            )
        except ApprovalDeniedError as denied:
            print(
                f"[demo] resume() raised ApprovalDeniedError "
                f"approver={denied.approver_subject} reason={denied.approver_reason}"
            )
        await client.close()
        return 0


if __name__ == "__main__":
    sys.exit(asyncio.run(main()))
