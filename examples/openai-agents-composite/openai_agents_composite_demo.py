"""Agentic SpendGuard + OpenAI Agents SDK runnable demo.

Demonstrates per-step budget enforcement for an `agents.Agent` running under
`Runner.run()`. Every model invocation inside the agent loop is gated through
the SpendGuard sidecar PRE-call. Budget exhaustion fail-closes the call before
the provider HTTP request is made.

Two modes:

* ``--mock`` (default): in-process fake — no openai-agents SDK, no SpendGuard
  sidecar, no OpenAI API key required. Exercises the PRE→ALLOW→LLM and
  PRE→DENY→short-circuit contracts so the wrapper invariant is provably
  enforced.
* ``--real``: wraps the real ``OpenAIChatCompletionsModel`` with
  ``SpendGuardAgentsModel`` against a live sidecar. Requires the SpendGuard
  demo stack running (``make demo-up``) plus ``OPENAI_API_KEY``.

Usage::

    # No dependencies — runs offline
    python examples/openai-agents-composite/openai_agents_composite_demo.py --mock

    # Against the demo stack (see README.md prerequisites)
    OPENAI_API_KEY=sk-... \\
    python examples/openai-agents-composite/openai_agents_composite_demo.py --real \\
        --socket /var/run/spendguard/adapter.sock \\
        --tenant 00000000-0000-4000-8000-000000000001 \\
        --budget 44444444-4444-4444-8444-444444444444 \\
        --window 55555555-5555-4555-8555-555555555555 \\
        --unit   66666666-6666-4666-8666-666666666666

See README.md for the full positioning matrix and what each path proves.
"""

from __future__ import annotations

import argparse
import asyncio
import os
import sys
from dataclasses import dataclass, field
from typing import Any, Callable


# ---------------------------------------------------------------------------
# Mock mode — exercises the SpendGuard PRE-call invariant without bringing in
# openai-agents or the sidecar. Mirrors what SpendGuardAgentsModel.get_response
# does: PRE decision first, only ALLOW reaches the inner Model.
# ---------------------------------------------------------------------------


@dataclass
class MockSpendGuardTransport:
    """Records PRE decisions; in-process; no sidecar.

    Mirrors the request_decision side of SpendGuardClient. Tracks both PRE
    calls (what the wrapper asks SpendGuard about) and LLM calls (what
    actually reached the inner Model). The deny path MUST keep llm_calls at
    zero — that's the invariant the example is built to demonstrate.
    """

    budget_cap_atomic: int
    used_atomic: int = 0
    pre_calls: list[int] = field(default_factory=list)
    llm_calls: list[str] = field(default_factory=list)

    def request_decision(self, claim_atomic: int) -> tuple[bool, str]:
        self.pre_calls.append(claim_atomic)
        if self.used_atomic + claim_atomic <= self.budget_cap_atomic:
            self.used_atomic += claim_atomic
            return True, "ALLOW"
        return False, "BUDGET_EXHAUSTED"

    @property
    def remaining_atomic(self) -> int:
        return self.budget_cap_atomic - self.used_atomic


class BudgetExhausted(Exception):
    """Raised on SpendGuard deny — the real wrapper surfaces this through
    `agents.exceptions.UserError` so Runner.run propagates it cleanly. In mock
    mode we use a plain exception so the demo runs without the SDK."""


@dataclass
class MockLLMResponse:
    text: str
    total_tokens: int


class MockGuardedAgentsModel:
    """In-process fake of SpendGuardAgentsModel.

    Does not subclass `agents.models.interface.Model` so this file runs without
    the openai-agents package installed. The contract it enforces is the same:

        PRE: claim_estimator(input) → transport.request_decision(claim_atomic)
        DENY → raise BudgetExhausted, NEVER call inner
        ALLOW → call inner (here a canned-response stand-in)
    """

    def __init__(
        self,
        transport: MockSpendGuardTransport,
        claim_estimator: Callable[[str], int],
    ) -> None:
        self._transport = transport
        self._claim_estimator = claim_estimator

    async def get_response(self, prompt: str) -> MockLLMResponse:
        claim = self._claim_estimator(prompt)
        allowed, reason = self._transport.request_decision(claim)
        if not allowed:
            raise BudgetExhausted(reason)
        # ALLOW path — this is where the real wrapper delegates to
        # OpenAIChatCompletionsModel. Mock just returns a canned response.
        self._transport.llm_calls.append(prompt)
        return MockLLMResponse(text=f"[mock-llm] echo: {prompt}", total_tokens=claim)


