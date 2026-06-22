#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# Copyright (c) SpendGuard Authors.
"""COV_d07 SLICE 8 — Python half of the both-language MAF demo.

Two modes (selected via the first CLI arg):

* ``--mock`` — In-process ``SpendGuardClient`` mock + in-process
  ``ChatMiddleware`` stack. No sidecar, no counting-stub required.
  Drives 3 calls (ALLOW + DENY + ALLOW2) and exits 0 on PASS / 7 on
  FAIL. Mirrors ``examples/maf-dotnet/Program.cs`` ``--mock`` mode.

* ``--real`` — Connect a real ``SpendGuardClient`` over the sidecar UDS,
  build a ``ChatMiddleware`` against it, and drive 3 calls through a
  counting-stub-backed inner ``ChatClient``. The
  ``DEMO_MODE=maf_python_real`` Makefile target wires this up.

3-step matrix (mirrors D04 / D06 / D08 composite demos):

  step 1 ALLOW   — small message within budget → counter +1.
  step 2 DENY    — message tagged ``trigger-deny`` so the sidecar
                   contract evaluator emits ``SPENDGUARD_DENY`` →
                   middleware raises ``DecisionDenied`` BEFORE the
                   inner chat client's HTTP fires → counter unchanged.
  step 3 ALLOW2  — second ALLOW call exercising cross-call
                   determinism. Replaces D04 / D06 / D08's STREAM step
                   (streaming gating is v0.1.x non-goal — design.md §3).

Success line (LOCKED — CI grep depends on the exact spelling, mirrors
the openai_agents_ts / inngest_agent_kit composite convention)::

    `[demo] maf_python ALL 3 steps PASS (ALLOW + DENY + ALLOW2)`

Launched by:

* direct ``python examples/maf-python/run.py --mock`` for laptop
  iteration.
* ``deploy/demo/demo/run_demo.py::run_maf_python_mode`` in the
  ``DEMO_MODE=maf_python_real`` Makefile target.
"""

from __future__ import annotations

import asyncio
import os
import sys
from types import SimpleNamespace
from typing import Any
from unittest.mock import AsyncMock, MagicMock

# ─── Shared config ──────────────────────────────────────────────────────────

SOCKET_PATH = os.environ.get(
    "SPENDGUARD_SIDECAR_UDS", "/var/run/spendguard/adapter.sock",
)
TENANT_ID = os.environ.get(
    "SPENDGUARD_TENANT_ID", "00000000-0000-4000-8000-000000000001",
)
BUDGET_ID = os.environ.get(
    "SPENDGUARD_BUDGET_ID", "44444444-4444-4444-8444-444444444444",
)
WINDOW_INSTANCE_ID = os.environ.get(
    "SPENDGUARD_WINDOW_INSTANCE_ID", "55555555-5555-4555-8555-555555555555",
)
# The unit MUST be a UUID (the ledger ledger_units.unit_id column is uuid) —
# the old "usd_micros" literal made the commit fail with "invalid input
# syntax for type uuid". The full pricing tuple (snapshot hash / fx / unit
# conversion) is sourced from the bundles runtime.env by the overlay so the
# commit's PricingFreeze matches the contract bundle the sidecar loaded.
UNIT_ID = os.environ.get(
    "SPENDGUARD_UNIT_ID", "66666666-6666-4666-8666-666666666666",
)
PRICING_VERSION = os.environ.get("SPENDGUARD_PRICING_VERSION", "demo-pricing-v1")
FX_RATE_VERSION = os.environ.get("SPENDGUARD_FX_RATE_VERSION", "demo-fx-v1")
UNIT_CONVERSION_VERSION = os.environ.get(
    "SPENDGUARD_UNIT_CONVERSION_VERSION", "demo-units-v1",
)
PRICE_SNAPSHOT_HASH_HEX = os.environ.get("SPENDGUARD_PRICE_SNAPSHOT_HASH_HEX", "")
COUNTING_STUB_URL = os.environ.get(
    "SPENDGUARD_COUNTING_STUB_URL", "http://counting-stub:8765",
)
HANDSHAKE_TIMEOUT_S = float(
    os.environ.get("SPENDGUARD_HANDSHAKE_TIMEOUT_MS", "30000"),
) / 1000.0


# ─── --mock implementation ─────────────────────────────────────────────────


