# D21 — Implementation

**Reads:** [`design.md`](design.md), [`acceptance.md`](acceptance.md), [`review-standards.md`](review-standards.md).
**Touches:** Python SDK + demo orchestration + public docs. No Rust changes. No proto changes. No DB schema changes.

## 1. Module layout

```
sdk/python/src/spendguard/integrations/
├── dspy.py                              # NEW — D21 (~250-300 LOC)
├── _default_estimator.py                # existing — REUSED for dspy default estimator
└── langchain.py                         # existing — UNCHANGED (reference for pattern)

sdk/python/src/spendguard/
└── _litellm_shim.py                     # NEW shim placeholder — exports `_IN_FLIGHT`
                                         # contextvar so D21 + D12 coexist without
                                         # circular import. Re-exported from
                                         # `integrations/litellm_shim.py` when D12
                                         # is installed.

deploy/demo/dspy/                        # NEW
└── README.md                            # demo-mode notes

deploy/demo/
├── Makefile                             # +DEMO_MODE=agent_real_dspy branches
├── verify_step_agent_real_dspy.sql      # NEW — SQL gate
└── demo/run_demo.py                     # +run_dspy_real_mode()

docs/site/docs/integrations/
└── dspy.md                              # NEW — public docs page

sdk/python/pyproject.toml                # +`dspy` extra

sdk/python/tests/integrations/
├── test_dspy.py                         # NEW — unit tests (~280 LOC)
└── test_dspy_real.py                    # NEW — integration with real dspy (~180 LOC)
```

## 2. Slice breakdown

### Slice 1 — Module skeleton + extras + `_PENDING` registry + run-context (S)

**Files:** `sdk/python/src/spendguard/integrations/dspy.py` (new), `sdk/python/src/spendguard/_litellm_shim.py` (new — 1 contextvar export), `sdk/python/pyproject.toml` (extra), `sdk/python/tests/integrations/test_dspy.py` (new — partial).

```python
# sdk/python/src/spendguard/integrations/dspy.py
"""DSPy integration — gates dspy.LM via BaseCallback.

Closes the path left open when D12 (LiteLLM shim) is NOT installed
OR when a custom dspy.LM subclass bypasses LiteLLM entirely. Register
this callback with `dspy.configure(callbacks=[SpendGuardDSPyCallback(...)])`
and every `dspy.LM(...)` invocation reserves before / commits after /
releases on exception.

Public surface:
    class SpendGuardDSPyCallback(BaseCallback)
"""

from __future__ import annotations

import asyncio
import contextvars
import logging
import time
import uuid
from collections.abc import Callable
from dataclasses import dataclass, field
from typing import Any

from ..client import DecisionOutcome, SpendGuardClient
from ..errors import (
    DecisionDenied,
    SidecarUnavailable,
    SpendGuardConfigError,
)
from ..ids import (
    derive_idempotency_key,
    derive_uuid_from_signature,
    new_uuid7,
)
from .._litellm_shim import _IN_FLIGHT as _SHIM_IN_FLIGHT

try:
    from dspy.utils.callback import BaseCallback
except ImportError as exc:  # pragma: no cover
    raise ImportError(
        "spendguard.integrations.dspy requires the [dspy] extra. "
        "Install with: pip install 'spendguard-sdk[dspy]'"
    ) from exc

try:
    from spendguard._proto.spendguard.common.v1 import common_pb2
except ImportError as exc:  # pragma: no cover
    raise ImportError(
        "spendguard proto stubs missing. Run `make proto` first."
    ) from exc


log = logging.getLogger("spendguard.integrations.dspy")

# Per-call state map. DSPy provides a UUID `call_id` to both start/end
# hooks; this dict bridges the two sync callbacks (which run on the
# same thread because DSPy hook dispatch is sequential).
_PENDING: dict[str, "_CallState"] = {}
_PENDING_TTL_SECONDS = 300  # 5 min upper bound; sweep on every on_lm_start


@dataclass(frozen=True, slots=True)
class RunContext:
    run_id: str


@dataclass
class _CallState:
    decision_id: str
    reservation_id: str | None
    llm_call_id: str
    step_id: str
    run_id: str
    unit: Any  # common_pb2.UnitRef
    pricing: Any  # common_pb2.PricingFreeze
    inputs_signature: str
    started_at: float = field(default_factory=time.monotonic)
    shim_token: contextvars.Token[bool] | None = None
```

