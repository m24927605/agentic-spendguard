# D25 — Implementation

**Reads:** [`design.md`](design.md), [`acceptance.md`](acceptance.md), [`review-standards.md`](review-standards.md).
**Touches:** Python SDK + demo orchestration + public docs. No Rust changes. No proto changes. No DB schema changes.

## 1. Module layout

```
sdk/python/src/spendguard/integrations/
├── openai_agents.py          # existing — RunContext / run_context REUSED, NOT modified
└── smolagents.py             # NEW — D25 (~350 LOC)

sdk/python/pyproject.toml     # +`smolagents` extra → `smolagents>=1.5`

sdk/python/tests/integrations/
├── test_smolagents.py        # NEW — unit tests with FakeSmolModel (~300 LOC)
└── test_smolagents_real.py   # NEW — integration tests over InferenceClientModel + OpenAIServerModel (~200 LOC)

deploy/demo/smolagents/       # NEW
└── bootstrap.py              # env-driven SpendGuardSmolModel factory

deploy/demo/
├── Makefile                  # +DEMO_MODE=agent_real_smolagents branch
├── verify_step_smolagents.sql # NEW — SQL gate for the new mode
└── demo/run_demo.py          # +run_agent_real_smolagents_mode()

docs/site/docs/integrations/
└── smolagents.md             # NEW — public docs page
```

## 2. Slice breakdown

### Slice 1 — Module skeleton + extras + ImportError contract (S)

**Files:** `sdk/python/src/spendguard/integrations/smolagents.py` (new), `sdk/python/pyproject.toml` (edit).

```python
# sdk/python/src/spendguard/integrations/smolagents.py
"""SmolAgents Model wrap adapter.

SmolAgents (HuggingFace, Apache-2.0) routes every LLM call through
`smolagents.Model.generate(messages, ...) -> ChatMessage`. The
wrapper subclasses `Model`, gates `generate()` with PRE/POST sidecar
hooks, and aliases `__call__` for `smolagents<1.5` compatibility.

LiteLLMModel users are covered transitively by the D12 SDK shim — see
`docs/site/docs/integrations/litellm-sdk-shim.md`.

Install with:

    pip install 'spendguard-sdk[smolagents]'

Integration shape:

    from smolagents import CodeAgent, OpenAIServerModel
    from spendguard import SpendGuardClient
    from spendguard.integrations.smolagents import (
        SpendGuardSmolModel, spendguard_step_callback,
    )
    from spendguard.integrations.openai_agents import RunContext, run_context

    client = SpendGuardClient(socket_path=..., tenant_id=...)
    await client.connect(); await client.handshake()

    guarded = SpendGuardSmolModel(
        inner=OpenAIServerModel(model_id="gpt-4o-mini", api_base=..., api_key=...),
        client=client, budget_id=..., window_instance_id=...,
        unit=..., pricing=...,
        claim_estimator=lambda messages: [common_pb2.BudgetClaim(...)],
    )
    agent = CodeAgent(
        model=guarded, tools=[...],
        step_callbacks=[spendguard_step_callback(client, run_id="...")],
    )
    async with run_context(RunContext(run_id="...")):
        result = await agent.arun("...")
"""

from __future__ import annotations

import hashlib
import logging
from collections.abc import Callable
from typing import Any

from ..client import DecisionOutcome, SpendGuardClient
from ..ids import derive_idempotency_key, derive_uuid_from_signature
from .openai_agents import current_run_context  # REUSED — do not duplicate

try:
    from smolagents import Model as _SmolModel  # type: ignore[attr-defined]
    from smolagents.models import ChatMessage  # type: ignore[attr-defined]
except ImportError as exc:  # pragma: no cover
    raise ImportError(
        "spendguard.integrations.smolagents requires the [smolagents] extra. "
        "Install with: pip install 'spendguard-sdk[smolagents]'"
    ) from exc

try:
    from spendguard._proto.spendguard.common.v1 import common_pb2
except ImportError as exc:  # pragma: no cover
    raise ImportError("spendguard proto stubs missing. Run `make proto`.") from exc


_log = logging.getLogger(__name__)


ClaimEstimator = Callable[[list[Any]], list[Any]]
"""Project BudgetClaim list from the messages payload."""
```

