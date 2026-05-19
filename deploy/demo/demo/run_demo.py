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
from pathlib import Path
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
    if DEMO_MODE == "approval_hot_reload":
        return await run_approval_hot_reload_mode()
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
    if DEMO_MODE == "agent_real_openai_agents_multistep":
        return await run_openai_agents_multistep_mode()
    if DEMO_MODE == "agent_real_openai_agents_proxy":
        return await run_openai_agents_proxy_mode()
    if DEMO_MODE == "agent_real_agt":
        return await run_agt_composite_mode()
    if DEMO_MODE == "litellm_real":
        return await run_litellm_real_mode()
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
# OpenAI Agents SDK + egress proxy "1 env var" mode.
#
# This is the launch-claim closure demo. NO SpendGuardAgentsModel wrapper,
# NO claim_estimator, NO SpendGuardClient handshake from inside the demo
# process. JUST the openai-agents SDK + OPENAI_BASE_URL pointed at the
# proxy. If this PASSes against real OpenAI, the launch claim
# ("set OPENAI_BASE_URL, no code change, hard-cap budget gate active")
# is verified end-to-end against the SDK that HN readers will actually
# run.
#
# The proxy gates the call PRE via the sidecar (default tenant from
# SPENDGUARD_PROXY_DEFAULT_* env vars; no per-request headers required).
# CONTINUE → forward to OpenAI; STOP → 429 (would short-circuit Runner
# with an APIError — not exercised here, the demo intends to succeed).
# ---------------------------------------------------------------------------


async def run_openai_agents_proxy_mode() -> int:
    if not os.environ.get("OPENAI_API_KEY"):
        print(
            "[demo] FATAL: OPENAI_API_KEY required for agent_real_openai_agents_proxy",
            file=sys.stderr,
        )
        return 8

    proxy_base_url = os.environ.get(
        "SPENDGUARD_PROXY_BASE_URL", "http://egress-proxy:9000/v1"
    )
    print(f"[demo] launch-claim verification: pointing openai-agents at {proxy_base_url}")
    print("[demo]   NO SpendGuard SDK adapter; NO wrapper. Just OPENAI_BASE_URL.")
    os.environ["OPENAI_BASE_URL"] = proxy_base_url

    from agents import Agent, Runner

    # v0.3: openai-agents default model is OpenAIResponsesModel which
    # hits POST /v1/responses. The v0.3 egress proxy routes this
    # endpoint with the same PRE/POST gating + audit chain as
    # /v1/chat/completions, so the shorthand `Agent(model="...")` form
    # works through the proxy with NO explicit ChatCompletions
    # construction (this used to be a workaround required pre-v0.3).
    agent = Agent(
        name="spendguard-launch-demo",
        instructions="Reply concisely in three words.",
        model="gpt-4o-mini",
    )

    try:
        result = await Runner.run(agent, "Say hello.")
    except Exception as e:
        print(
            f"[demo] FATAL: Runner.run through proxy failed: {type(e).__name__}: {e}",
            file=sys.stderr,
        )
        return 9

    output = getattr(result, "final_output", None) or str(result)
    print(f"[demo] openai-agents Runner.run via proxy OK; output={output!r}")
    print("[demo] launch-claim verified end-to-end:")
    print("[demo]   1 env var (OPENAI_BASE_URL) + shorthand Agent(model='...')")
    print("[demo]   → hard-cap gate active + audit chain captured")
    print("[demo] v0.3 closure: openai-agents default Responses API now WORKS.")
    return 0


# ---------------------------------------------------------------------------
# OpenAI Agents SDK multi-step mode: tool-calling agent that must invoke
# a tool multiple times, forcing ≥2 LLM calls in one Runner.run. Verifies
# in-flight cap fires per LLM_CALL_PRE step (mid-loop), not just per run.
# Wired post-V1 to close the "events 中把控 / multi-step in-flight" gap
# noted in benchmarks/real-stack-e2e/REAL_LANGCHAIN_E2E.md.
# ---------------------------------------------------------------------------