```python
# sdk/python/src/spendguard/_litellm_shim.py
"""Internal: shared `_IN_FLIGHT` contextvar between D21 + D12.

D12 publishes the canonical one at import time of
`spendguard.integrations.litellm_shim`. D21 needs to set the same
sentinel without forcing the litellm extra on dspy-only installs.
Solution: a tiny standalone module both consult.

If D12's `litellm_shim` is also imported, it reuses this contextvar
(import side effect: D12's `_IN_FLIGHT` becomes a re-export of this
one). If D12 is absent, the contextvar exists but has no consumer —
setting it is a no-op.
"""
from __future__ import annotations
import contextvars

_IN_FLIGHT: contextvars.ContextVar[bool] = contextvars.ContextVar(
    "spendguard_shim_in_flight", default=False,
)
```

```toml
# sdk/python/pyproject.toml (additive)
[project.optional-dependencies]
dspy = [
  "dspy-ai>=2.6,<3.0",
]
```

**Tests in slice 1:** `test_import_error_when_dspy_missing`, `test_run_context_default_factory_emits_uuid7`, `test_pending_registry_ttl_sweep_drops_old_entries`, `test_shared_contextvar_is_same_object_as_d12`.

### Slice 2 — Callback class: `on_lm_start` + `on_lm_end` wiring (M)

**Files:** `sdk/python/src/spendguard/integrations/dspy.py` (extend Slice 1), `sdk/python/tests/integrations/test_dspy.py` (extend Slice 1).

