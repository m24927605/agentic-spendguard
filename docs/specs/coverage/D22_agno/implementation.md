# D22 — Implementation

## 1. Module layout

```
sdk/python/
├── pyproject.toml                                   # +[agno] extra
└── src/spendguard/integrations/
    ├── agno.py                                      # NEW
    └── _default_estimator.py                        # +agno_default_claim_estimator
examples/agno-prehooks/
├── run.py                                           # NEW (demo entrypoint)
└── README.md                                        # NEW
deploy/demo/demo/run_demo.py                         # +agent_real_agno branch
docs/site/docs/integrations/agno.md                  # NEW
README.md                                            # +adapter row
sdk/python/tests/integrations/
├── test_agno_pre_post.py                            # NEW (unit + integration)
└── test_agno_default_estimator.py                   # NEW
```

## 2. `pyproject.toml` extra

```toml
[project.optional-dependencies]
agno = [
  "agno>=1.0,<2.0",
]
```

Inserted alphabetically between `agt` and `langchain`. No transitive provider SDK pin — Agno deliberately leaves vendor SDKs to the user.

## 3. `integrations/agno.py` skeleton

```python
"""Agno pre_hooks / post_hooks integration — gates Agent.arun() via the sidecar.

Wrap an Agno `Agent` with `SpendGuardAgnoPreHook` + `SpendGuardAgnoPostHook` to
route every LLM call through the SpendGuard sidecar's
RequestDecision → CommitEstimated lifecycle. Works across every Agno
`Model` subclass (`OpenAIChat`, `Claude`, `Gemini`, `Groq`, ...) because
the hook surface is callable-based — there is no model subclassing.

POC scope:
  - Streaming (`Agent.arun(stream=True)`) is gated at PRE only; POST emits
    after the final chunk.
  - DEGRADE mutation patches are surfaced as `MutationApplyFailed` rather than
    applied (parity with pydantic_ai / langchain integrations).
  - Tool-call hooks (`tool_hooks`) are NOT covered by D22; future deliverable.
"""

from __future__ import annotations

import contextvars
import hashlib
import logging
from collections import OrderedDict
from collections.abc import Callable
from contextlib import asynccontextmanager
from dataclasses import dataclass
from typing import Any, AsyncIterator

from ..client import DecisionOutcome, SpendGuardClient
from ..errors import DecisionDenied
from ..ids import (
    derive_idempotency_key,
    derive_uuid_from_signature,
    new_uuid7,
)

try:
    from agno.agent import Agent  # noqa: F401
    from agno.run.response import RunResponse  # path stable across 1.x
except ImportError as exc:  # pragma: no cover
    raise ImportError(
        "spendguard.integrations.agno requires the [agno] extra. "
        "Install with: pip install 'spendguard-sdk[agno]'"
    ) from exc

try:
    from spendguard._proto.spendguard.common.v1 import common_pb2
except ImportError as exc:  # pragma: no cover
    raise ImportError(
        "spendguard proto stubs missing. Run `make proto` first."
    ) from exc

logger = logging.getLogger(__name__)

# Shared with langchain / pydantic_ai / openai_agents — multi-framework
# agents reuse one run_id.
_RUN_CONTEXT: contextvars.ContextVar["RunContext | None"] = contextvars.ContextVar(
    "spendguard_run_context", default=None
)

# Bound the inflight map. 10k entries × ~200B == ~2MB ceiling. FIFO eviction
# matches D04 §5. Concurrent access is guarded by the asyncio scheduler:
# pre/post hooks run on the same event loop as Agent.arun().
_INFLIGHT_MAX = 10_000


@dataclass(frozen=True, slots=True)
class RunContext:
    """Per Agent.arun() identifiers."""

    run_id: str


@asynccontextmanager
async def run_context(ctx: RunContext) -> AsyncIterator[RunContext]:
    token = _RUN_CONTEXT.set(ctx)
    try:
        yield ctx
    finally:
        _RUN_CONTEXT.reset(token)


def current_run_context() -> RunContext:
    ctx = _RUN_CONTEXT.get()
    if ctx is None:
        raise RuntimeError(
            "spendguard.integrations.agno hook fired outside an active "
            "run_context(). Wrap your Agent.arun call:\n\n"
            "    async with run_context(RunContext(run_id=...)):\n"
            "        await agent.arun(prompt)\n"
        )
    return ctx


ClaimEstimator = Callable[[Any, Any], list[Any]]
"""(agent, run_input) → list[BudgetClaim]. ``run_input`` is whatever the
caller passed to ``agent.arun(...)``: str | list[Message] | dict."""

CallSignatureFn = Callable[[Any, Any], str]
"""(agent, run_input) → 32-hex content signature, hashed for idempotency."""


@dataclass(slots=True)
class _InflightReservation:
    signature: str
    reservation_ids: list[str]
    decision_id: str
    llm_call_id: str
    step_id: str
    unit: Any
    pricing: Any


def _default_call_signature(agent: Any, run_input: Any) -> str:
    """Hash (model.id || visible run_input) into a 32-char hex digest.

    blake2b-16 matches the LangChain integration's signature width so
    downstream ID derivation is symmetric.
    """
    model_id = getattr(getattr(agent, "model", None), "id", "") or ""
    payload = f"{model_id}\n{run_input!s}"
    return hashlib.blake2b(payload.encode("utf-8"), digest_size=16).hexdigest()


class SpendGuardAgnoPreHook:
    """Factory: returns a callable suitable for ``Agent(pre_hooks=[...])``."""

    def __init__(
        self,
        *,
        client: SpendGuardClient,
        budget_id: str,
        window_instance_id: str,
        unit: Any,
        pricing: Any,
        claim_estimator: ClaimEstimator | None = None,
        call_signature_fn: CallSignatureFn | None = None,
        route: str = "llm.call",
        inflight: "OrderedDict[tuple[str, str], _InflightReservation] | None" = None,
    ) -> None:
        self._client = client
        self._budget_id = budget_id
        self._window_instance_id = window_instance_id
        self._unit = unit
        self._pricing = pricing
        self._claim_estimator = claim_estimator
        self._call_signature_fn = call_signature_fn or _default_call_signature
        self._route = route
        # Shared with the paired post-hook via constructor or via the
        # module-level slot below.
        self._inflight = inflight if inflight is not None else _SHARED_INFLIGHT

    def __call__(self) -> Callable[..., None]:
        if self._claim_estimator is None:
            from ._default_estimator import agno_default_claim_estimator
            self._claim_estimator = agno_default_claim_estimator(
                budget_id=self._budget_id,
                window_instance_id=self._window_instance_id,
                unit=self._unit,
                model="",  # resolved per-call from agent.model.id
            )

        async def _pre_hook(agent: Any, run_input: Any) -> None:
            ctx = current_run_context()
            signature = self._call_signature_fn(agent, run_input)
            llm_call_id = str(derive_uuid_from_signature(signature, scope="llm_call_id"))
            decision_id = str(derive_uuid_from_signature(signature, scope="decision_id"))
            step_id = f"{ctx.run_id}:agno-call:{signature[:16]}"
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
                route=self._route,
                projected_claims=self._claim_estimator(agent, run_input),
                idempotency_key=idempotency_key,
            )
            # raises DecisionDenied / DecisionStopped on STOP — Agno
            # propagates the exception out of Agent.arun().

            key = (ctx.run_id, signature)
            self._inflight[key] = _InflightReservation(
                signature=signature,
                reservation_ids=list(outcome.reservation_ids),
                decision_id=outcome.decision_id,
                llm_call_id=llm_call_id,
                step_id=step_id,
                unit=self._unit,
                pricing=self._pricing,
            )
            # Bounded FIFO eviction.
            while len(self._inflight) > _INFLIGHT_MAX:
                self._inflight.popitem(last=False)

        # Mirror Agno's signature-injection expectations: keep the
        # parameter NAMES (agent, run_input) — Agno reads via inspect.
        return _pre_hook


class SpendGuardAgnoPostHook:
    """Factory: returns a callable suitable for ``Agent(post_hooks=[...])``."""

    def __init__(
        self,
        *,
        client: SpendGuardClient,
        unit: Any,
        pricing: Any,
        call_signature_fn: CallSignatureFn | None = None,
        inflight: "OrderedDict[tuple[str, str], _InflightReservation] | None" = None,
    ) -> None:
        self._client = client
        self._unit = unit
        self._pricing = pricing
        self._call_signature_fn = call_signature_fn or _default_call_signature
        self._inflight = inflight if inflight is not None else _SHARED_INFLIGHT

    def __call__(self) -> Callable[..., None]:
        async def _post_hook(agent: Any, run_response: Any) -> None:
            ctx = current_run_context()
            signature = self._call_signature_fn(agent, getattr(run_response, "input", None) or "")
            slot = self._inflight.pop((ctx.run_id, signature), None)
            if slot is None:
                # No matching reservation — either pre-hook never fired
                # (user instrumentation bug) or two post-hooks fired for
                # one pre. Log once and no-op rather than emit a
                # commit-without-reserve event.
                logger.warning(
                    "spendguard.agno: post_hook fired without matching pre "
                    "reservation (run_id=%s sig=%s)",
                    ctx.run_id, signature[:8],
                )
                return
            if not slot.reservation_ids:
                return  # SKIPPED / no reservation issued.

            total_tokens, provider_event_id, outcome = _extract_usage(run_response)
            await self._client.emit_llm_call_post(
                run_id=ctx.run_id,
                step_id=slot.step_id,
                llm_call_id=slot.llm_call_id,
                decision_id=slot.decision_id,
                reservation_id=slot.reservation_ids[0],
                provider_reported_amount_atomic="",
                estimated_amount_atomic=str(total_tokens),
                unit=slot.unit,
                pricing=slot.pricing,
                provider_event_id=provider_event_id,
                outcome=outcome,
            )

        return _post_hook


# Module-shared inflight map. Same process, all paired hooks see it.
_SHARED_INFLIGHT: "OrderedDict[tuple[str, str], _InflightReservation]" = OrderedDict()


def _extract_usage(run_response: Any) -> tuple[int, str, str]:
    """Return (total_tokens, provider_event_id, outcome).

    Agno's RunResponse exposes ``metrics`` (token counts) and ``event``
    (status). When event == "RunError" we report PROVIDER_ERROR so the
    projector releases the reservation; success path reports SUCCESS
    with whatever total_tokens metric the provider returned.
    """
    if run_response is None:
        return 0, "", "PROVIDER_ERROR"
    event = getattr(run_response, "event", "") or ""
    if event == "RunError" or getattr(run_response, "error", None):
        return 0, "", "PROVIDER_ERROR"

    metrics = getattr(run_response, "metrics", None) or {}
    total = 0
    if isinstance(metrics, dict):
        for key in ("total_tokens", "input_tokens", "output_tokens"):
            v = metrics.get(key)
            if isinstance(v, list) and v:
                v = v[0]
            if isinstance(v, (int, float)):
                if key == "total_tokens":
                    total = int(v); break
                total += int(v)

    provider_event_id = ""
    rid = getattr(run_response, "run_id", None) or getattr(run_response, "response_id", "")
    if isinstance(rid, str):
        provider_event_id = rid

    return total, provider_event_id, "SUCCESS"


__all__ = [
    "ClaimEstimator",
    "CallSignatureFn",
    "RunContext",
    "SpendGuardAgnoPreHook",
    "SpendGuardAgnoPostHook",
    "current_run_context",
    "run_context",
]
```

