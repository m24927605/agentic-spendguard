# D28 — Implementation

**Reads:** [`design.md`](design.md), [`acceptance.md`](acceptance.md), [`review-standards.md`](review-standards.md).
**Touches:** Python SDK + demo orchestration + public docs. No Rust changes. No proto changes. No DB schema changes.

## 1. Module layout

```
sdk/python/src/spendguard/integrations/
├── openai_agents.py            # existing — RunContext / run_context REUSED, NOT modified
└── atomic_agents.py            # NEW — D28 (~350 LOC)

sdk/python/pyproject.toml       # +`atomic-agents` extra → atomic-agents>=1.0,<2.0, instructor>=1.5,<2.0

sdk/python/tests/integrations/
├── conftest_atomic_agents.py   # NEW — FakeInstructor + FakeAsyncInstructor fixtures
├── test_atomic_agents.py       # NEW — unit tests (~320 LOC)
└── test_atomic_agents_real.py  # NEW — integration tests with real atomic-agents + instructor + pytest-httpx (~220 LOC)

deploy/demo/atomic_agents/      # NEW
└── bootstrap.py                # env-driven SpendGuardInstructorProxy factory

deploy/demo/
├── Makefile                    # +DEMO_MODE=agent_real_atomic_agents branch
├── verify_step_atomic_agents.sql  # NEW — SQL gate
└── demo/run_demo.py            # +run_agent_real_atomic_agents_mode()

docs/site/docs/integrations/
└── atomic-agents.md            # NEW — public docs page (Instructor-wrap rationale)
```

## 2. Slice breakdown

### Slice 1 — Module skeleton + extras + factory dispatch (S)

**Files:** `sdk/python/src/spendguard/integrations/atomic_agents.py` (new), `sdk/python/pyproject.toml` (edit).

```python
# sdk/python/src/spendguard/integrations/atomic_agents.py
"""Atomic Agents (Instructor + Pydantic) adapter.

Atomic Agents constructs `BaseAgent` via `BaseAgentConfig(client=<instructor>,
...)` and at run time calls `self.client.chat.completions.create_with_completion
(response_model=..., ...)`. There is no first-class LLM-call middleware. The
only surface that observes every call (including Instructor's validation
retries) is the Instructor object itself.

SpendGuard wraps the Instructor object via composition:

  - sync   `instructor.Instructor`     → SpendGuardInstructorProxy
  - async  `instructor.AsyncInstructor`→ SpendGuardAsyncInstructorProxy

Both proxies override `.chat.completions.create` and
`.chat.completions.create_with_completion`. Other attributes pass through
via `__getattr__`.

Install::

    pip install 'spendguard-sdk[atomic-agents]'
    # transitively pulls atomic-agents>=1.0,<2.0 + instructor>=1.5,<2.0

Integration shape::

    import instructor
    from openai import OpenAI
    from atomic_agents.agents.base_agent import BaseAgent, BaseAgentConfig

    from spendguard import SpendGuardClient
    from spendguard.integrations.atomic_agents import wrap_instructor_client
    from spendguard.integrations.openai_agents import RunContext, run_context

    sg = SpendGuardClient(socket_path=..., tenant_id=...)
    await sg.connect(); await sg.handshake()

    raw = instructor.from_openai(OpenAI())
    guarded = wrap_instructor_client(
        raw, spendguard_client=sg,
        budget_id=..., window_instance_id=...,
        unit=..., pricing=...,
        claim_estimator=lambda kwargs: [common_pb2.BudgetClaim(...)],
    )

    agent = BaseAgent(BaseAgentConfig(
        client=guarded, model="gpt-4o-mini",
        system_prompt_generator=..., input_schema=..., output_schema=...,
    ))

    async with run_context(RunContext(run_id="...")):
        result = agent.run({"query": "..."})
"""

from __future__ import annotations

import hashlib
from collections.abc import Callable
from typing import Any

from ..client import DecisionOutcome, SpendGuardClient
from ..ids import derive_idempotency_key, derive_uuid_from_signature
from .openai_agents import current_run_context  # REUSED — single shared RunContext

try:
    import instructor
    from instructor import AsyncInstructor, Instructor
except ImportError as exc:  # pragma: no cover
    raise ImportError(
        "spendguard.integrations.atomic_agents requires the [atomic-agents] extra. "
        "Install with: pip install 'spendguard-sdk[atomic-agents]'."
    ) from exc

try:
    # Atomic Agents itself is not imported (we wrap the Instructor object,
    # not a BaseAgent type), but we surface a friendly hint if it's missing
    # since this adapter is named for it.
    import atomic_agents  # noqa: F401
except ImportError as exc:  # pragma: no cover
    raise ImportError(
        "spendguard.integrations.atomic_agents requires atomic-agents installed. "
        "Install with: pip install 'spendguard-sdk[atomic-agents]' "
        "(pulls atomic-agents>=1.0,<2.0)."
    ) from exc

try:
    from spendguard._proto.spendguard.common.v1 import common_pb2
except ImportError as exc:  # pragma: no cover
    raise ImportError("spendguard proto stubs missing. Run `make proto`.") from exc


ClaimEstimator = Callable[[dict[str, Any]], list[Any]]
"""Project BudgetClaim list from Instructor's create-call kwargs.

Keys present: model, messages, response_model, tools, tool_choice,
max_retries, validation_context, plus any provider-specific kwargs.
"""


def wrap_instructor_client(
    client: Instructor | AsyncInstructor,
    *,
    spendguard_client: SpendGuardClient,
    budget_id: str,
    window_instance_id: str,
    unit: Any,
    pricing: Any,
    claim_estimator: ClaimEstimator,
) -> "SpendGuardInstructorProxy | SpendGuardAsyncInstructorProxy":
    """Factory — return a sync or async proxy matching the inner Instructor type.

    Sync `instructor.Instructor` → `SpendGuardInstructorProxy`.
    Async `instructor.AsyncInstructor` → `SpendGuardAsyncInstructorProxy`.
    Anything else raises TypeError — we do NOT silently wrap raw OpenAI clients
    (that would miss Instructor's retry loop; rejected per design.md §1).
    """
    if isinstance(client, AsyncInstructor):
        return SpendGuardAsyncInstructorProxy(
            inner=client, spendguard_client=spendguard_client,
            budget_id=budget_id, window_instance_id=window_instance_id,
            unit=unit, pricing=pricing, claim_estimator=claim_estimator,
        )
    if isinstance(client, Instructor):
        return SpendGuardInstructorProxy(
            inner=client, spendguard_client=spendguard_client,
            budget_id=budget_id, window_instance_id=window_instance_id,
            unit=unit, pricing=pricing, claim_estimator=claim_estimator,
        )
    raise TypeError(
        f"wrap_instructor_client expects instructor.Instructor or "
        f"instructor.AsyncInstructor; got {type(client).__name__}. "
        f"If you have a raw provider client (e.g. openai.OpenAI), wrap it "
        f"first: instructor.from_openai(client)."
    )
```