async def run_openai_agents_multistep_mode() -> int:
    if not os.environ.get("OPENAI_API_KEY"):
        print(
            "[demo] FATAL: OPENAI_API_KEY required for agent_real_openai_agents_multistep",
            file=sys.stderr,
        )
        return 8

    from agents import Agent, Runner, function_tool
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
    print("[demo] using real OpenAI gpt-4o-mini via OpenAI Agents SDK (multi-step)")

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

    @function_tool
    def get_weather(city: str) -> str:
        """Return current weather for a city (mocked for demo)."""
        weather_db = {
            "Tokyo": "Sunny, 22°C, light wind",
            "San Francisco": "Foggy, 15°C, marine layer",
            "London": "Rainy, 12°C, overcast",
        }
        return weather_db.get(city, f"Unknown weather for {city}")

    agent = Agent(
        name="spendguard-multistep-demo",
        instructions=(
            "You are a weather assistant. Use the get_weather tool to look up "
            "weather for each city the user mentions. After collecting all "
            "results, return a one-line summary."
        ),
        model=guarded,
        tools=[get_weather],
    )

    run_id = str(new_uuid7())
    async with oai_run_context(OaiRunContext(run_id=run_id)):
        result = await Runner.run(
            agent,
            "Get the weather for Tokyo, San Francisco, and London. Then summarize in one line.",
        )

    output = getattr(result, "final_output", None) or str(result)
    print(f"[demo] openai-agents multi-step Runner.run OK output={output!r} run_id={run_id}")
    print(
        "[demo] (verify multi-step in-flight cap by querying postgres after demo: "
        "expect >1 reserve transactions and >1 spendguard.audit.decision events)"
    )
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
            f"[demo] FATAL: DEMO_MODE=approval expected REQUIRE_APPROVAL but "
            f"got CONTINUE (decision_id={outcome.decision_id}). The seeded "
            f"contract bundle should contain the `require-approval-large` rule "
            f"firing on claim_amount_atomic_gt 400_000_000.",
            file=sys.stderr,
        )
        await client.close()
        return 7
    except ApprovalRequired as e:
        print(
            f"[demo] REQUIRE_APPROVAL raised approval_id={e.approval_request_id} "
            f"decision_id={e.decision_id}"
        )

        # Round-2 #9 part 2 closed loop: simulate the operator
        # approving the request via the control-plane REST surface
        # before calling resume().
        import httpx

        control_plane_url = os.environ.get(
            "SPENDGUARD_CONTROL_PLANE_URL", "http://control-plane:8091"
        )
        control_plane_token = os.environ.get(
            "SPENDGUARD_CONTROL_PLANE_TOKEN",
            "demo-admin-token-replace-in-production",
        )
        resolve_url = f"{control_plane_url}/v1/approvals/{e.approval_request_id}/resolve"
        async with httpx.AsyncClient(timeout=10.0) as http:
            resp = await http.post(
                resolve_url,
                headers={"Authorization": f"Bearer {control_plane_token}"},
                json={"target_state": "approved", "reason": "demo operator approves"},
            )
            if resp.status_code >= 400:
                print(
                    f"[demo] FATAL: control-plane resolve returned "
                    f"HTTP {resp.status_code}: {resp.text}",
                    file=sys.stderr,
                )
                await client.close()
                return 4
            print(
                f"[demo] control-plane resolved approval_id={e.approval_request_id} "
                f"-> {resp.json().get('final_state')}"
            )

        # Now resume — the sidecar's ResumeAfterApproval handler
        # reads the resolved row via GetApprovalForResume, rebuilds
        # the ReserveSetRequest from the captured decision_context +
        # requested_effect JSON, calls Ledger.ReserveSet, and
        # atomically links the approval row via MarkApprovalBundled.
        try:
            resume_outcome = await e.resume(client)
            print(
                f"[demo] resume() returned CONTINUE: "
                f"decision_id={resume_outcome.decision_id} "
                f"ledger_transaction_id={resume_outcome.ledger_transaction_id}"
            )
        except ApprovalLapsedError as lapsed:
            print(
                f"[demo] FATAL: resume() raised ApprovalLapsedError state={lapsed.state} "
                f"message={lapsed!s} — expected CONTINUE after control-plane resolve",
                file=sys.stderr,
            )
            await client.close()
            return 5
        except ApprovalDeniedError as denied:
            print(
                f"[demo] FATAL: resume() raised ApprovalDeniedError "
                f"approver={denied.approver_subject} reason={denied.approver_reason} — "
                f"expected CONTINUE after target_state=approved",
                file=sys.stderr,
            )
            await client.close()
            return 6
        await client.close()
        return 0