```python
ClaimEstimator = Callable[[dict[str, Any]], list[Any]]
ClaimReconciler = Callable[[Any], list[Any]]
BudgetResolver = Callable[[str], "BudgetBinding"]


@dataclass(frozen=True, slots=True)
class BudgetBinding:
    budget_id: str
    window_instance_id: str
    unit: Any  # common_pb2.UnitRef
    pricing: Any  # common_pb2.PricingFreeze


class SpendGuardDSPyCallback(BaseCallback):
    """DSPy callback that reserves before each LM call and commits after.

    Register on the global DSPy callback list::

        dspy.configure(
            lm=dspy.LM("openai/gpt-4o-mini"),
            callbacks=[SpendGuardDSPyCallback(
                client=client,
                budget_resolver=resolve_budget_for_model,
                claim_reconciler=reconcile_from_dspy_usage,
            )],
        )

    Placement: this callback MUST appear FIRST in the callbacks list
    so reserve precedes any user observer callback.
    """

    class SyncInAsyncContext(SpendGuardConfigError):
        """on_lm_start invoked from inside a running event loop."""

    def __init__(
        self,
        *,
        client: SpendGuardClient,
        budget_resolver: BudgetResolver,
        claim_estimator: ClaimEstimator | None = None,
        claim_reconciler: ClaimReconciler,
        run_context_factory: Callable[[], RunContext] | None = None,
    ) -> None:
        super().__init__()
        self._client = client
        self._budget_resolver = budget_resolver
        self._claim_estimator = claim_estimator
        self._claim_reconciler = claim_reconciler
        self._run_context_factory = run_context_factory or (
            lambda: RunContext(run_id=str(new_uuid7()))
        )

    def on_lm_start(
        self, call_id: str, instance: Any, inputs: dict[str, Any]
    ) -> None:
        self._sweep_pending()
        self._guard_async_context()
        binding = self._budget_resolver(getattr(instance, "model", ""))
        signature = _signature_from_inputs(inputs)
        rc = self._run_context_factory()
        llm_call_id = str(derive_uuid_from_signature(signature, scope="llm_call_id"))
        decision_id = str(derive_uuid_from_signature(signature, scope="decision_id"))
        step_id = f"{rc.run_id}:dspy-call:{call_id[:16]}"
        idempotency_key = derive_idempotency_key(
            tenant_id=self._client.tenant_id,
            session_id=self._client.session_id,
            run_id=rc.run_id,
            step_id=step_id,
            llm_call_id=llm_call_id,
            trigger="LLM_CALL_PRE",
        )
        projected_claims = (
            self._claim_estimator(inputs) if self._claim_estimator
            else self._default_estimator(inputs, binding)
        )

        # Block D12 wrapper from double-reserving the same call.
        token = _SHIM_IN_FLIGHT.set(True)

        try:
            outcome: DecisionOutcome = asyncio.run(
                self._client.request_decision(
                    trigger="LLM_CALL_PRE",
                    run_id=rc.run_id,
                    step_id=step_id,
                    llm_call_id=llm_call_id,
                    tool_call_id="",
                    decision_id=decision_id,
                    route="llm.call",
                    projected_claims=projected_claims,
                    idempotency_key=idempotency_key,
                )
            )
        except (DecisionDenied, SidecarUnavailable):
            _SHIM_IN_FLIGHT.reset(token)
            raise

        _PENDING[call_id] = _CallState(
            decision_id=outcome.decision_id,
            reservation_id=(
                outcome.reservation_ids[0] if outcome.reservation_ids else None
            ),
            llm_call_id=llm_call_id,
            step_id=step_id,
            run_id=rc.run_id,
            unit=binding.unit,
            pricing=binding.pricing,
            inputs_signature=signature,
            shim_token=token,
        )

    def on_lm_end(
        self,
        call_id: str,
        outputs: Any,
        exception: BaseException | None,
    ) -> None:
        state = _PENDING.pop(call_id, None)
        if state is None:
            log.warning(
                "spendguard.integrations.dspy: on_lm_end fired with no "
                "matching on_lm_start state for call_id=%s", call_id,
            )
            return
        try:
            if state.reservation_id is None:
                return
            outcome_label = (
                "CANCELLED" if isinstance(exception, asyncio.CancelledError)
                else "FAILURE" if exception is not None
                else "SUCCESS"
            )
            total_tokens = (
                self._extract_total_tokens(outputs)
                if exception is None else 0
            )
            provider_event_id = (
                self._extract_provider_event_id(outputs)
                if exception is None else ""
            )
            asyncio.run(
                self._client.emit_llm_call_post(
                    run_id=state.run_id,
                    step_id=state.step_id,
                    llm_call_id=state.llm_call_id,
                    decision_id=state.decision_id,
                    reservation_id=state.reservation_id,
                    provider_reported_amount_atomic="",
                    estimated_amount_atomic=str(total_tokens),
                    unit=state.unit,
                    pricing=state.pricing,
                    provider_event_id=provider_event_id,
                    outcome=outcome_label,
                )
            )
        finally:
            if state.shim_token is not None:
                try:
                    _SHIM_IN_FLIGHT.reset(state.shim_token)
                except ValueError:
                    # Token from a different context — best effort.
                    pass

    # ----- helpers ----------------------------------------------------------

    def _guard_async_context(self) -> None:
        try:
            asyncio.get_running_loop()
        except RuntimeError:
            return
        raise SpendGuardDSPyCallback.SyncInAsyncContext(
            "SpendGuardDSPyCallback.on_lm_start cannot be invoked from "
            "inside a running event loop (DSPy callbacks are sync and we "
            "would call asyncio.run from within a loop). Run dspy calls "
            "from a sync entrypoint or pre-emit reservations via the "
            "SpendGuardClient directly."
        )

    def _sweep_pending(self) -> None:
        if not _PENDING:
            return
        now = time.monotonic()
        stale = [
            cid for cid, st in _PENDING.items()
            if (now - st.started_at) > _PENDING_TTL_SECONDS
        ]
        for cid in stale:
            log.warning(
                "spendguard.integrations.dspy: TTL-sweeping stale call_id=%s "
                "(no on_lm_end after %ds)",
                cid, _PENDING_TTL_SECONDS,
            )
            _PENDING.pop(cid, None)

    @staticmethod
    def _extract_total_tokens(outputs: Any) -> int:
        # DSPy >=2.6 LMResponse exposes .usage dict; some custom LMs
        # return a bare list of strings. Be defensive.
        if outputs is None:
            return 0
        first = outputs[0] if isinstance(outputs, list) and outputs else outputs
        usage = getattr(first, "usage", None)
        if isinstance(usage, dict):
            return int(usage.get("total_tokens") or 0)
        return 0

    @staticmethod
    def _extract_provider_event_id(outputs: Any) -> str:
        if outputs is None:
            return ""
        first = outputs[0] if isinstance(outputs, list) and outputs else outputs
        rid = getattr(first, "id", None) or getattr(first, "response_id", None)
        return rid if isinstance(rid, str) else ""

    def _default_estimator(
        self, inputs: dict[str, Any], binding: BudgetBinding,
    ) -> list[Any]:
        # DSPy inputs typically carry `messages: list[dict]` (chat) or
        # `prompt: str` (completions). Estimate via chars/4 fallback.
        from ._default_estimator import langchain_default_claim_estimator  # noqa: E501
        # Reuse the langchain estimator stub — same shape.
        # In practice slice 2 will wire a dspy-specific dispatch when
        # vendored tokenizers are available; this fallback keeps
        # default-mode users functional.
        chars = 0
        for m in inputs.get("messages") or []:
            chars += len(str(m.get("content", "")))
        chars += len(str(inputs.get("prompt", "")))
        projected = max(50, chars // 4)
        return [common_pb2.BudgetClaim(
            budget_id=binding.budget_id,
            unit=binding.unit,
            amount_atomic=str(projected),
            direction=common_pb2.BudgetClaim.DEBIT,
            window_instance_id=binding.window_instance_id,
        )]


def _signature_from_inputs(inputs: dict[str, Any]) -> str:
    import hashlib, json
    payload = json.dumps(inputs, sort_keys=True, default=str)
    return hashlib.blake2b(payload.encode("utf-8"), digest_size=16).hexdigest()


__all__ = [
    "BudgetBinding",
    "ClaimEstimator",
    "ClaimReconciler",
    "BudgetResolver",
    "RunContext",
    "SpendGuardDSPyCallback",
]
```