async def mock_main() -> None:
    print("=" * 60)
    print("  SpendGuard + OpenAI Agents SDK Demo (mock mode)")
    print("=" * 60)

    transport = MockSpendGuardTransport(budget_cap_atomic=1000, used_atomic=800)

    print("\n--- Setup ---")
    print("  Inner model: MOCK (canned response — no OpenAI API key needed)")
    print("  SpendGuard transport: MOCK (in-process — no sidecar)")
    print(
        f"  Budget cap: {transport.budget_cap_atomic} atomic units, "
        f"{transport.used_atomic} already used "
        f"({transport.remaining_atomic} remaining)"
    )

    guarded = MockGuardedAgentsModel(
        transport=transport,
        # Tokens-equivalent atomic estimate per call. Real integrations pass
        # back a list[BudgetClaim] from the proto schema; here we collapse to
        # an int for clarity.
        claim_estimator=lambda prompt: 100 if "cheap" in prompt else 500,
    )

    # --- Path 1: ALLOW — budget has room, LLM is invoked ---
    print("\n--- Path 1: ALLOW (budget has room) ---")
    before_llm = len(transport.llm_calls)
    response = await guarded.get_response("Say hello (cheap)")
    print(f"  Prompt: 'Say hello (cheap)'  (estimated 100 atomic units)")
    print(f"  PRE decision: ALLOW")
    print(f"  LLM called: {len(transport.llm_calls) - before_llm == 1}")
    print(f"  Response: {response.text!r}")
    print(f"  Remaining budget: {transport.remaining_atomic} atomic units")
    assert len(transport.llm_calls) - before_llm == 1, (
        "Path 1: ALLOW must invoke the inner Model"
    )

    # --- Path 2: DENY — budget exhausted, LLM short-circuited ---
    print("\n--- Path 2: DENY (budget exhausted) ---")
    before_llm = len(transport.llm_calls)
    try:
        await guarded.get_response("Generate a long essay")  # estimated 500
    except BudgetExhausted as e:
        print(f"  Prompt: 'Generate a long essay'  (estimated 500 atomic units)")
        print(f"  PRE decision: DENY ({e})")
        print(f"  LLM called: {len(transport.llm_calls) - before_llm > 0}")
        print(
            f"  Provider HTTP request was NOT issued — fail-closed enforcement "
            f"works as expected."
        )
    else:
        raise AssertionError("Path 2: DENY must raise BudgetExhausted")
    assert len(transport.llm_calls) - before_llm == 0, (
        "Path 2: DENY MUST NOT call the inner Model — this is the core invariant"
    )

    # --- Path 3: ALLOW — verify ledger state advanced after Path 1 only ---
    print("\n--- Path 3: ledger state after the run ---")
    print(f"  PRE calls recorded: {len(transport.pre_calls)} (expected 2)")
    print(f"  LLM calls recorded: {len(transport.llm_calls)} (expected 1)")
    print(f"  Atomic consumed:    {transport.used_atomic - 800} (expected 100)")
    assert len(transport.pre_calls) == 2 and len(transport.llm_calls) == 1

    print("\n" + "=" * 60)
    print("  All paths PASS — wrapper invariant verified:")
    print("    SpendGuard DENY ⇒ inner Model is NEVER invoked")
    print("=" * 60)


# ---------------------------------------------------------------------------
# Real mode — uses the actual openai-agents SDK + SpendGuardAgentsModel.
# Mirrors deploy/demo/demo/run_demo.py::run_openai_agents_mode().
# ---------------------------------------------------------------------------


