# D23 — BeeAI Framework `Emitter` adapter — Implementation

## 1. Module layout

```
sdk/python/src/spendguard/integrations/
├── beeai.py                # NEW — public surface
├── _beeai_inflight.py      # NEW (slice 2) — bounded inflight map
└── langchain.py            # UNCHANGED reference

sdk/python/tests/integrations/
├── test_beeai_skeleton.py        # NEW (slice 1)
├── test_beeai_subscribe.py       # NEW (slice 2)
├── test_beeai_reserve_commit.py  # NEW (slice 3)
└── test_beeai_default_e2e.py     # NEW (slice 3) — mirrors test_langchain_default_e2e.py

sdk/python/pyproject.toml         # +1 extras entry: `beeai = ["beeai-framework>=0.3,<1.0"]`

deploy/demo/Makefile              # +2 arms: agent_real_beeai, agent_real_beeai_deny
deploy/demo/demo/run_demo.py      # +run_beeai_mode() + dispatcher branches
deploy/demo/demo/verify_beeai.sql # NEW — outbox / ledger post-call assertions

docs/site/docs/integrations/beeai.md  # NEW
README.md                              # +1 adapter table row
```

## 2. Public surface — `sdk/python/src/spendguard/integrations/beeai.py`

```python
"""BeeAI Framework (>=0.3) integration — gates ChatModel calls via the sidecar.

Subscribes a single async handler to the agent's Emitter that intercepts
`*.llm.*.start` / `*.llm.*.success` / `*.llm.*.error` and routes them
through SpendGuard's RequestDecision → CommitEstimated lifecycle.

Integration shape::

    from beeai_framework.agents.react import ReActAgent
    from beeai_framework.backend import OpenAIChatModel
    from spendguard import SpendGuardClient, new_uuid7
    from spendguard.integrations.beeai import (
        subscribe_spendguard, run_context, RunContext,
    )

    agent = ReActAgent(llm=OpenAIChatModel("gpt-4o-mini"), tools=[...])
    unsubscribe = subscribe_spendguard(
        agent, client,
        budget_id=..., window_instance_id=..., unit=..., pricing=...,
    )

    async with run_context(RunContext(run_id=str(new_uuid7()))):
        result = await agent.run("Say hello in three words.").result
    unsubscribe()
"""

from __future__ import annotations

import logging
from collections.abc import Callable, Sequence
from dataclasses import dataclass
from typing import Any

from ..client import DecisionOutcome, SpendGuardClient
from ..errors import DecisionDenied
from ..ids import (
    derive_idempotency_key,
    derive_uuid_from_signature,
)
# Re-use the LangChain adapter's contextvar so a single run_context covers both.
from .langchain import (
    RunContext,
    _default_call_signature,
    current_run_context,
    run_context,
)
from ._beeai_inflight import InflightMap, InflightEntry

try:
    from beeai_framework.agents.base import BaseAgent
    from beeai_framework.emitter.emitter import Emitter, EventMeta
except ImportError as exc:  # pragma: no cover
    raise ImportError(
        "spendguard.integrations.beeai requires the [beeai] extra. "
        "Install with: pip install 'spendguard-sdk[beeai]'"
    ) from exc

try:
    from spendguard._proto.spendguard.common.v1 import common_pb2
except ImportError as exc:  # pragma: no cover
    raise ImportError(
        "spendguard proto stubs missing. Run `make proto` first."
    ) from exc


_LOG = logging.getLogger(__name__)

ClaimEstimator = Callable[["BeeAiStartEvent"], list[Any]]
CallSignatureFn = Callable[["BeeAiStartEvent"], str]


@dataclass(frozen=True, slots=True)
class BeeAiStartEvent:
    """Normalised view of a BeeAI `start` event."""

    input: Sequence[Any]
    model_id: str
    path: str  # EventMeta.path; full hierarchical path including trailing `.start`


def _stable_call_key(path: str) -> str:
    """Strip the trailing `.start|.success|.error` segment.

    BeeAI emits one event per name on the same hierarchical path —
    `agent.react.llm.<uuid>.start`, then `.success`. Stripping the
    last segment gives the stable per-call correlation key.
    """
    return path.rsplit(".", 1)[0]


def subscribe_spendguard(
    agent: BaseAgent,
    client: SpendGuardClient,
    *,
    budget_id: str,
    window_instance_id: str,
    unit: Any,                 # common_pb2.UnitRef
    pricing: Any,              # common_pb2.PricingFreeze
    claim_estimator: ClaimEstimator | None = None,
    call_signature_fn: CallSignatureFn | None = None,
    route: str = "llm.call",
) -> Callable[[], None]:
    """Install a SpendGuard subscriber on `agent.emitter`.

    Returns a no-arg `unsubscribe()` callable. Idempotent only in that
    a second `subscribe_spendguard` on the same agent installs a second
    subscriber — callers must hold + invoke the returned unsubscribe.
    """
    inflight = InflightMap(capacity=10_000)
    sig_fn = call_signature_fn or (lambda ev: _default_call_signature(ev.input))

    if claim_estimator is None:
        from ._default_estimator import langchain_default_claim_estimator
        # The default estimator is model-name dispatched; for BeeAI we read
        # ev.model_id rather than the inner-model attr cascade. Wrap so the
        # estimator interface matches BeeAiStartEvent.
        def _resolve(ev: BeeAiStartEvent):  # type: ignore[no-redef]
            inner = langchain_default_claim_estimator(
                budget_id=budget_id,
                window_instance_id=window_instance_id,
                unit=unit,
                model=ev.model_id,
            )
            return inner(ev.input)
        estimator: ClaimEstimator = _resolve
    else:
        estimator = claim_estimator

    async def _handle(data: Any, meta: EventMeta) -> None:
        ev_name = meta.name
        if ev_name == "start":
            await _on_start(data, meta)
        elif ev_name == "success":
            await _on_success(data, meta)
        elif ev_name == "error":
            await _on_error(data, meta)
        # silently ignore newToken / partialUpdate etc.

    async def _on_start(data: Any, meta: EventMeta) -> None:
        ctx = current_run_context()
        start_ev = BeeAiStartEvent(
            input=getattr(data, "input", None) or getattr(data, "messages", None) or [],
            model_id=getattr(data, "modelId", None) or getattr(data, "model_id", "") or "",
            path=meta.path,
        )
        call_key = _stable_call_key(meta.path)
        signature = sig_fn(start_ev)
        llm_call_id = str(derive_uuid_from_signature(signature, scope="llm_call_id"))
        decision_id = str(derive_uuid_from_signature(signature, scope="decision_id"))
        step_id = f"{ctx.run_id}:beeai:{call_key}"
        idem = derive_idempotency_key(
            tenant_id=client.tenant_id,
            session_id=client.session_id,
            run_id=ctx.run_id,
            step_id=step_id,
            llm_call_id=llm_call_id,
            trigger="LLM_CALL_PRE",
        )
        outcome: DecisionOutcome = await client.request_decision(
            trigger="LLM_CALL_PRE",
            run_id=ctx.run_id,
            step_id=step_id,
            llm_call_id=llm_call_id,
            tool_call_id="",
            decision_id=decision_id,
            route=route,
            projected_claims=estimator(start_ev),
            idempotency_key=idem,
        )
        # request_decision raises DecisionDenied on DENY; reaching here = ALLOW or DEGRADE.
        if outcome.reservation_ids:
            inflight.put(call_key, InflightEntry(
                reservation_id=outcome.reservation_ids[0],
                decision_id=outcome.decision_id,
                llm_call_id=llm_call_id,
                step_id=step_id,
                run_id=ctx.run_id,
            ))

    async def _on_success(data: Any, meta: EventMeta) -> None:
        entry = inflight.pop(_stable_call_key(meta.path))
        if entry is None:
            return
        usage = getattr(data, "usage", None) or {}
        total = int(usage.get("total_tokens", 0)) if isinstance(usage, dict) else 0
        provider_event_id = getattr(data, "id", "") or getattr(data, "response_id", "") or ""
        await client.emit_llm_call_post(
            run_id=entry.run_id,
            step_id=entry.step_id,
            llm_call_id=entry.llm_call_id,
            decision_id=entry.decision_id,
            reservation_id=entry.reservation_id,
            provider_reported_amount_atomic="",
            estimated_amount_atomic=str(total),
            unit=unit,
            pricing=pricing,
            provider_event_id=str(provider_event_id),
            outcome="SUCCESS",
        )

    async def _on_error(data: Any, meta: EventMeta) -> None:
        entry = inflight.pop(_stable_call_key(meta.path))
        if entry is None:
            return
        await client.emit_llm_call_post(
            run_id=entry.run_id,
            step_id=entry.step_id,
            llm_call_id=entry.llm_call_id,
            decision_id=entry.decision_id,
            reservation_id=entry.reservation_id,
            provider_reported_amount_atomic="",
            estimated_amount_atomic="0",
            unit=unit,
            pricing=pricing,
            provider_event_id="",
            outcome="PROVIDER_ERROR",
        )

    # `match` predicate: any event named start/success/error whose path
    # contains an `llm` segment. Covers ReActAgent + Workflow + custom agents.
    def _predicate(event: EventMeta) -> bool:
        if event.name not in ("start", "success", "error"):
            return False
        return "llm" in event.path.split(".")

    # Emitter.match returns the unsubscribe callable in 0.3.x.
    unsubscribe = agent.emitter.match(_predicate, _handle)
    return unsubscribe


__all__ = [
    "BeeAiStartEvent",
    "ClaimEstimator",
    "CallSignatureFn",
    "RunContext",
    "run_context",
    "subscribe_spendguard",
]
```