## 4. `_default_estimator.py` extension

```python
def agno_default_claim_estimator(
    *,
    budget_id: str,
    window_instance_id: str,
    unit: Any,
    model: str,
) -> Callable[[Any, Any], list[Any]]:
    """Agno ``(agent, run_input) → list[BudgetClaim]``.

    Resolves the model family from ``agent.model.id`` at call time, so a
    single hook instance can serve multi-model `Team` agents. When
    ``run_input`` is a list of messages we forward it directly; when it
    is a str we wrap as a single user message; otherwise we stringify.
    """
    def estimator(agent: Any, run_input: Any) -> list[Any]:
        agno_model = getattr(getattr(agent, "model", None), "id", "") or model or ""
        fns = estimator_for_model(agno_model)
        if isinstance(run_input, str):
            messages = [{"role": "user", "content": run_input}]
        elif isinstance(run_input, list):
            messages = run_input
        else:
            messages = [{"role": "user", "content": str(run_input)}]
        input_tokens = fns.count_input_tokens(messages, agno_model)
        output_tokens = fns.count_output_tokens_max(None, agno_model)
        return [
            _build_claim(
                budget_id=budget_id,
                window_instance_id=window_instance_id,
                unit=unit,
                amount_atomic=input_tokens + output_tokens,
            )
        ]
    return estimator
```

## 5. Demo wiring

`examples/agno-prehooks/run.py` constructs `SpendGuardClient` against `SPENDGUARD_SOCKET_PATH`, builds `Agent(model=OpenAIChat(id="gpt-4o-mini"), pre_hooks=[pre()], post_hooks=[post()])`, and runs three flows under `run_context(...)`: (1) success path, (2) DENY budget pre-seeded so the reserve raises before OpenAI is called, (3) PROVIDER_ERROR path where the OpenAI client is patched to raise — confirms `outcome=PROVIDER_ERROR` lands in `commit_estimated`.

`deploy/demo/demo/run_demo.py` adds `if DEMO_MODE == "agent_real_agno":` mirroring the `agent_real_langchain` branch — same `OPENAI_API_KEY` requirement, same exit-code semantics.

## 6. Hook-callable shape note

Agno invokes pre/post hooks via `inspect.signature` introspection — the parameter **names** matter. The closure returns an async function literally named `_pre_hook(agent, run_input)` / `_post_hook(agent, run_response)`. Slice 3 tests assert these names persist after closure wrapping (`functools.wraps` is intentionally NOT used because Agno disambiguates by name, not by `__wrapped__`).
