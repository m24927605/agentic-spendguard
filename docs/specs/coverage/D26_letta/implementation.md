# D26 — Implementation

**Reads:** [`design.md`](design.md), [`acceptance.md`](acceptance.md), [`review-standards.md`](review-standards.md).
**Touches:** Python SDK + demo orchestration + public docs. No Rust changes. No proto changes. No DB schema changes.

## 1. Module layout

```
sdk/python/src/spendguard/integrations/
├── openai_agents.py        # existing — RunContext / run_context REUSED, NOT modified
└── letta.py                # NEW — D26 (~350 LOC)

sdk/python/pyproject.toml   # +`letta` extra → `letta>=0.8,<1.0`

sdk/python/tests/integrations/
├── conftest_letta.py       # NEW — FakeLLMClient + LettaAgent stub fixtures
├── test_letta.py           # NEW — unit tests with FakeLLMClient (~300 LOC)
└── test_letta_real.py      # NEW — integration tests with real letta>=0.8 (~200 LOC)

deploy/demo/letta/          # NEW
└── bootstrap.py            # env-driven SpendGuardLettaClient factory

deploy/demo/
├── Makefile                # +DEMO_MODE=agent_real_letta branch
├── verify_step_letta.sql   # NEW — SQL gate
└── demo/run_demo.py        # +run_agent_real_letta_mode()

docs/site/docs/integrations/
└── letta.md                # NEW — public docs page (library vs server decision table)
```

## 2. Slice breakdown

### Slice 1 — Module skeleton + extras + factory stub (S)

**Files:** `sdk/python/src/spendguard/integrations/letta.py` (new), `sdk/python/pyproject.toml` (edit).

```python
# sdk/python/src/spendguard/integrations/letta.py
"""Letta (ex-MemGPT) LLMClient subclass adapter.

Letta exposes provider-specific subclasses of
`letta.llm_api.llm_client_base.LLMClientBase`. SpendGuard wraps any
instance via composition: PRE/POST gating around `send_llm_request`
and `send_llm_request_sync`.

When to use what:

- Embedded Letta library (in-process `Agent.step()`)  → use D26 wrap.
- Self-hosted `letta server` REST surface             → use D02/D03 egress-proxy
  drop-in. D26 is unnecessary and ignored on the server-side path.

Install:

    pip install 'spendguard-sdk[letta]'
    # then EITHER:
    pip install 'letta>=0.8,<1.0'

Integration shape:

    from letta.llm_api.openai_client import OpenAIClient

    from spendguard import SpendGuardClient
    from spendguard.integrations.letta import wrap_llm_client
    from spendguard.integrations.openai_agents import RunContext, run_context

    client = SpendGuardClient(socket_path=..., tenant_id=...)
    await client.connect(); await client.handshake()

    inner = OpenAIClient(...)
    guarded = wrap_llm_client(
        inner=inner, client=client,
        budget_id=..., window_instance_id=...,
        unit=..., pricing=...,
        claim_estimator=lambda req: [common_pb2.BudgetClaim(...)],
    )
    agent = letta_agent_factory(llm_client=guarded, ...)

    async with run_context(RunContext(run_id="...")):
        response = await agent.step(message)
"""

from __future__ import annotations

import asyncio
import hashlib
from collections.abc import Callable
from typing import Any

from ..client import DecisionOutcome, SpendGuardClient
from ..ids import derive_idempotency_key, derive_uuid_from_signature
from .openai_agents import current_run_context  # REUSED

try:
    from letta.llm_api.llm_client_base import LLMClientBase as _LLMClientBase
except ImportError as exc:  # pragma: no cover
    raise ImportError(
        "spendguard.integrations.letta requires the [letta] extra. "
        "Install with: pip install 'spendguard-sdk[letta]' 'letta>=0.8,<1.0'."
    ) from exc

try:
    from spendguard._proto.spendguard.common.v1 import common_pb2
except ImportError as exc:  # pragma: no cover
    raise ImportError("spendguard proto stubs missing. Run `make proto`.") from exc


ClaimEstimator = Callable[[Any], list[Any]]
"""Project BudgetClaim list from the Letta `request_data` payload."""


def wrap_llm_client(
    *,
    inner: _LLMClientBase,
    client: SpendGuardClient,
    budget_id: str,
    window_instance_id: str,
    unit: Any,
    pricing: Any,
    claim_estimator: ClaimEstimator,
) -> "SpendGuardLettaClient":
    """Factory — wrap any Letta `LLMClientBase` subclass instance."""
    return SpendGuardLettaClient(
        inner=inner,
        client=client,
        budget_id=budget_id,
        window_instance_id=window_instance_id,
        unit=unit,
        pricing=pricing,
        claim_estimator=claim_estimator,
    )
```