## 3. Inflight map — `sdk/python/src/spendguard/integrations/_beeai_inflight.py`

```python
"""Bounded FIFO map keyed by stable BeeAI call path.

10 000 entry cap; oldest entry evicted on overflow with one-shot warning.
Not thread-safe — BeeAI Emitter dispatches sequentially on the asyncio loop,
which is the only concurrency contract Emitter offers in 0.3.
"""
from __future__ import annotations

import logging
from collections import OrderedDict
from dataclasses import dataclass

_LOG = logging.getLogger(__name__)


@dataclass(slots=True, frozen=True)
class InflightEntry:
    reservation_id: str
    decision_id: str
    llm_call_id: str
    step_id: str
    run_id: str


class InflightMap:
    __slots__ = ("_map", "_capacity", "_warned")

    def __init__(self, capacity: int) -> None:
        self._map: OrderedDict[str, InflightEntry] = OrderedDict()
        self._capacity = capacity
        self._warned = False

    def put(self, key: str, entry: InflightEntry) -> None:
        if key in self._map:
            self._map.pop(key)
        self._map[key] = entry
        if len(self._map) > self._capacity:
            evicted_key, _ = self._map.popitem(last=False)
            if not self._warned:
                _LOG.warning(
                    "spendguard.integrations.beeai inflight map at capacity "
                    "%d; FIFO-evicting %s. This usually means a BeeAI "
                    "`success`/`error` event was never emitted for an earlier "
                    "call — reservations for evicted entries will TTL-sweep.",
                    self._capacity, evicted_key,
                )
                self._warned = True

    def pop(self, key: str) -> InflightEntry | None:
        return self._map.pop(key, None)

    def __len__(self) -> int:
        return len(self._map)
```