async def _mock_main() -> int:
    """In-process MAF middleware + stub sidecar + stub chat client."""
    print("[demo] maf_python driver: --mock mode (no sidecar, in-process stubs)")

    from agent_framework import ChatContext, ChatResponse, Message

    from spendguard.errors import DecisionDenied
    from spendguard.integrations.agent_framework import (
        RunContext,
        SpendGuardAgentFrameworkOptions,
        SpendGuardMiddleware,
        run_context,
    )

    # In-process SpendGuardClient double. The middleware only touches
    # ``request_decision`` + ``emit_llm_call_post`` + ``release_reservation``
    # on the happy/sad path, plus the ``tenant_id`` / ``session_id``
    # attribute getters. We mirror the SLICE 7 unit-test shape.
    class MockSpendGuardClient:
        def __init__(self) -> None:
            self.tenant_id = TENANT_ID
            self.session_id = "session-mock-1"
            self.request_count = 0
            self.commit_count = 0
            self.next_deny = False

        async def request_decision(self, **kwargs: Any):
            self.request_count += 1
            if self.next_deny:
                self.next_deny = False
                raise DecisionDenied(
                    f"contract-eval STOP request_count={self.request_count}",
                    decision_id=f"dec-{self.request_count}",
                    reason_codes=("BUDGET_EXCEEDED",),
                )
            return SimpleNamespace(
                decision_id=f"dec-{self.request_count}",
                reservation_ids=(f"res-{self.request_count}",),
                audit_decision_event_id=f"aud-{self.request_count}",
            )

        async def emit_llm_call_post(self, **kwargs: Any) -> None:
            self.commit_count += 1

        async def release_reservation(self, **kwargs: Any) -> None:
            pass

    client = MockSpendGuardClient()

    # Build the middleware against the mock client.
    options = SpendGuardAgentFrameworkOptions(
        tenant_id=TENANT_ID,
        budget_id=BUDGET_ID,
        window_instance_id=WINDOW_INSTANCE_ID,
        sidecar_socket_path=SOCKET_PATH,
    )

    def _claim_estimator(_messages):
        return [
            SimpleNamespace(
                budget_id=BUDGET_ID,
                window_instance_id=WINDOW_INSTANCE_ID,
                amount_atomic="1000000",
                unit=SimpleNamespace(unit_id="usd_micros"),
            ),
        ]

    middleware = SpendGuardMiddleware(
        client=client,
        options=options,
        unit=SimpleNamespace(unit_id="usd_micros"),
        pricing=SimpleNamespace(pricing_version="demo-pricing-v1"),
        claim_estimator=_claim_estimator,
    )

    # Mock inner chat client: counts every invocation. The MAF
    # ``call_next`` closure flows the result back through
    # ``context.result``. We adapt by writing a fixed ``ChatResponse``
    # into the context inside the call_next.
    inner_call_count = 0

    async def _run_step(prompt: str) -> bool:
        """Drive a single ALLOW / DENY iteration through the middleware.

        Returns True on CONTINUE, False on DENY.
        """
        nonlocal inner_call_count
        ctx = ChatContext(
            client=MagicMock(),
            messages=[Message(role="user", contents=[prompt])],
            options={"model": "gpt-4o-mini"},
        )
        response = ChatResponse(
            messages=[Message(role="assistant", contents=["ok from mock"])],
            response_id="resp-mock",
            model="gpt-4o-mini",
            usage_details={
                "input_token_count": 5,
                "output_token_count": 7,
                "total_token_count": 12,
            },
        )

        async def _call_next() -> None:
            nonlocal inner_call_count
            inner_call_count += 1
            ctx.result = response

        try:
            await middleware.process(ctx, _call_next)
            return True
        except DecisionDenied:
            return False

    async with run_context(RunContext(run_id="run-mock-1")):
        # ALLOW
        print("[demo] (1) ALLOW step — small message within budget")
        ok1 = await _run_step("hi from python")
        if not ok1:
            print("[demo] FATAL: ALLOW step raised DecisionDenied unexpectedly",
                  file=sys.stderr)
            return 7
        if inner_call_count != 1:
            print(
                f"[demo] FATAL ALLOW: inner_call_count={inner_call_count} "
                "(expected 1)",
                file=sys.stderr,
            )
            return 7

        # DENY
        print("[demo] (2) DENY step — forcing contract STOP")
        client.next_deny = True
        ok2 = await _run_step("trigger-deny: please block me")
        if ok2:
            print(
                "[demo] FATAL DENY: middleware did NOT raise DecisionDenied",
                file=sys.stderr,
            )
            return 7
        if inner_call_count != 1:
            print(
                f"[demo] FATAL DENY INV-1.6: inner was called; "
                f"inner_call_count={inner_call_count} (expected 1)",
                file=sys.stderr,
            )
            return 7

        # ALLOW2
        print("[demo] (3) ALLOW2 step — second small message within budget")
        ok3 = await _run_step("another hi")
        if not ok3:
            print("[demo] FATAL ALLOW2: raised DecisionDenied unexpectedly",
                  file=sys.stderr)
            return 7
        if inner_call_count != 2:
            print(
                f"[demo] FATAL ALLOW2: inner_call_count={inner_call_count} "
                "(expected 2)",
                file=sys.stderr,
            )
            return 7

    print("[demo] maf_python ALL 3 steps PASS (ALLOW + DENY + ALLOW2)")
    print(
        f"[demo] summary: request_count={client.request_count} "
        f"commit_count={client.commit_count} "
        f"inner_call_count={inner_call_count}",
    )
    return 0