`pyproject.toml`:

```toml
[project.optional-dependencies]
letta = [
  "letta>=0.8,<1.0",
]
```

### Slice 2 — `SpendGuardLettaClient.send_llm_request()` (M)

```python
def _signature(request_data: Any, llm_config: Any, tools: Any, force_tool_use: Any) -> str:
    text = (
        repr(request_data)
        + "|" + repr(llm_config)
        + "|" + repr(tools)
        + "|" + repr(force_tool_use)
    )
    return hashlib.blake2b(text.encode("utf-8"), digest_size=16).hexdigest()


class SpendGuardLettaClient(_LLMClientBase):  # type: ignore[misc, valid-type]
    """Letta LLMClientBase wrapper.

    Subclasses `letta.llm_api.llm_client_base.LLMClientBase` and overrides
    `send_llm_request` / `send_llm_request_sync` to insert PRE/POST sidecar
    hooks around the inner client. Other LLMClientBase methods
    (`build_request_data`, `convert_response_to_chat_completion`, etc.)
    pass through via `__getattr__`.
    """

    def __init__(
        self,
        *,
        inner: _LLMClientBase,
        client: SpendGuardClient,
        budget_id: str,
        window_instance_id: str,
        unit: Any,
        pricing: Any,
        claim_estimator: ClaimEstimator,
    ) -> None:
        # Skip super().__init__() — LLMClientBase init takes provider config
        # the wrapper doesn't own. Inner already initialized; __getattr__
        # delegates llm_config / provider / model accessors.
        self._inner = inner
        self._client = client
        self._budget_id = budget_id
        self._window_instance_id = window_instance_id
        self._unit = unit
        self._pricing = pricing
        self._claim_estimator = claim_estimator

    def __getattr__(self, name: str) -> Any:
        # Delegate any LLMClientBase attribute we don't override
        # (llm_config, provider, build_request_data,
        # convert_response_to_chat_completion, etc.) to inner.
        # Note: __getattr__ only fires after normal attribute lookup fails,
        # so our explicit attrs (_inner, _client, ...) shadow correctly.
        return getattr(self._inner, name)

    async def send_llm_request(
        self,
        request_data: Any,
        llm_config: Any,
        tools: Any = None,
        force_tool_use: bool = False,
        **kwargs: Any,
    ) -> Any:
        ctx = current_run_context()
        signature = _signature(request_data, llm_config, tools, force_tool_use)
        llm_call_id = str(derive_uuid_from_signature(signature, scope="llm_call_id"))
        decision_id = str(derive_uuid_from_signature(signature, scope="decision_id"))
        step_id = f"{ctx.run_id}:letta-call:{signature[:16]}"
        idempotency_key = derive_idempotency_key(
            tenant_id=self._client.tenant_id,
            session_id=self._client.session_id,
            run_id=ctx.run_id,
            step_id=step_id,
            llm_call_id=llm_call_id,
            trigger="LLM_CALL_PRE",
        )

        outcome: DecisionOutcome = await self._client.request_decision(
            trigger="LLM_CALL_PRE",
            run_id=ctx.run_id,
            step_id=step_id,
            llm_call_id=llm_call_id,
            tool_call_id="",
            decision_id=decision_id,
            route="llm.call",
            projected_claims=self._claim_estimator(request_data),
            idempotency_key=idempotency_key,
        )

        try:
            result = await self._inner.send_llm_request(
                request_data, llm_config, tools=tools,
                force_tool_use=force_tool_use, **kwargs,
            )
        except BaseException as exc:
            if outcome.reservation_ids:
                outcome_kind = (
                    "CANCELLED" if type(exc).__name__ == "CancelledError"
                    else "FAILURE"
                )
                await self._client.emit_llm_call_post(
                    run_id=ctx.run_id, step_id=step_id, llm_call_id=llm_call_id,
                    decision_id=outcome.decision_id,
                    reservation_id=outcome.reservation_ids[0],
                    provider_reported_amount_atomic="",
                    estimated_amount_atomic="0",
                    unit=self._unit, pricing=self._pricing,
                    provider_event_id="", outcome=outcome_kind,
                )
            raise

        total_tokens = self._extract_total_tokens(result)
        provider_event_id = self._extract_provider_event_id(result)
        if outcome.reservation_ids:
            await self._client.emit_llm_call_post(
                run_id=ctx.run_id, step_id=step_id, llm_call_id=llm_call_id,
                decision_id=outcome.decision_id,
                reservation_id=outcome.reservation_ids[0],
                provider_reported_amount_atomic="",
                estimated_amount_atomic=str(total_tokens),
                unit=self._unit, pricing=self._pricing,
                provider_event_id=provider_event_id, outcome="SUCCESS",
            )
        return result

    @staticmethod
    def _extract_total_tokens(result: Any) -> int:
        # Letta normalizes all provider responses through
        # convert_response_to_chat_completion → ChatCompletionResponse
        # which carries OpenAI-style usage. Use total_tokens directly.
        usage = getattr(result, "usage", None)
        if usage is None:
            return 0
        total = getattr(usage, "total_tokens", None)
        if isinstance(total, int):
            return total
        prompt = getattr(usage, "prompt_tokens", 0) or 0
        completion = getattr(usage, "completion_tokens", 0) or 0
        return int(prompt) + int(completion)

    @staticmethod
    def _extract_provider_event_id(result: Any) -> str:
        # ChatCompletionResponse.id when present (OpenAI-shaped).
        return str(getattr(result, "id", "") or "")
```