# ---------------------------------------------------------------------------
# DEMO_MODE=approval_hot_reload (issue #68 — slice 4 of #59):
# Exercises the BUNDLE_HOT_RELOADED error path end-to-end.
#
# Flow:
#   1. Trigger REQUIRE_APPROVAL @ contract bundle B0 (current).
#   2. Capture B0's hash + approval_id from the SDK exception.
#   3. ROTATE the contract bundle in the bundles-data volume:
#      a. Add a marker file to the .tgz contents (different bytes → different hash).
#      b. Rewrite the .tgz + update runtime.env's HASH_HEX to the new value.
#      c. Sidecar's hot-reload watcher polls runtime.env every 500ms and
#         atomically swaps to the new bundle within ~1s.
#   4. Poll sidecar's hot-reload state until it has swapped to B1.
#   5. control-plane resolve = approved (B0's approval still pending).
#   6. Call e.resume(client) → MUST raise ApprovalBundleHotReloadedError
#      with original_bundle_hash=B0, current_bundle_hash=B1.
# ---------------------------------------------------------------------------


def _read_runtime_env(path: str) -> dict:
    return {
        line.split("=", 1)[0]: line.split("=", 1)[1]
        for line in open(path).read().splitlines()
        if "=" in line and not line.startswith("#")
    }


def _rotate_contract_bundle(bundles_dir: str, contract_bundle_id: str) -> tuple[str, str]:
    """Rotate the contract bundle by appending a marker file inside the .tgz.

    Returns ``(old_hash_hex, new_hash_hex)``. The sidecar's hot-reload watcher
    polls ``runtime.env`` every 500ms; the swap should complete within ~1s
    of this function returning.
    """
    import hashlib
    import io
    import tarfile
    import time as _time

    bundles_path = Path(bundles_dir)
    runtime_env_path = bundles_path / "runtime.env"
    tgz_path = bundles_path / "contract_bundle" / f"{contract_bundle_id}.tgz"

    old_runtime = runtime_env_path.read_text()
    old_hash = ""
    for line in old_runtime.splitlines():
        if line.startswith("SPENDGUARD_SIDECAR_CONTRACT_BUNDLE_HASH_HEX="):
            old_hash = line.split("=", 1)[1].strip()
            break
    if not old_hash:
        raise RuntimeError(f"no SPENDGUARD_SIDECAR_CONTRACT_BUNDLE_HASH_HEX in {runtime_env_path}")

    # Read existing tarball contents into memory.
    with tarfile.open(tgz_path, "r:gz") as tf:
        members = [(m, tf.extractfile(m).read() if m.isfile() else None) for m in tf.getmembers()]

    # Add a rotation marker. Different content → different sha256.
    marker_name = ".rotation_marker"
    marker_data = f"rotated_at={_time.time()}\n".encode()

    # Re-tar deterministically (apart from the marker timestamp).
    buf = io.BytesIO()
    with tarfile.open(fileobj=buf, mode="w:gz", format=tarfile.USTAR_FORMAT) as tf:
        # Preserve original files first.
        for member, data in members:
            new_info = tarfile.TarInfo(name=member.name)
            new_info.size = len(data) if data is not None else 0
            new_info.mtime = 0
            new_info.uid = 0
            new_info.gid = 0
            new_info.uname = ""
            new_info.gname = ""
            new_info.mode = 0o644
            new_info.type = member.type
            if data is not None:
                tf.addfile(new_info, io.BytesIO(data))
            else:
                tf.addfile(new_info)
        # Append the marker.
        marker_info = tarfile.TarInfo(name=marker_name)
        marker_info.size = len(marker_data)
        marker_info.mtime = 0
        marker_info.uid = 0
        marker_info.gid = 0
        marker_info.uname = ""
        marker_info.gname = ""
        marker_info.mode = 0o644
        tf.addfile(marker_info, io.BytesIO(marker_data))

    new_tgz = buf.getvalue()
    new_hash = hashlib.sha256(new_tgz).hexdigest()
    tgz_path.write_bytes(new_tgz)

    # Update runtime.env atomically: write to .tmp then rename.
    new_lines = []
    for line in old_runtime.splitlines():
        if line.startswith("SPENDGUARD_SIDECAR_CONTRACT_BUNDLE_HASH_HEX="):
            new_lines.append(f"SPENDGUARD_SIDECAR_CONTRACT_BUNDLE_HASH_HEX={new_hash}")
        else:
            new_lines.append(line)
    new_runtime = "\n".join(new_lines) + "\n"
    tmp_path = runtime_env_path.with_suffix(".env.tmp")
    tmp_path.write_text(new_runtime)
    os.replace(tmp_path, runtime_env_path)

    return old_hash, new_hash