`pyproject.toml`:

```toml
[project.optional-dependencies]
atomic-agents = [
  "atomic-agents>=1.0,<2.0",
  "instructor>=1.5,<2.0",
]
```

### Slice 2 — `create` / `create_with_completion` PRE/POST sync + async (M)

```python
def _signature(kwargs: dict[str, Any]) -> str:
    """Derive a stable signature from Instructor create-call kwargs.

    Includes model, messages, response_model identity, tools, and
    tool_choice. Each Instructor validation retry mutates `messages`
    (validation error injected), so the signature naturally diverges
    across retries — yielding a fresh llm_call_id per attempt.
    """
    response_model = kwargs.get("response_model")
    rm_repr = (
        f"{response_model.__module__}.{response_model.__qualname__}"
        if response_model is not None else ""
    )
    text = (
        f"model={kwargs.get('model')!r}|"
        f"messages={kwargs.get('messages')!r}|"
        f"response_model={rm_repr}|"
        f"tools={kwargs.get('tools')!r}|"
        f"tool_choice={kwargs.get('tool_choice')!r}"
    )
    return hashlib.blake2b(text.encode("utf-8"), digest_size=16).hexdigest()


def _extract_total_tokens(raw_completion: Any) -> int:
    """Read usage.total_tokens from a ChatCompletion (or parsed-model
    `_raw_response`). 0 when absent."""
    usage = getattr(raw_completion, "usage", None)
    if usage is None:
        return 0
    total = getattr(usage, "total_tokens", None)
    if isinstance(total, int):
        return total
    prompt = getattr(usage, "prompt_tokens", 0) or 0
    completion = getattr(usage, "completion_tokens", 0) or 0
    return int(prompt) + int(completion)


def _extract_provider_event_id(raw_completion: Any) -> str:
    return str(getattr(raw_completion, "id", "") or "")


class _ChatCompletionsNamespace:
    """Mirrors `Instructor.chat.completions.{create,create_with_completion}`.
    Holds a back-reference to the proxy so we can gate around the inner call.
    """
    def __init__(self, proxy: "_ProxyBase") -> None:
        self._proxy = proxy


class _SyncChatCompletionsNamespace(_ChatCompletionsNamespace):
    def create(self, **kwargs: Any) -> Any:
        return self._proxy._sync_gated_call(
            method_name="create", **kwargs,
        )

    def create_with_completion(self, **kwargs: Any) -> Any:
        return self._proxy._sync_gated_call(
            method_name="create_with_completion", **kwargs,
        )


class _ChatNamespace:
    def __init__(self, completions: _ChatCompletionsNamespace) -> None:
        self.completions = completions


class _ProxyBase:
    def __init__(
        self,
        *,
        inner: Instructor | AsyncInstructor,
        spendguard_client: SpendGuardClient,
        budget_id: str,
        window_instance_id: str,
        unit: Any,
        pricing: Any,
        claim_estimator: ClaimEstimator,
    ) -> None:
        self._inner = inner
        self._client = spendguard_client
        self._budget_id = budget_id
        self._window_instance_id = window_instance_id
        self._unit = unit
        self._pricing = pricing
        self._claim_estimator = claim_estimator

    def __getattr__(self, name: str) -> Any:
        # Delegate unknown attributes to inner Instructor.
        # Only fires on miss — our explicit attrs shadow correctly.
        return getattr(self._inner, name)


class SpendGuardInstructorProxy(_ProxyBase):
    """Sync proxy. `agent.run(...)` synchronously calls
    `client.chat.completions.create_with_completion(...)`."""

    def __init__(self, **kwargs: Any) -> None:
        super().__init__(**kwargs)
        self.chat = _ChatNamespace(_SyncChatCompletionsNamespace(self))

    def _sync_gated_call(self, *, method_name: str, **kwargs: Any) -> Any:
        ctx = current_run_context()
        signature = _signature(kwargs)
        llm_call_id = str(derive_uuid_from_signature(signature, scope="llm_call_id"))
        decision_id = str(derive_uuid_from_signature(signature, scope="decision_id"))
        step_id = f"{ctx.run_id}:atomic-agents:{signature[:16]}"
        idempotency_key = derive_idempotency_key(
            tenant_id=self._client.tenant_id,
            session_id=self._client.session_id,
            run_id=ctx.run_id, step_id=step_id, llm_call_id=llm_call_id,
            trigger="LLM_CALL_PRE",
        )

        outcome: DecisionOutcome = self._client.request_decision_sync(
            trigger="LLM_CALL_PRE",
            run_id=ctx.run_id, step_id=step_id, llm_call_id=llm_call_id,
            tool_call_id="", decision_id=decision_id, route="llm.call",
            projected_claims=self._claim_estimator(kwargs),
            idempotency_key=idempotency_key,
        )

        inner_method = getattr(self._inner.chat.completions, method_name)
        try:
            result = inner_method(**kwargs)
        except BaseException as exc:
            if outcome.reservation_ids:
                outcome_kind = "CANCELLED" if type(exc).__name__ == "CancelledError" else "FAILURE"
                self._client.emit_llm_call_post_sync(
                    run_id=ctx.run_id, step_id=step_id, llm_call_id=llm_call_id,
                    decision_id=outcome.decision_id,
                    reservation_id=outcome.reservation_ids[0],
                    provider_reported_amount_atomic="",
                    estimated_amount_atomic="0",
                    unit=self._unit, pricing=self._pricing,
                    provider_event_id="", outcome=outcome_kind,
                )
            raise

        # `create_with_completion` returns (parsed, raw_completion).
        # `create` returns parsed; raw is on `parsed._raw_response`.
        if method_name == "create_with_completion":
            _parsed, raw_completion = result
        else:
            raw_completion = getattr(result, "_raw_response", None)

        if outcome.reservation_ids:
            self._client.emit_llm_call_post_sync(
                run_id=ctx.run_id, step_id=step_id, llm_call_id=llm_call_id,
                decision_id=outcome.decision_id,
                reservation_id=outcome.reservation_ids[0],
                provider_reported_amount_atomic="",
                estimated_amount_atomic=str(_extract_total_tokens(raw_completion)),
                unit=self._unit, pricing=self._pricing,
                provider_event_id=_extract_provider_event_id(raw_completion),
                outcome="SUCCESS",
            )
        return result


class _AsyncChatCompletionsNamespace(_ChatCompletionsNamespace):
    async def create(self, **kwargs: Any) -> Any:
        return await self._proxy._async_gated_call(
            method_name="create", **kwargs,
        )

    async def create_with_completion(self, **kwargs: Any) -> Any:
        return await self._proxy._async_gated_call(
            method_name="create_with_completion", **kwargs,
        )


class SpendGuardAsyncInstructorProxy(_ProxyBase):
    """Async proxy. `agent.run_async(...)` awaits create_with_completion."""

    def __init__(self, **kwargs: Any) -> None:
        super().__init__(**kwargs)
        self.chat = _ChatNamespace(_AsyncChatCompletionsNamespace(self))

    async def _async_gated_call(self, *, method_name: str, **kwargs: Any) -> Any:
        ctx = current_run_context()
        signature = _signature(kwargs)
        llm_call_id = str(derive_uuid_from_signature(signature, scope="llm_call_id"))
        decision_id = str(derive_uuid_from_signature(signature, scope="decision_id"))
        step_id = f"{ctx.run_id}:atomic-agents:{signature[:16]}"
        idempotency_key = derive_idempotency_key(
            tenant_id=self._client.tenant_id,
            session_id=self._client.session_id,
            run_id=ctx.run_id, step_id=step_id, llm_call_id=llm_call_id,
            trigger="LLM_CALL_PRE",
        )

        outcome: DecisionOutcome = await self._client.request_decision(
            trigger="LLM_CALL_PRE",
            run_id=ctx.run_id, step_id=step_id, llm_call_id=llm_call_id,
            tool_call_id="", decision_id=decision_id, route="llm.call",
            projected_claims=self._claim_estimator(kwargs),
            idempotency_key=idempotency_key,
        )

        inner_method = getattr(self._inner.chat.completions, method_name)
        try:
            result = await inner_method(**kwargs)
        except BaseException as exc:
            if outcome.reservation_ids:
                outcome_kind = "CANCELLED" if type(exc).__name__ == "CancelledError" else "FAILURE"
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

        if method_name == "create_with_completion":
            _parsed, raw_completion = result
        else:
            raw_completion = getattr(result, "_raw_response", None)

        if outcome.reservation_ids:
            await self._client.emit_llm_call_post(
                run_id=ctx.run_id, step_id=step_id, llm_call_id=llm_call_id,
                decision_id=outcome.decision_id,
                reservation_id=outcome.reservation_ids[0],
                provider_reported_amount_atomic="",
                estimated_amount_atomic=str(_extract_total_tokens(raw_completion)),
                unit=self._unit, pricing=self._pricing,
                provider_event_id=_extract_provider_event_id(raw_completion),
                outcome="SUCCESS",
            )
        return result


__all__ = [
    "ClaimEstimator",
    "SpendGuardAsyncInstructorProxy",
    "SpendGuardInstructorProxy",
    "wrap_instructor_client",
]
```