### Slice 3 — `send_llm_request_sync()` + passthrough (S)

```python
    def send_llm_request_sync(
        self,
        request_data: Any,
        llm_config: Any,
        tools: Any = None,
        force_tool_use: bool = False,
        **kwargs: Any,
    ) -> Any:
        # Detect running loop — if inside one, refuse silent asyncio.run().
        try:
            asyncio.get_running_loop()
        except RuntimeError:
            running = False
        else:
            running = True
        if running:
            raise RuntimeError(
                "spendguard.integrations.letta.SpendGuardLettaClient."
                "send_llm_request_sync called from inside an active asyncio "
                "loop. Use `await client.send_llm_request(...)` instead."
            )
        return asyncio.run(
            self.send_llm_request(
                request_data, llm_config, tools=tools,
                force_tool_use=force_tool_use, **kwargs,
            )
        )


__all__ = ["SpendGuardLettaClient", "ClaimEstimator", "wrap_llm_client"]
```

### Slice 4 — Tests

See `tests.md` §2.

### Slice 5 — Demo + docs

`Makefile` additions:

```make
else ifeq ($(DEMO_MODE),agent_real_letta)
	@echo "[demo] DEMO_MODE=agent_real_letta → Letta Agent wrapped via SpendGuardLettaClient"
	$(MAKE) up_minimal
	docker compose run --rm \
	    --env SPENDGUARD_DEMO_MODE=agent_real_letta \
	    demo python /workspace/run_demo.py
	$(MAKE) verify_letta
```

`verify_step_letta.sql` asserts at least one `audit_outbox` row with `route='llm.call'`, `trigger='LLM_CALL_PRE'`, and a paired `LLM_CALL_POST` with non-zero `estimated_amount_atomic` within 60s. Also asserts at least one `LLM_CALL_PRE` with `decision='DENY'` and NO paired POST.

## 3. Public docs page

`docs/site/docs/integrations/letta.md` — single page leading with the decision table:

| If you run Letta as | Use | Why |
|---------------------|-----|-----|
| **`letta server` REST** (recommended) | D02 closed-CLI install + D03 base-URL drop-in. Skip D26. | Egress-proxy already covers it; no SDK changes in Letta. |
| **Embedded library** (`from letta import ...`) | D26 `wrap_llm_client(inner=OpenAIClient(...), ...)` | The only safe per-call gate without upstream hooks. |
| **LiteLLM-routed** (any Letta deployment) | D12 LiteLLM SDK shim covers transitively | No D26 work needed. |

Followed by a working `wrap_llm_client(...)` code block and a polyglot-trace example sharing `RunContext` with the OpenAI Agents SDK adapter.
