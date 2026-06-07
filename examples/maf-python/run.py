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
    """End-to-end against the sidecar UDS + counting-stub via httpx."""
    print(
        f"[demo] maf_python driver: --real mode socket={SOCKET_PATH} "
        f"tenant={TENANT_ID} counting_stub={COUNTING_STUB_URL}",
    )

    import httpx
    from agent_framework import ChatContext, ChatResponse, Message

    from spendguard.client import SpendGuardClient
    from spendguard.errors import DecisionDenied, SidecarUnavailable
    from spendguard.integrations.agent_framework import (
        RunContext,
        SpendGuardAgentFrameworkOptions,
        SpendGuardMiddleware,
        run_context,
    )
    from spendguard._proto.spendguard.common.v1 import common_pb2

    # 1) Wait for the sidecar UDS to be visible (Docker volume race).
    deadline = asyncio.get_event_loop().time() + HANDSHAKE_TIMEOUT_S
    last_err: str = ""
    while asyncio.get_event_loop().time() < deadline:
        if os.path.exists(SOCKET_PATH):
            print(f"[demo] sidecar UDS visible at {SOCKET_PATH}")
            break
        await asyncio.sleep(1.0)
    else:
        print(
            f"[demo] FATAL: sidecar UDS at {SOCKET_PATH} did not appear "
            f"within {HANDSHAKE_TIMEOUT_S:.0f}s: {last_err}",
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
        # 3) Wire the middleware.
        options = SpendGuardAgentFrameworkOptions(
            tenant_id=TENANT_ID,
            budget_id=BUDGET_ID,
            window_instance_id=WINDOW_INSTANCE_ID,
            sidecar_socket_path=SOCKET_PATH,
        )

        def _claim_estimator(_messages):
            # Minimal projected claim — the sidecar's contract evaluator
            # owns the actual budget arithmetic. The DENY step overrides
            # via spendguard_estimate_override on the counting-stub
            # request body, mirroring the inngest_agent_kit / openai_agents_ts
            # demos.
            claim = common_pb2.BudgetClaim()
            claim.budget_id = BUDGET_ID
            claim.window_instance_id = WINDOW_INSTANCE_ID
            claim.amount_atomic = "1000000"
            claim.unit.unit_id = "usd_micros"
            return [claim]

        unit = common_pb2.UnitRef(unit_id="usd_micros")
        pricing = common_pb2.PricingFreeze(pricing_version="demo-pricing-v1")

        middleware = SpendGuardMiddleware(
            client=client,
            options=options,
            unit=unit,
            pricing=pricing,
            claim_estimator=_claim_estimator,
        )

        # 4) Define a helper that drives one MAF call through the middleware.
        async with httpx.AsyncClient(
            base_url=COUNTING_STUB_URL, timeout=30.0,
        ) as http:

            async def _read_counter() -> int:
                r = await http.get("/_count")
                r.raise_for_status()
                return int(r.json()["calls"])

            async def _run_step(prompt: str, deny: bool = False) -> bool:
                """Drive ALLOW / DENY iteration. Returns True on CONTINUE."""
                ctx = ChatContext(
                    client=MagicMock(),
                    messages=[Message(role="user", contents=[prompt])],
                    options={"model": "gpt-4o-mini"},
                )
                response = ChatResponse(
                    messages=[Message(role="assistant", contents=["ok"])],
                    response_id="resp-real",
                    model="gpt-4o-mini",
                    usage_details={
                        "input_token_count": 5,
                        "output_token_count": 7,
                        "total_token_count": 12,
                    },
                )

                async def _call_next() -> None:
                    # Hit the counting stub on CONTINUE only; the
                    # middleware throws DecisionDenied before reaching
                    # here on STOP.
                    body: dict[str, Any] = {
                        "model": "gpt-4o-mini",
                        "messages": [{"role": "user", "content": prompt}],
                    }
                    if deny:
                        body["spendguard_estimate_override"] = "2000000000"
                    r = await http.post(
                        "/v1/chat/completions",
                        json=body,
                        headers={
                            "authorization": "Bearer demo-counting-stub",
                        },
                    )
                    r.raise_for_status()
                    ctx.result = response

                try:
                    await middleware.process(ctx, _call_next)
                    return True
                except DecisionDenied as exc:
                    print(
                        f"[demo] caught DecisionDenied: {exc}",
                    )
                    return False
                except SidecarUnavailable:
                    raise

            async with run_context(RunContext(run_id="run-real-1")):
                # ALLOW
                print("[demo] (1) ALLOW step — small message within budget")
                pre_allow = await _read_counter()
                ok1 = await _run_step("hi from python")
                post_allow = await _read_counter()
                if not ok1:
                    print(
                        "[demo] FATAL: ALLOW step raised DecisionDenied "
                        "unexpectedly",
                        file=sys.stderr,
                    )
                    return 7
                if post_allow != pre_allow + 1:
                    print(
                        f"[demo] FATAL ALLOW: counting-stub pre={pre_allow} "
                        f"post={post_allow} (expected +1)",
                        file=sys.stderr,
                    )
                    return 7

                # DENY
                print("[demo] (2) DENY step — forcing hard-cap overflow")
                pre_deny = post_allow
                ok2 = await _run_step(
                    "trigger-deny: please block me", deny=True,
                )
                post_deny = await _read_counter()
                if ok2:
                    print(
                        "[demo] FATAL DENY: middleware did NOT raise "
                        "DecisionDenied",
                        file=sys.stderr,
                    )
                    return 7
                if post_deny != pre_deny:
                    print(
                        f"[demo] FATAL DENY INV-1.6: counting-stub "
                        f"pre={pre_deny} post={post_deny} (expected 0)",
                        file=sys.stderr,
                    )
                    return 7

                # ALLOW2
                print("[demo] (3) ALLOW2 step — second small message within budget")
                pre_allow2 = post_deny
                ok3 = await _run_step("another hi")
                post_allow2 = await _read_counter()
                if not ok3:
                    print(
                        "[demo] FATAL ALLOW2: raised DecisionDenied "
                        "unexpectedly",
                        file=sys.stderr,
                    )
                    return 7
                if post_allow2 != pre_allow2 + 1:
                    print(
                        f"[demo] FATAL ALLOW2: counting-stub "
                        f"pre={pre_allow2} post={post_allow2} (expected +1)",
                        file=sys.stderr,
                    )
                    return 7

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