`pyproject.toml`:

```toml
[project.optional-dependencies]
smolagents = [
  "smolagents>=1.5",
]
```

### Slice 2 — `SpendGuardSmolModel.generate()` + `__call__` alias (M)

```python
def _signature(
    messages: list[Any],
    stop_sequences: Any,
    response_format: Any,
    tools_to_call_from: Any,
    extra_kwargs: dict[str, Any],
) -> str:
    text = (
        repr(messages)
        + "|" + repr(stop_sequences)
        + "|" + repr(response_format)
        + "|" + repr(tools_to_call_from)
        + "|" + repr(sorted(extra_kwargs.items()))
    )
    return hashlib.blake2b(text.encode("utf-8"), digest_size=16).hexdigest()


class SpendGuardSmolModel(_SmolModel):  # type: ignore[misc, valid-type]
    """SmolAgents Model wrapper gating each generate() through the sidecar.

    Subclasses `smolagents.Model` and overrides `generate` to insert
    PRE/POST sidecar hooks around the inner model's call. `__call__` is
    aliased to `generate` so `smolagents<1.5` agents (which still call
    `model(messages, ...)`) route through the same gate.

    Pass-through methods (`get_tool_call_message`, `_prepare_completion_kwargs`,
    `to_dict`, etc.) delegate verbatim to the inner via `__getattr__` fallback.
    """

    def __init__(
        self,
        *,
        inner: _SmolModel,
        client: SpendGuardClient,
        budget_id: str,
        window_instance_id: str,
        unit: Any,
        pricing: Any,
        claim_estimator: ClaimEstimator,
    ) -> None:
        # smolagents.Model.__init__ sets attributes used only by vendor
        # subclasses; the wrapper owns no model_id. Skipping super() keeps
        # the inner's introspection intact (matches D24 / openai_agents pattern).
        self._inner = inner
        self._client = client
        self._budget_id = budget_id
        self._window_instance_id = window_instance_id
        self._unit = unit
        self._pricing = pricing
        self._claim_estimator = claim_estimator

    async def generate(
        self,
        messages: list[Any],
        stop_sequences: list[str] | None = None,
        response_format: Any = None,
        tools_to_call_from: list[Any] | None = None,
        **kwargs: Any,
    ) -> Any:
        ctx = current_run_context()
        signature = _signature(messages, stop_sequences, response_format,
                               tools_to_call_from, kwargs)
        llm_call_id = str(derive_uuid_from_signature(signature, scope="llm_call_id"))
        decision_id = str(derive_uuid_from_signature(signature, scope="decision_id"))
        step_id = f"{ctx.run_id}:smol-call:{signature[:16]}"
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
            projected_claims=self._claim_estimator(messages),
            idempotency_key=idempotency_key,
        )

        try:
            result = await self._inner.generate(
                messages,
                stop_sequences=stop_sequences,
                response_format=response_format,
                tools_to_call_from=tools_to_call_from,
                **kwargs,
            )
        except BaseException as exc:
            if outcome.reservation_ids:
                outcome_kind = (
                    "CANCELLED"
                    if type(exc).__name__ == "CancelledError"
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
        if outcome.reservation_ids:
            await self._client.emit_llm_call_post(
                run_id=ctx.run_id, step_id=step_id, llm_call_id=llm_call_id,
                decision_id=outcome.decision_id,
                reservation_id=outcome.reservation_ids[0],
                provider_reported_amount_atomic="",
                estimated_amount_atomic=str(total_tokens),
                unit=self._unit, pricing=self._pricing,
                provider_event_id="", outcome="SUCCESS",
            )
        return result

    # `smolagents<1.5` agents call `model(messages, ...)`. Route through
    # the gated generate() to prevent version-drift bypass.
    async def __call__(
        self,
        messages: list[Any],
        stop_sequences: list[str] | None = None,
        response_format: Any = None,
        tools_to_call_from: list[Any] | None = None,
        **kwargs: Any,
    ) -> Any:
        return await self.generate(
            messages,
            stop_sequences=stop_sequences,
            response_format=response_format,
            tools_to_call_from=tools_to_call_from,
            **kwargs,
        )

    @staticmethod
    def _extract_total_tokens(result: Any) -> int:
        usage = getattr(result, "token_usage", None)
        if usage is None:
            return 0
        input_tokens = getattr(usage, "input_tokens", 0) or 0
        output_tokens = getattr(usage, "output_tokens", 0) or 0
        return int(input_tokens) + int(output_tokens)

    # Forward arbitrary inner methods (`get_tool_call_message`,
    # `_prepare_completion_kwargs`, `to_dict`, `flatten_messages_as_text`,
    # vendor-specific helpers) without enumerating them — keeps the
    # wrapper resilient to upstream additions.
    def __getattr__(self, name: str) -> Any:
        if name.startswith("_"):
            raise AttributeError(name)
        return getattr(self._inner, name)
```