# ─── --real implementation ─────────────────────────────────────────────────


async def _real_main() -> int:
    """End-to-end against the sidecar UDS + counting-stub via a REAL MAF
    ``OpenAIChatClient.get_response(messages, middleware=[SpendGuardMiddleware])``
    — no MagicMock, no hand-rolled ``call_next``, no fabricated ``ChatResponse``.
    MAF invokes the middleware's ``process(context, call_next)`` with its OWN
    real chat call as ``call_next``; the SpendGuard gate runs PRE, the real
    OpenAIChatClient HTTP hits the counting-stub on ALLOW, and on DENY the gate
    raises BEFORE that HTTP (counter unchanged).
    """
    print(
        f"[demo] maf_python driver: --real mode socket={SOCKET_PATH} "
        f"tenant={TENANT_ID} counting_stub={COUNTING_STUB_URL}",
    )

    import httpx
    from agent_framework import Message
    # OpenAIChatCompletionClient targets the /v1/chat/completions API (the
    # counting-stub's shape); the default OpenAIChatClient targets the newer
    # /v1/responses API which the stub does not serve (404).
    from agent_framework.openai import OpenAIChatCompletionClient
    from openai import AsyncOpenAI

    from spendguard.client import SpendGuardClient
    from spendguard.errors import (
        DecisionDenied,
        SidecarUnavailable,
        SpendGuardError,
    )
    from spendguard.integrations.agent_framework import (
        RunContext,
        SpendGuardAgentFrameworkOptions,
        SpendGuardMiddleware,
        run_context,
    )
    from spendguard._proto.spendguard.common.v1 import common_pb2

    if not PRICE_SNAPSHOT_HASH_HEX:
        print(
            "[demo] FATAL: SPENDGUARD_PRICE_SNAPSHOT_HASH_HEX not set — the "
            "overlay must source the bundles runtime.env so the commit's "
            "PricingFreeze matches the contract bundle.",
            file=sys.stderr,
        )
        return 7

    # 1) Wait for the sidecar UDS to be visible (Docker volume race).
    deadline = asyncio.get_event_loop().time() + HANDSHAKE_TIMEOUT_S
    while asyncio.get_event_loop().time() < deadline:
        if os.path.exists(SOCKET_PATH):
            print(f"[demo] sidecar UDS visible at {SOCKET_PATH}")
            break
        await asyncio.sleep(1.0)
    else:
        print(
            f"[demo] FATAL: sidecar UDS at {SOCKET_PATH} did not appear "
            f"within {HANDSHAKE_TIMEOUT_S:.0f}s",
            file=sys.stderr,
        )
        return 7

    # 2) Connect a real SpendGuardClient.
    client = SpendGuardClient(
        socket_path=SOCKET_PATH,
        tenant_id=TENANT_ID,
        runtime_kind="microsoft-agent-framework-python",
    )
    await client.connect()
    await client.handshake()
    print(f"[demo] handshake ok session_id={client.session_id}")

    try:
        options = SpendGuardAgentFrameworkOptions(
            tenant_id=TENANT_ID,
            budget_id=BUDGET_ID,
            window_instance_id=WINDOW_INSTANCE_ID,
            sidecar_socket_path=SOCKET_PATH,
        )

        # UUID unit (commit lands in the uuid ledger_units column) + the full
        # pricing tuple from the bundles runtime.env.
        unit = common_pb2.UnitRef(
            unit_id=UNIT_ID, token_kind="output_token", model_family="gpt-4"
        )
        pricing = common_pb2.PricingFreeze(
            pricing_version=PRICING_VERSION,
            price_snapshot_hash=bytes.fromhex(PRICE_SNAPSHOT_HASH_HEX),
            fx_rate_version=FX_RATE_VERSION,
            unit_conversion_version=UNIT_CONVERSION_VERSION,
        )

        # One estimator drives every turn via a mutable flag. ALLOW claims a
        # small 100 atomic (so ALLOW + ALLOW2 both fit the 500-atomic demo
        # budget); DENY flips to a 2B raw claim through the MIDDLEWARE's gate
        # (NOT a counting-stub body override) so the sidecar's contract
        # evaluator sees the overflow and blocks BEFORE call_next.
        deny_state = {"deny": False}

        def _claim_estimator(_messages):
            amount = "2000000000" if deny_state["deny"] else "100"
            claim = common_pb2.BudgetClaim()
            claim.budget_id = BUDGET_ID
            claim.window_instance_id = WINDOW_INSTANCE_ID
            claim.amount_atomic = amount
            claim.unit.unit_id = UNIT_ID
            return [claim]

        middleware = SpendGuardMiddleware(
            client=client,
            options=options,
            unit=unit,
            pricing=pricing,
            claim_estimator=_claim_estimator,
        )

        # Real MAF chat client pointed at the counting-stub via an explicit
        # AsyncOpenAI (guarantees base_url overrides the public endpoint).
        chat_client = OpenAIChatCompletionClient(
            model="gpt-4o-mini",
            async_client=AsyncOpenAI(
                api_key="sk-spendguard-demo-stub",
                base_url=f"{COUNTING_STUB_URL}/v1",
            ),
        )

        async def _read_counter() -> int:
            async with httpx.AsyncClient(
                base_url=COUNTING_STUB_URL, timeout=30.0
            ) as h:
                r = await h.get("/_count")
                r.raise_for_status()
                return int(r.json()["calls"])

        def _is_block(exc: BaseException) -> bool:
            # Fail-closed: a contract hard-cap STOP raises DecisionDenied; a
            # large-claim/approval or budget-floor STOP surfaces as a
            # SpendGuardError. Either is a genuine block (the counter-flat
            # assertion + the ALLOW positive control rule out an outage). Walk
            # the cause/context chain — MAF may wrap the middleware error.
            seen: set[int] = set()
            cur: BaseException | None = exc
            while cur is not None and id(cur) not in seen:
                seen.add(id(cur))
                if isinstance(cur, (DecisionDenied, SpendGuardError)):
                    return True
                cur = cur.__cause__ or cur.__context__
            return False

        async def _drive(prompt: str):
            return await chat_client.get_response(
                messages=[Message(role="user", contents=[prompt])],
                middleware=[middleware],
            )

        async with run_context(RunContext(run_id="run-real-1")):
            # ── (1) ALLOW ───────────────────────────────────────────────
            print("[demo] (1) ALLOW step — small message within budget")
            pre_allow = await _read_counter()
            await _drive("hi from python")
            post_allow = await _read_counter()
            if post_allow != pre_allow + 1:
                print(
                    f"[demo] FATAL ALLOW: counting-stub pre={pre_allow} "
                    f"post={post_allow} (expected +1)",
                    file=sys.stderr,
                )
                return 7
            print(f"[demo] ALLOW ok — provider reached {pre_allow} -> {post_allow}")

            # ── (2) DENY ────────────────────────────────────────────────
            print("[demo] (2) DENY step — 2B raw claim through the gate")
            deny_state["deny"] = True
            pre_deny = post_allow
            blocked = False
            try:
                await _drive("trigger-deny: please block me")
            except (DecisionDenied, SpendGuardError):
                blocked = True
            except Exception as exc:  # noqa: BLE001 — MAF may wrap the block
                if _is_block(exc):
                    blocked = True
                else:
                    raise
            post_deny = await _read_counter()
            deny_state["deny"] = False
            if not blocked:
                print(
                    "[demo] FATAL DENY: middleware did NOT fail-closed",
                    file=sys.stderr,
                )
                return 7
            if post_deny != pre_deny:
                print(
                    f"[demo] FATAL DENY: counting-stub pre={pre_deny} "
                    f"post={post_deny} (expected unchanged)",
                    file=sys.stderr,
                )
                return 7
            print(
                f"[demo] DENY ok — gate blocked BEFORE provider; counting "
                f"UNCHANGED ({pre_deny} -> {post_deny})"
            )

            # ── (3) ALLOW2 ──────────────────────────────────────────────
            print("[demo] (3) ALLOW2 step — second small message within budget")
            pre_allow2 = post_deny
            await _drive("another hi")
            post_allow2 = await _read_counter()
            if post_allow2 != pre_allow2 + 1:
                print(
                    f"[demo] FATAL ALLOW2: counting-stub pre={pre_allow2} "
                    f"post={post_allow2} (expected +1)",
                    file=sys.stderr,
                )
                return 7
            print(
                f"[demo] ALLOW2 ok — provider reached {pre_allow2} -> {post_allow2}"
            )

        print("[demo] maf_python ALL 3 steps PASS (ALLOW + DENY + ALLOW2)")
        return 0
    finally:
        await client.close()


# ─── Entry point ───────────────────────────────────────────────────────────


def _parse_mode(argv: list[str]) -> str:
    if "--real" in argv:
        return "real"
    if "--mock" in argv:
        return "mock"
    return "mock"


async def _main() -> int:
    mode = _parse_mode(sys.argv[1:])
    try:
        if mode == "real":
            return await _real_main()
        return await _mock_main()
    except Exception as exc:  # noqa: BLE001
        import traceback
        print(f"[demo] FAIL: {type(exc).__name__}: {exc}", file=sys.stderr)
        traceback.print_exc()
        return 7


if __name__ == "__main__":
    sys.exit(asyncio.run(_main()))