### Slice 3 — Tests

See `tests.md` §2.

### Slice 4 — Demo + docs

`Makefile` additions:

```make
else ifeq ($(DEMO_MODE),agent_real_atomic_agents)
	@echo "[demo] DEMO_MODE=agent_real_atomic_agents → Atomic Agents BaseAgent wrapped via SpendGuardInstructorProxy"
	$(MAKE) up_minimal
	docker compose run --rm \
	    --env SPENDGUARD_DEMO_MODE=agent_real_atomic_agents \
	    demo python /workspace/run_demo.py
	$(MAKE) verify_atomic_agents
```

`verify_step_atomic_agents.sql` asserts at least one `audit_outbox` row with `route='llm.call'`, `trigger='LLM_CALL_PRE'`, and a paired `LLM_CALL_POST` with non-zero `estimated_amount_atomic` within 60s. Also asserts at least one `LLM_CALL_PRE` with `decision='DENY'` and NO paired POST.

## 3. Public docs page

`docs/site/docs/integrations/atomic-agents.md` — single page with:

1. **Why we wrap the Instructor object, not the raw provider SDK.** Short table reproducing `design.md` §1 with the rejected raw-SDK row explained: Instructor's validation retries re-enter the patched method, never the raw transport, so a raw-SDK wrap silently undercounts.
2. **Working `wrap_instructor_client(...)` code block** mirroring `design.md` §7.
3. **`BaseAgent` + Pydantic `output_schema`** example with structured output.
4. **Polyglot trace example** sharing `RunContext` with `spendguard.integrations.openai_agents`.
5. **Provider-routing note.** Operator must pick a `claim_estimator` matching the inner Instructor's provider (OpenAI / Anthropic / Gemini / Cohere) — `spendguard.integrations.openai_agents._default_estimator` covers the OpenAI case.
6. **Async pointer.** When using `instructor.from_openai(AsyncOpenAI(...))`, the factory returns `SpendGuardAsyncInstructorProxy` automatically; `agent.run_async(...)` works unchanged.