### Slice 3 — `spendguard_step_callback()` informational helper (S)

```python
def spendguard_step_callback(
    client: SpendGuardClient,
    *,
    run_id: str,
) -> Callable[[Any], None]:
    """Return a step_callbacks-compatible callable that emits an
    informational `agent_step` audit event.

    NOT a gating surface — step_callbacks fire AFTER each step
    completes; they cannot deny a pending LLM call. The callable
    catches every exception so a sidecar outage cannot abort the
    host agent run.

    Use the SpendGuardSmolModel wrapper for actual gating.
    """

    def _cb(step: Any) -> None:
        try:
            step_kind = type(step).__name__  # "ActionStep" | "PlanningStep"
            step_number = getattr(step, "step_number", None)
            client.emit_agent_step_telemetry(
                run_id=run_id,
                step_kind=step_kind,
                step_number=int(step_number) if step_number is not None else 0,
            )
        except Exception:  # never crash the host agent
            _log.warning(
                "spendguard_step_callback swallowed exception",
                exc_info=True,
            )

    return _cb


__all__ = [
    "ClaimEstimator",
    "SpendGuardSmolModel",
    "spendguard_step_callback",
]
```

`SpendGuardClient.emit_agent_step_telemetry` is a thin sidecar pass-through (existing audit-emit substrate; no new proto). If the method does not exist yet at slice time, fall back to `client.emit_custom_audit("agent_step", payload)` — both surfaces are documented in `sdk/python/src/spendguard/client.py`.

### Slice 4 — Tests parametrized over inner Model classes

See `tests.md` §2.

### Slice 5 — Demo mode + docs

`Makefile` addition:

```make
else ifeq ($(DEMO_MODE),agent_real_smolagents)
	@echo "[demo] DEMO_MODE=agent_real_smolagents → CodeAgent wrapped via SpendGuardSmolModel"
	$(MAKE) up_minimal
	docker compose run --rm \
	    --env SPENDGUARD_DEMO_MODE=agent_real_smolagents \
	    demo python /workspace/run_demo.py
	$(MAKE) verify_smolagents
```

`verify_step_smolagents.sql` asserts at least one `audit_outbox` row with `route='llm.call'`, `trigger='LLM_CALL_PRE'`, paired `LLM_CALL_POST` with non-zero `estimated_amount_atomic` within 60s; plus exactly one `LLM_CALL_PRE` with `decision='DENY'` and no paired POST.

## 3. Public docs page

`docs/site/docs/integrations/smolagents.md` — decision table:

| If you use | Install | Integration |
|------------|---------|-------------|
| `InferenceClientModel` (HF Inference API) | `pip install 'spendguard-sdk[smolagents]' smolagents` | `SpendGuardSmolModel(inner=InferenceClientModel(...))` |
| `OpenAIServerModel` (vLLM / Ollama / Together / Groq / OpenAI-compatible) | (same) | `SpendGuardSmolModel(inner=OpenAIServerModel(...))` |
| `TransformersModel` (in-process HF transformers) | (same) | `SpendGuardSmolModel(inner=TransformersModel(...))` — token count POST only |
| `LiteLLMModel` | `pip install spendguard-litellm-shim` | D12 shim, no SmolAgents wrap needed |
| `step_callbacks` telemetry mirror | (same as wrapper) | `spendguard_step_callback(client, run_id=...)` — informational only, does NOT gate |

Page also includes a "polyglot trace" example showing a `CodeAgent` running inside the same `run_context()` as an `openai_agents.Agent`, both producing audit rows under the same `run_id`.