**Tests in slice 2:** `test_on_lm_start_calls_request_decision`, `test_on_lm_start_records_pending_state`, `test_on_lm_end_commits_with_real_usage`, `test_on_lm_end_failure_outcome_propagates`, `test_on_lm_end_cancellation_outcome`, `test_async_context_raises`, `test_in_flight_contextvar_set_by_on_lm_start`, `test_in_flight_reset_after_on_lm_end`, `test_on_lm_end_without_matching_start_logs_and_returns`.

### Slice 3 — Tests + demo `agent_real_dspy` (M)

**Files:** `sdk/python/tests/integrations/test_dspy.py` (complete), `sdk/python/tests/integrations/test_dspy_real.py` (new), `deploy/demo/Makefile` (extend), `deploy/demo/demo/run_demo.py` (extend), `deploy/demo/verify_step_agent_real_dspy.sql` (new).

Complete unit suite (~280 LOC) per `tests.md` §2. Real-dspy integration test (~180 LOC): builds a real `dspy.LM("openai/gpt-4o-mini")`, mocks the OpenAI HTTP endpoint via `pytest-httpx`, configures `dspy.settings.configure(lm=..., callbacks=[SpendGuardDSPyCallback(...)])`, runs `dspy.Predict("question -> answer")("hi")`, asserts:

- `pytest-httpx` records exactly one OpenAI call
- Fake sidecar records exactly one `RequestDecision`
- Strict order: reserve event set before httpx records

Demo `run_dspy_real_mode` (3 steps):

```python
async def run_dspy_real_mode() -> int:
    if not os.environ.get("OPENAI_API_KEY"):
        print("[demo] FATAL: OPENAI_API_KEY required for agent_real_dspy", file=sys.stderr)
        return 8

    import dspy
    from spendguard import SpendGuardClient, new_uuid7
    from spendguard.integrations.dspy import (
        SpendGuardDSPyCallback, BudgetBinding,
    )
    from spendguard._proto.spendguard.common.v1 import common_pb2

    # Connect + handshake (same pattern as run_openai_agents_mode)
    ...

    def resolve(model_str: str) -> BudgetBinding:
        return BudgetBinding(
            budget_id=budget_id, window_instance_id=window_id,
            unit=unit, pricing=pricing,
        )

    def reconcile(outputs):
        first = outputs[0] if outputs else None
        usage = getattr(first, "usage", {}) or {}
        return [common_pb2.BudgetClaim(
            budget_id=budget_id, unit=unit,
            amount_atomic=str(usage.get("total_tokens", 100)),
            direction=common_pb2.BudgetClaim.DEBIT,
            window_instance_id=window_id,
        )]

    callback = SpendGuardDSPyCallback(
        client=client, budget_resolver=resolve, claim_reconciler=reconcile,
    )
    dspy.configure(
        lm=dspy.LM("openai/gpt-4o-mini"),
        callbacks=[callback],  # FIRST in list
    )

    # Step 1 ALLOW: ChainOfThought end-to-end
    qa = dspy.ChainOfThought("question -> answer")
    result = qa(question="What is 2+2?")
    print(f"[demo] step 1 ALLOW: {result.answer!r}")

    # Step 2 DENY: budget exhausted via resolver injection
    ...

    # Step 3 STREAM: dspy.LM with stream=True
    ...

    print("[demo] agent_real_dspy ALL 3 steps PASS")
    return 0
```