async def real_main(
    *,
    socket: str,
    tenant: str,
    budget: str,
    window: str,
    unit_id: str,
    pricing_version: str,
) -> None:
    if not os.environ.get("OPENAI_API_KEY"):
        sys.exit(
            "[demo] FATAL: --real mode requires OPENAI_API_KEY in the environment."
        )

    try:
        from agents import Agent, Runner
        from agents.models.openai_chatcompletions import OpenAIChatCompletionsModel
        from openai import AsyncOpenAI

        from spendguard import SpendGuardClient, new_uuid7
        from spendguard.integrations.openai_agents import (
            RunContext,
            SpendGuardAgentsModel,
            run_context,
        )
        from spendguard._proto.spendguard.common.v1 import common_pb2
    except ImportError as exc:
        sys.exit(
            "[demo] --real mode requires the openai-agents extra of "
            "spendguard-sdk:\n"
            "    pip install --pre 'spendguard-sdk[openai-agents]>=0.4'\n"
            f"(import failed: {exc})"
        )

    print("=" * 60)
    print("  SpendGuard + OpenAI Agents SDK Demo (real mode)")
    print("=" * 60)
    print(f"  Sidecar:   {socket}")
    print(f"  Tenant:    {tenant}")
    print(f"  Budget:    {budget}")

    unit = common_pb2.UnitRef(
        unit_id=unit_id, token_kind="output_token", model_family="gpt-4"
    )
    pricing = common_pb2.PricingFreeze(pricing_version=pricing_version)

    def estimate_claims(_inp: Any) -> list:
        return [
            common_pb2.BudgetClaim(
                budget_id=budget,
                unit=unit,
                amount_atomic="500",
                direction=common_pb2.BudgetClaim.DEBIT,
                window_instance_id=window,
            )
        ]

    async with SpendGuardClient(socket_path=socket, tenant_id=tenant) as client:
        await client.handshake()

        inner_model = OpenAIChatCompletionsModel(
            model="gpt-4o-mini", openai_client=AsyncOpenAI()
        )
        guarded = SpendGuardAgentsModel(
            inner=inner_model,
            client=client,
            budget_id=budget,
            window_instance_id=window,
            unit=unit,
            pricing=pricing,
            claim_estimator=estimate_claims,
        )

        agent = Agent(
            name="spendguard-demo", instructions="Reply concisely.", model=guarded
        )
        run_id = str(new_uuid7())
        async with run_context(RunContext(run_id=run_id)):
            result = await Runner.run(agent, "Say hello in three words.")

        output = getattr(result, "final_output", None) or str(result)
        print(f"\n  Runner.run OK")
        print(f"  output={output!r}")
        print(f"  run_id={run_id}")

    print("\n" + "=" * 60)
    print("  Real-mode run complete — see SpendGuard audit chain for the row.")
    print("=" * 60)


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    mode = parser.add_mutually_exclusive_group()
    mode.add_argument("--mock", action="store_true",
                      help="in-process fake — no sidecar, no API key (default)")
    mode.add_argument("--real", action="store_true",
                      help="wrap real OpenAIChatCompletionsModel against live sidecar")
    parser.add_argument("--socket", default="/var/run/spendguard/adapter.sock")
    parser.add_argument("--tenant", default="00000000-0000-4000-8000-000000000001")
    parser.add_argument("--budget", default="44444444-4444-4444-8444-444444444444")
    parser.add_argument("--window", default="55555555-5555-4555-8555-555555555555")
    parser.add_argument("--unit",   default="66666666-6666-4666-8666-666666666666")
    parser.add_argument("--pricing-version", default="demo-pricing-v1")
    args = parser.parse_args()

    if args.real:
        asyncio.run(real_main(
            socket=args.socket,
            tenant=args.tenant,
            budget=args.budget,
            window=args.window,
            unit_id=args.unit,
            pricing_version=args.pricing_version,
        ))
    else:
        asyncio.run(mock_main())


if __name__ == "__main__":
    main()