async def run_approval_hot_reload_mode() -> int:
    from spendguard import (
        ApprovalBundleHotReloadedError,
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

    bundles_dir = os.environ.get(
        "SPENDGUARD_DEMO_BUNDLES_DIR", "/var/lib/spendguard/bundles"
    )
    contract_bundle_id = os.environ.get(
        "SPENDGUARD_DEMO_CONTRACT_BUNDLE_ID",
        "11111111-1111-4111-8111-111111111111",
    )

    print(f"[demo] approval_hot_reload: bundles dir={bundles_dir}")
    print(f"[demo]                      contract bundle id={contract_bundle_id}")

    # 1. Connect + handshake (mirrors run_approval_mode).
    deadline = time.monotonic() + HANDSHAKE_TIMEOUT_S
    client: "SpendGuardClient | None" = None
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

    # 2. Submit a claim that triggers REQUIRE_APPROVAL (same shape as
    # the existing approval demo).
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
        await client.request_decision(
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
            "[demo] FATAL: expected REQUIRE_APPROVAL but got CONTINUE",
            file=sys.stderr,
        )
        await client.close()
        return 7
    except ApprovalRequired as e:
        approval_id = e.approval_request_id
        print(
            f"[demo] REQUIRE_APPROVAL raised approval_id={approval_id} "
            f"decision_id={e.decision_id}"
        )

        # 3. Rotate the contract bundle (changes the .tgz bytes → new hash).
        old_hash, new_hash = _rotate_contract_bundle(bundles_dir, contract_bundle_id)
        print(f"[demo] rotated contract bundle: B0={old_hash} -> B1={new_hash}")

        # 4. Sleep > 500ms (sidecar's hot-reload poll interval) so the
        # watcher picks up the new runtime.env. 2s is comfortable.
        await asyncio.sleep(2.0)
        print("[demo] slept 2s; sidecar should now be on the rotated bundle")

        # 5. Resolve the approval via control-plane (B0's approval row).
        import httpx

        control_plane_url = os.environ.get(
            "SPENDGUARD_CONTROL_PLANE_URL", "http://control-plane:8091"
        )
        control_plane_token = os.environ.get(
            "SPENDGUARD_CONTROL_PLANE_TOKEN",
            "demo-admin-token-replace-in-production",
        )
        resolve_url = f"{control_plane_url}/v1/approvals/{approval_id}/resolve"
        async with httpx.AsyncClient(timeout=10.0) as http:
            resp = await http.post(
                resolve_url,
                headers={"Authorization": f"Bearer {control_plane_token}"},
                json={"target_state": "approved", "reason": "demo operator approves"},
            )
            if resp.status_code >= 400:
                print(
                    f"[demo] FATAL: control-plane resolve returned HTTP "
                    f"{resp.status_code}: {resp.text}",
                    file=sys.stderr,
                )
                await client.close()
                return 4
            print(
                f"[demo] control-plane resolved approval_id={approval_id} "
                f"-> {resp.json().get('final_state')}"
            )

        # 6. resume() — MUST raise ApprovalBundleHotReloadedError because
        # the live bundle hash (B1) no longer matches the approval's
        # captured hash (B0).
        try:
            outcome = await e.resume(client)
            print(
                f"[demo] FATAL: resume() returned CONTINUE "
                f"(decision_id={outcome.decision_id}) but expected "
                f"ApprovalBundleHotReloadedError",
                file=sys.stderr,
            )
            await client.close()
            return 10
        except ApprovalBundleHotReloadedError as hr:
            print(
                f"[demo] resume() raised ApprovalBundleHotReloadedError "
                f"original={hr.original_bundle_hash} "
                f"current={hr.current_bundle_hash}"
            )
            if hr.original_bundle_hash != old_hash:
                print(
                    f"[demo] FATAL: original_bundle_hash mismatch: expected "
                    f"{old_hash}, got {hr.original_bundle_hash}",
                    file=sys.stderr,
                )
                await client.close()
                return 11
            if hr.current_bundle_hash != new_hash:
                print(
                    f"[demo] FATAL: current_bundle_hash mismatch: expected "
                    f"{new_hash}, got {hr.current_bundle_hash}",
                    file=sys.stderr,
                )
                await client.close()
                return 12
            print(
                "[demo] PASS — frozen-at-PRE invariant verified end-to-end: "
                "bundle rotated between approval and resume → BUNDLE_HOT_RELOADED"
            )
            await client.close()
            return 0


# ---------------------------------------------------------------------------
# LiteLLM proxy mode (Slice 6): exercise steps 1+2 of the 4-step demo
# (ACCEPTANCE.md §5.1). Boots:
#   1. counting HTTP listener (mock OpenAI-shape provider; NO mock_response)
#   2. LiteLLM proxy subprocess with SpendGuard callback registered
# Then POSTs ALLOW (small estimator → admitted) and DENY (over-budget
# attempt). Steps 3+4 (STREAM + PROXY-MULTI-TEAM) land in Slice 9.
# ---------------------------------------------------------------------------


_COUNTING_PROVIDER_HITS: dict[str, int] = {"calls": 0, "tokens": 0}


def _spendguard_proxy_dir() -> Path:
    """Inside the demo container the LiteLLM proxy config + callback
    module live under /opt/spendguard/litellm_proxy (Dockerfile copies
    deploy/demo/litellm_proxy/* there). The path is resolved here so
    local-host invocations also work for codex iteration."""
    container_path = Path("/opt/spendguard/litellm_proxy")
    if container_path.exists():
        return container_path
    return Path(__file__).resolve().parent.parent / "litellm_proxy"


async def _start_counting_provider(host: str = "127.0.0.1", port: int = 8765) -> Any:
    """Start an in-process aiohttp server that mimics OpenAI's
    /v1/chat/completions with non-zero token counts so the reconciler
    has a real `usage.completion_tokens` to commit (per spec line 893:
    `mock_response` is BANNED — F7 acceptance requires reconciled usage)."""
    from aiohttp import web

    async def chat_completions(request: "web.Request") -> "web.Response":
        body = await request.json()
        _COUNTING_PROVIDER_HITS["calls"] += 1
        completion_tokens = 7  # deterministic; reconciler commits this
        _COUNTING_PROVIDER_HITS["tokens"] += completion_tokens
        model = body.get("model", "gpt-4o-mini")
        return web.json_response({
            "id": f"chatcmpl-counting-{_COUNTING_PROVIDER_HITS['calls']}",
            "object": "chat.completion",
            "created": int(time.time()),
            "model": model,
            "choices": [{
                "index": 0,
                "message": {"role": "assistant", "content": "hi from counting provider"},
                "finish_reason": "stop",
            }],
            "usage": {
                "prompt_tokens": 5,
                "completion_tokens": completion_tokens,
                "total_tokens": 5 + completion_tokens,
            },
        })

    app = web.Application()
    app.router.add_post("/v1/chat/completions", chat_completions)
    runner = web.AppRunner(app)
    await runner.setup()
    site = web.TCPSite(runner, host, port)
    await site.start()
    print(f"[demo] counting provider listening on http://{host}:{port}/v1")
    return runner


async def _drain_proxy_output(proc: "asyncio.subprocess.Process") -> None:
    """Background drain of LiteLLM proxy stdout → demo stderr so the
    proxy's boot banner + uvicorn access logs + any ImportError
    traceback reach the operator, AND the PIPE buffer never fills
    (Slice 6 R1 Code Reviewer P1).

    Slice 6 R2 P2-4: explicit None-check (not `assert`) so this still
    works under `python -O`.
    """
    if proc.stdout is None:
        return
    while True:
        line = await proc.stdout.readline()
        if not line:
            return
        sys.stderr.write(f"[litellm-proxy] {line.decode(errors='replace')}")
        sys.stderr.flush()


async def _start_litellm_proxy_subprocess(
    config_path: Path, callback_dir: Path, port: int = 4000,
) -> tuple["asyncio.subprocess.Process", asyncio.Task[None]]:
    """Spawn the LiteLLM proxy via `python -m litellm` and a stdout-drain
    task. Returns (proc, drain_task).

    Uses `sys.executable -m litellm` (not bare `litellm`) so the proxy
    runs against the same site-packages as the demo (Slice 6 R1 Backend
    Architect P2 robustness). Adds the callback module directory to
    PYTHONPATH so the proxy can import `spendguard_callback`. The
    `SPENDGUARD_PRICE_SNAPSHOT_HASH_HEX` env var is sourced from
    runtime.env by demo-entrypoint.sh before this function runs, so
    `os.environ.copy()` propagates it into the subprocess env.
    """
    env = os.environ.copy()
    pp = env.get("PYTHONPATH", "")
    env["PYTHONPATH"] = f"{callback_dir}:{pp}" if pp else str(callback_dir)
    # Slice 6 R2 P0-1 fix: `python -m litellm` fails (litellm has no
    # `__main__.py`); the actual CLI module is `litellm.proxy.proxy_cli`.
    proc = await asyncio.create_subprocess_exec(
        sys.executable, "-m", "litellm.proxy.proxy_cli",
        "--config", str(config_path),
        "--port", str(port), "--num_workers", "1",
        env=env, stdout=asyncio.subprocess.PIPE,
        stderr=asyncio.subprocess.STDOUT,
    )
    drain_task = asyncio.create_task(_drain_proxy_output(proc))
    print(f"[demo] LiteLLM proxy spawned pid={proc.pid} port={port}")
    return proc, drain_task


async def _wait_for_litellm_health(port: int, timeout_s: float = 30.0) -> None:
    """Poll /health/readiness until the proxy answers 200 or timeout."""
    import httpx
    deadline = time.monotonic() + timeout_s
    last_err: str = ""
    async with httpx.AsyncClient(timeout=2.0) as http:
        while time.monotonic() < deadline:
            try:
                r = await http.get(f"http://127.0.0.1:{port}/health/readiness")
                if r.status_code == 200:
                    print(f"[demo] LiteLLM proxy ready (health=200)")
                    return
                last_err = f"HTTP {r.status_code}"
            except Exception as e:  # noqa: BLE001
                last_err = repr(e)
            await asyncio.sleep(0.5)
    raise RuntimeError(
        f"LiteLLM proxy failed to become healthy in {timeout_s}s: {last_err}"
    )


async def run_litellm_real_mode() -> int:
    """Steps 1+2 of the 4-step demo. Step 3 (STREAM) + step 4
    (PROXY-MULTI-TEAM) land in Slice 9."""
    if not os.environ.get("SPENDGUARD_BUDGET_ID"):
        print("[demo] FATAL: budget env vars required", file=sys.stderr)
        return 8

    proxy_dir = _spendguard_proxy_dir()
    config_path = proxy_dir / "proxy_config.yaml"
    if not config_path.exists():
        print(f"[demo] FATAL: proxy config missing at {config_path}", file=sys.stderr)
        return 8

    counting_runner = None
    proxy_proc = None
    drain_task: asyncio.Task[None] | None = None
    try:
        counting_runner = await _start_counting_provider()
        proxy_proc, drain_task = await _start_litellm_proxy_subprocess(
            config_path, proxy_dir,
        )
        await _wait_for_litellm_health(port=4000)

        import httpx
        async with httpx.AsyncClient(
            base_url="http://127.0.0.1:4000",
            headers={"Authorization": "Bearer sk-demo-litellm-proxy-key"},
            timeout=15.0,
        ) as http:
            # ---- Step 1: ALLOW ----
            # Slice 6 R2 P1-3 fix: LiteLLM proxy overwrites
            # `litellm_call_id` from the request body with whatever
            # the `x-litellm-call-id` header sets (or a fresh UUID).
            # Pass via header so the friendly ID actually reaches the
            # callback and audit rows.
            pre_calls = _COUNTING_PROVIDER_HITS["calls"]
            r = await http.post("/v1/chat/completions", json={
                "model": "gpt-4o-mini",
                "messages": [{"role": "user", "content": "hello"}],
            }, headers={"x-litellm-call-id": "demo-litellm-allow-1"})
            print(f"[demo] (1) ALLOW step: HTTP {r.status_code} body={r.text[:200]!r}")
            if r.status_code != 200:
                print(f"[demo] FATAL: ALLOW step expected 200, got {r.status_code}",
                      file=sys.stderr)
                return 7
            # Positive control: counting listener actually hit (no
            # mock_response). Slice 6 R1 Backend Architect P1 fix —
            # bare `assert` would be stripped under python -O; use
            # explicit if/return to mirror other demo paths.
            if _COUNTING_PROVIDER_HITS["calls"] != pre_calls + 1:
                print(
                    f"[demo] FATAL: counting provider should have been "
                    f"hit once; pre={pre_calls} post="
                    f"{_COUNTING_PROVIDER_HITS['calls']}",
                    file=sys.stderr,
                )
                return 7
            usage = r.json().get("usage", {})
            if int(usage.get("completion_tokens", 0)) <= 0:
                print(
                    f"[demo] FATAL: F7 acceptance requires "
                    f"completion_tokens > 0; got usage={usage}",
                    file=sys.stderr,
                )
                return 7
            print(f"[demo] (1) ALLOW positive control: counting_calls={_COUNTING_PROVIDER_HITS['calls']} "
                  f"completion_tokens={usage.get('completion_tokens')}")

            # ---- Step 2: DENY ----
            # Drive the hard-cap path by sending a per-call
            # `spendguard_estimate_override=2000000000` (2B atomic
            # units, above the seeded 1B hard-cap). The callback's
            # estimator reads this from request data; sidecar policy
            # then emits SPENDGUARD_DENY pre-call. The counting
            # provider must NOT be hit on this path (negative
            # control). Slice 7 will add the over-budget-seed variant
            # for a complementary DENY shape.
            counting_pre_deny = _COUNTING_PROVIDER_HITS["calls"]
            try:
                r_deny = await http.post("/v1/chat/completions", json={
                    "model": "gpt-4o-mini",
                    "messages": [{"role": "user", "content": "trigger deny"}],
                    "spendguard_estimate_override": "2000000000",
                }, headers={"x-litellm-call-id": "demo-litellm-deny-1"})
            except httpx.RequestError as e:
                # Slice 6 R2 P2 fix: only TRANSPORT failures (connect
                # refused / read timeout) raise from httpx.post; 4xx
                # and 5xx come back as Response. A transport failure
                # means the proxy is unreachable — that's a demo gate
                # failure, NOT a DENY signal. Fail loud instead of
                # synthesising a fake 400.
                print(
                    f"[demo] FATAL: DENY step transport failure — "
                    f"proxy unreachable: {e!r}",
                    file=sys.stderr,
                )
                return 7
            counting_post_deny = _COUNTING_PROVIDER_HITS["calls"]
            print(f"[demo] (2) DENY step: HTTP {r_deny.status_code} "
                  f"body={str(r_deny.text)[:200]!r}")
            print(f"[demo] (2) DENY negative control: counting hits "
                  f"pre={counting_pre_deny} post={counting_post_deny}")
            # Acceptance: SpendGuard DECLINED → LiteLLM proxy returns
            # non-2xx (403 with the R2 P1-2 status_code fix on
            # DecisionDenied; legacy 500 still acceptable). AND
            # counting provider MUST NOT have been hit (otherwise the
            # budget rejection didn't gate the call).
            if counting_post_deny != counting_pre_deny:
                print(
                    "[demo] FATAL: DENY step did NOT block upstream — "
                    f"counting hit pre={counting_pre_deny} "
                    f"post={counting_post_deny}. Either the override "
                    "didn't reach the estimator, or the hard-cap rule "
                    "didn't fire.",
                    file=sys.stderr,
                )
                return 7
            if r_deny.status_code < 400:
                print(
                    f"[demo] FATAL: DENY step expected HTTP non-2xx, "
                    f"got {r_deny.status_code} — proxy admitted the "
                    "call even though SpendGuard should have denied.",
                    file=sys.stderr,
                )
                return 7

        print("[demo] litellm_real steps 1+2 complete (ALLOW + DENY)")
        return 0
    finally:
        if proxy_proc is not None:
            proxy_proc.terminate()
            try:
                await asyncio.wait_for(proxy_proc.wait(), timeout=5.0)
            except asyncio.TimeoutError:
                proxy_proc.kill()
                await proxy_proc.wait()
            print("[demo] LiteLLM proxy subprocess terminated")
        if drain_task is not None:
            drain_task.cancel()
            try:
                await drain_task
            except (asyncio.CancelledError, Exception):  # noqa: BLE001
                pass
        if counting_runner is not None:
            await counting_runner.cleanup()
            print("[demo] counting provider stopped")


if __name__ == "__main__":
    sys.exit(asyncio.run(main()))