## 4. `pyproject.toml` extras

Add directly after the `agt` block:

```toml
beeai = [
  "beeai-framework>=0.3,<1.0",
]
```

`spendguard-sdk[beeai]` then pulls the framework as a peer dep. No transitive provider SDK (BeeAI agents bring their own `OpenAIChatModel` / `WatsonxChatModel`).

## 5. Demo wiring — slice 4

### 5.1 `deploy/demo/Makefile`

```makefile
else ifeq ($(DEMO_MODE),agent_real_beeai)
	@echo "[demo] DEMO_MODE=agent_real_beeai → ReActAgent + OpenAIChatModel + SpendGuard subscriber"
	$(MAKE) demo-base-up
	$(MAKE) -C ../../sdk/python install-extra EXTRA=beeai
	$(MAKE) demo-run SPENDGUARD_DEMO_MODE=agent_real_beeai
else ifeq ($(DEMO_MODE),agent_real_beeai_deny)
	@echo "[demo] DEMO_MODE=agent_real_beeai_deny → hard-cap contract makes reserve raise DecisionDenied"
	$(MAKE) demo-base-up SPENDGUARD_HARD_CAP_ATOMIC=1
	$(MAKE) -C ../../sdk/python install-extra EXTRA=beeai
	$(MAKE) demo-run SPENDGUARD_DEMO_MODE=agent_real_beeai_deny
```

### 5.2 `deploy/demo/demo/run_demo.py`

Append to the dispatcher:

```python
    if DEMO_MODE == "agent_real_beeai":
        return await run_beeai_mode(deny=False)
    if DEMO_MODE == "agent_real_beeai_deny":
        return await run_beeai_mode(deny=True)
```

Plus `async def run_beeai_mode(deny: bool) -> int:` that mirrors `run_langchain_mode` but uses `ReActAgent(llm=OpenAIChatModel("gpt-4o-mini"), tools=[])` + `subscribe_spendguard(agent, client, ...)`. The `deny` path expects `await agent.run("...")` to raise `DecisionDenied` and asserts the upstream HTTP counter stayed flat (via `verify_beeai.sql`).

### 5.3 `deploy/demo/demo/verify_beeai.sql`

Asserts (a) `decision_outbox` has exactly one PRE row keyed by the run_id with `decision = 'ALLOW'` (or `'DENY'` for deny mode) and (b) for ALLOW, exactly one matching POST row with `outcome = 'SUCCESS'`.

## 6. Docs page — slice 4

`docs/site/docs/integrations/beeai.md` mirrors the LangChain integration page: install, drop-in snippet, claim-estimator note, `run_context` rule, troubleshooting (ImportError, missing run_context, DEGRADE behaviour). README adapter table gains a `BeeAI Framework (Python)` row pointing at the new page.

## 7. Slice → file mapping

| Slice | Files touched | LOC budget |
|-------|---------------|-----------|
| S1 | `beeai.py` skeleton (signature only), `pyproject.toml`, `test_beeai_skeleton.py`, `test_beeai_missing_extra.py` | ~150 |
| S2 | `_beeai_inflight.py`, `beeai.py` (subscribe wiring), `test_beeai_subscribe.py` | ~300 |
| S3 | `beeai.py` (start/success/error handlers), `test_beeai_reserve_commit.py`, `test_beeai_default_e2e.py` | ~400 |
| S4 | `Makefile`, `run_demo.py`, `verify_beeai.sql`, `beeai.md`, `README.md` | ~250 |