`verify_step_agent_real_dspy.sql` (mirrors `verify_step_litellm_direct.sql` shape — 5 assertions: reserve ≥ 1 + commit ≥ 1 + denied ≥ 1 + audit `decision_context->>'integration' = 'dspy'` ≥ 1 + canonical event ≥ 1).

### Slice 4 — Docs + README (S)

**Files:** `docs/site/docs/integrations/dspy.md` (new), `README.md` (extend adapter table), `deploy/demo/dspy/README.md` (new).

`dspy.md` ~200 LOC. Contents:

- "1-minute install" code snippet with `dspy.configure(callbacks=[...])` call.
- 2-path decision matrix:
  | Path | When to use |
  |------|------------|
  | D12 LiteLLM shim (transitive) | You already use `litellm` SDK directly and want one install to cover all framework callers. |
  | D21 BaseCallback (direct) | You want first-class DSPy gating, OR you use custom `dspy.LM` subclasses that bypass LiteLLM, OR D12 is too broad for your install footprint. |
- "Limitations" section listing the 4 non-goals from `design.md` §3.
- Cross-link to D12 docs page.

`README.md` adapter table row: `| DSPy | Python | pip install 'spendguard-sdk[dspy]' |`.

## 3. Backwards compatibility

| Surface | Action |
|---------|--------|
| `sdk/python/src/spendguard/_litellm_shim.py` (new) | Net-new module. D12's `litellm_shim.py` will import this for its `_IN_FLIGHT` contextvar on next D12 ship (verified by spec-pair check). |
| `integrations/langchain.py` / `pydantic_ai.py` / `openai_agents.py` | Untouched. |
| `[litellm]` / `[langchain]` / `[openai-agents]` extras | Untouched. New `[dspy]` extra is additive. |
| `DEMO_MODE=agent_real_dspy` | New mode; no collisions. |

## 4. Failure modes (must be tested)

| Mode | Expected | Test |
|------|----------|------|
| DSPy not installed | `ImportError` at module import with install hint | `test_import_error_message` |
| `on_lm_start` called inside running loop | `SyncInAsyncContext` | `test_async_context_raises` |
| Sidecar DENY | `DecisionDenied` propagates; no `_PENDING` entry; `_IN_FLIGHT` reset | `test_deny_blocks_and_resets_shim` |
| Sidecar DEGRADE | `SidecarUnavailable`; provider not called (DSPy sees the raise) | `test_degrade_fail_closed` |
| `SPENDGUARD_DSPY_FAIL_OPEN=1` + DEGRADE | call allowed; WARN; no commit row | `test_fail_open_skips_commit` |
| `on_lm_end` with `exception=ProviderHttpError` | `outcome=FAILURE`; `_IN_FLIGHT` reset | `test_on_lm_end_failure_outcome_propagates` |
| `on_lm_end` with `CancelledError` | `outcome=CANCELLED` | `test_on_lm_end_cancellation_outcome` |
| `on_lm_end` without matching `on_lm_start` | WARN log; no commit; no exception | `test_on_lm_end_without_start_logs_and_returns` |
| `_PENDING` entry older than 5min | Swept on next `on_lm_start`; WARN | `test_pending_registry_ttl_sweep_drops_old_entries` |
| Custom `dspy.LM` subclass missing `.usage` | Estimator-projected claim + WARN; outcome SUCCESS | `test_custom_lm_subclass_no_usage_falls_back` |

## 5. Code size estimate

| File | Impl LOC | Test LOC |
|------|----------|----------|
| `dspy.py` | ~280 | — |
| `_litellm_shim.py` | ~15 | — |
| `test_dspy.py` | — | ~280 |
| `test_dspy_real.py` | — | ~180 |
| `verify_step_agent_real_dspy.sql` | — | ~70 |
| Demo additions to `run_demo.py` | ~150 | — |
| `dspy.md` | ~200 | — |
| README adapter row | +1 | — |

Total: ~650 impl + ~530 test.

## 6. Out of scope

Everything in `design.md` §3. Plus: no changes to `litellm_shim.py` (will pull from `_litellm_shim.py` on its own ship cycle). Plus: no proto / SQL / Rust changes. Plus: no `on_tool_*` / `on_module_*` hook coverage (D21.1).
