# LiteLLM ⇄ Agentic SpendGuard Integration — IMPLEMENTATION.md

> Status: Proposed slice plan (doc-first; no code yet)
> Companion: `DESIGN.md`, `TEST_PLAN.md`, `ACCEPTANCE.md`, `REVIEW_STANDARDS.md`
> Branch: `feat/litellm-integration` (slices accumulate on the same branch)
> Hard cap: **≤250 lines of code per slice.**

This doc slices the integration in `DESIGN.md` into 10 machine-actionable
units. The slice count expanded from the initial 7-slice draft to cover
the 4-step `litellm_real` demo (ALLOW + DENY + STREAM + PROXY), the
3-step `litellm_deny` demo (budget / sidecar offline / resolver None),
and the ACCEPTANCE.md D1–D3 docs site coverage that were missing from
the pre-Phase-0 draft. Each slice's implementer writes ≤250 lines, runs
up to 5 Codex review rounds (REVIEW_STANDARDS.md §3.4), then proceeds to
slice N+1.

---

## 1. Slice ordering & dependencies

```
Slice 1 → 2 → 3 → 4 ┐
              │     ├→ 5 → 6 → 7 → 8 → 9 → 10
              └─────┘
                    (4 informs 9; 8 unblocks 9 step 4 PROXY)
```

Strictly serial — each slice consumes symbols, demo modes, or config keys
introduced by lower-numbered slices. The only parallel branch is
**`TEST_PLAN.md` test authoring** (peer agent writes `tests-for-slice-N`
sections concurrently with code, no merge-order dependency).

| Slice | Title | Depends on |
|-------|-------|------------|
| 1 | SDK skeleton + errors | (base) |
| 2 | Pre-call hook + reservation | 1 |
| 3 | Success commit + reconciler (non-streaming) | 2 |
| 4 | Streaming reconciler | 3 |
| 5 | Failure release + retry handling | 2, 3 |
| 6 | Demo `litellm_real` ALLOW + DENY | 1–5 |
| 7 | Demo `litellm_deny` (3 fail-closed sub-steps) | 1–5 |
| 8 | Proxy callback template + recipe | 1–5 |
| 9 | Demo `litellm_real` STREAM + PROXY | 4, 8 (and 6 — extends litellm_real) |
| 10 | Docs site + final Codex pass | 1–9 |

The 4-step `litellm_real` demo is **built incrementally**: Slice 6 ships
steps 1+2, Slice 9 appends steps 3+4 to the same `run_litellm_real_mode`
function. The demo only becomes "complete" (per ACCEPTANCE.md §5.1)
when Slice 9 lands.

---

## 2. Per-slice specifications

### Slice 1 — SDK skeleton + errors

**Goal.** Land an importable but inert `spendguard.integrations.litellm`:
dataclasses (`LiteLLMRunContext`, `ResolverContext`, `BudgetBinding`),
type aliases (`BudgetResolver`/`ClaimEstimator`/`ClaimReconciler`),
exception classes, optional-import guard, `__all__`, callback class
with `NotImplementedError` bodies, and the two new exception classes in
`errors.py`. **No business logic.**

**Files touched.**
- `sdk/python/src/spendguard/integrations/litellm.py` (NEW, ~180 lines
  — includes `_LoopBoundCallback` + `decision_context_json` plumbing)
- `sdk/python/src/spendguard/integrations/__init__.py` (+3 lines doc comment)
- `sdk/python/src/spendguard/errors.py` (+12 lines:
  `SpendGuardSidecarUnavailable`, `SpendGuardConfigError`)
- `sdk/python/src/spendguard/client.py` (+8 lines: add
  `decision_context_json: dict | None = None` kwarg to
  `request_decision`; fold into `runtime_metadata` Struct via the
  existing `google.protobuf.struct_pb2` path — Round 3 P0.2 fix)
- `sdk/python/pyproject.toml` (+5 lines: `[litellm]` extra, `litellm>=1.50,<2.0`)

**Line budget.** 208 lines total. Hard cap 250 (margin: 42).

**Inputs.** Existing SDK: `SpendGuardClient`, `DecisionDenied` (already in
`errors.py`), `derive_idempotency_key`, `derive_uuid_from_signature`,
`new_uuid7`, `common_pb2`.

**Outputs (new symbols).** `LiteLLMRunContext`, `ResolverContext`,
`BudgetBinding`, type aliases `BudgetResolver`/`ClaimEstimator`/
`ClaimReconciler`, `SpendGuardLiteLLMCallback`, **`_LoopBoundCallback`**
(SDK class that lazy-binds `SpendGuardClient` to the serving event
loop — Round 3 P0.3 fix: now lives in the SDK, not the operator
template), `install(...)` stub, `run_context()`/`current_run_context()`,
two new exceptions (`SpendGuardSidecarUnavailable`,
`SpendGuardConfigError`) in `errors.py`. `DecisionDenied` is REUSED
unchanged (DESIGN.md §5). Slice 1 also extends
`SpendGuardClient.request_decision` (sdk/python/src/spendguard/client.py
line 374) with a new `decision_context_json: dict | None = None`
kwarg which is folded into the existing `runtime_metadata` Struct
(Round 3 P0.2 fix — gives the integration a real wire path for the
12 audit fields).

**Code skeleton.**

```python
"""LiteLLM CustomLogger integration. See DESIGN.md."""
from __future__ import annotations
import contextvars
from collections.abc import Callable, Mapping
from contextlib import asynccontextmanager
from dataclasses import dataclass
from typing import Any, AsyncIterator

from ..client import SpendGuardClient
from ..errors import (
    DecisionDenied, SpendGuardConfigError, SpendGuardError,
    SpendGuardSidecarUnavailable,
)
from ..ids import (
    derive_idempotency_key, derive_uuid_from_signature, new_uuid7,
)

try:
    from litellm.integrations.custom_logger import (  # type: ignore[import-not-found]
        CustomLogger,
    )
except ImportError as exc:  # pragma: no cover
    raise ImportError(
        "spendguard.integrations.litellm requires LiteLLM. "
        "pip install 'spendguard-sdk[litellm]'"
    ) from exc

try:
    from spendguard._proto.spendguard.common.v1 import common_pb2
except ImportError as exc:  # pragma: no cover
    raise ImportError("Run `make proto` first.") from exc

_RUN_CONTEXT: contextvars.ContextVar["LiteLLMRunContext | None"] = (
    contextvars.ContextVar("spendguard_litellm_run_context", default=None)
)

@dataclass(frozen=True, slots=True)
class LiteLLMRunContext:
    run_id: str
    step_id: str | None = None

@asynccontextmanager
async def run_context(ctx): ...

def current_run_context(): return _RUN_CONTEXT.get()

@dataclass(frozen=True, slots=True)
class ResolverContext:
    """Inputs the BudgetResolver sees. See DESIGN.md §6."""
    data: Mapping[str, Any]
    user_api_key_dict: Any | None
    call_type: str

@dataclass(frozen=True, slots=True)
class BudgetBinding:
    budget_id: str
    window_instance_id: str
    unit: Any       # common_pb2.UnitRef
    pricing: Any    # common_pb2.PricingFreeze

BudgetResolver = Callable[[ResolverContext], "BudgetBinding | None"]  # Round 3 P2.2
ClaimEstimator = Callable[[ResolverContext], list[Any]]
ClaimReconciler = Callable[[ResolverContext, Any], list[Any]]

class SpendGuardLiteLLMCallback(CustomLogger):
    def __init__(self, *, client, budget_resolver, claim_estimator,
                 claim_reconciler, fail_closed: bool = True) -> None:
        self._client = client
        self._budget_resolver = budget_resolver
        self._claim_estimator = claim_estimator
        self._claim_reconciler = claim_reconciler
        self._fail_closed = fail_closed
        # Per-call stash; lives on the callback, not on `data` (P1.5).
        # Key: litellm_call_id; populated by Slice 2's pre-call hook;
        # consumed and popped by Slices 3/4/5's log-event hooks.
        self._stash: dict[str, dict] = {}

    async def async_pre_call_hook(self, user_api_key_dict, cache, data, call_type):
        raise NotImplementedError("Slice 2")
    async def async_log_success_event(self, kwargs, response_obj, start_time, end_time):
        raise NotImplementedError("Slice 3 / Slice 4 (streaming)")
    async def async_log_failure_event(self, kwargs, response_obj, start_time, end_time):
        raise NotImplementedError("Slice 5")

    # ADR-005 P0.7 fix: sync pre-wire hook MUST fail-closed loudly,
    # not silently bypass enforcement when the user calls
    # litellm.completion() (sync) with this callback installed.
    # log_pre_api_call fires only for sync calls; async path uses
    # async_pre_call_hook and never reaches this method.
    def log_pre_api_call(self, model, messages, kwargs):
        raise SpendGuardConfigError(
            "Sync litellm.completion() is not supported by the SpendGuard "
            "callback. Use litellm.acompletion() or Shape A (egress "
            "proxy chain). See DESIGN.md ADR-005."
        )


# Round 3 P0.3 fix: _LoopBoundCallback lives in the SDK (not in the
# operator template). The proxy template just instantiates it with
# the operator's resolver/estimator/reconciler. This keeps the
# event-loop-affinity workaround testable + versioned with the SDK.
class _LoopBoundCallback(SpendGuardLiteLLMCallback):
    """Lazy-init wrapper that binds the SpendGuardClient to whichever
    event loop the proxy actually serves on.

    Construction time: no client created (no event loop assumption).
    First async-hook invocation: creates the client on the calling
    loop and reuses it for all subsequent calls.
    """

    def __init__(self, *, budget_resolver, claim_estimator,
                 claim_reconciler, fail_closed: bool = True,
                 socket_path: str, tenant_id: str) -> None:
        super().__init__(
            client=None,  # set lazily
            budget_resolver=budget_resolver,
            claim_estimator=claim_estimator,
            claim_reconciler=claim_reconciler,
            fail_closed=fail_closed,
        )
        self._socket_path = socket_path
        self._tenant_id = tenant_id
        self._init_lock: asyncio.Lock | None = None

    async def _ensure_client(self) -> SpendGuardClient:
        if self._client is not None:
            return self._client
        if self._init_lock is None:
            self._init_lock = asyncio.Lock()
        async with self._init_lock:
            if self._client is None:
                c = SpendGuardClient(
                    socket_path=self._socket_path,
                    tenant_id=self._tenant_id,
                )
                await c.connect()
                await c.handshake()
                self._client = c
        return self._client

    async def async_pre_call_hook(self, *args, **kwargs):
        await self._ensure_client()
        return await super().async_pre_call_hook(*args, **kwargs)

    async def async_log_success_event(self, *args, **kwargs):
        await self._ensure_client()
        return await super().async_log_success_event(*args, **kwargs)

    async def async_log_failure_event(self, *args, **kwargs):
        await self._ensure_client()
        return await super().async_log_failure_event(*args, **kwargs)

def install(*, client, budget_resolver, claim_estimator,
            claim_reconciler, fail_closed: bool = True):
    raise NotImplementedError("Slice 2")

__all__ = [
    "BudgetBinding", "BudgetResolver", "ClaimEstimator", "ClaimReconciler",
    "LiteLLMRunContext", "ResolverContext", "SpendGuardLiteLLMCallback",
    "_LoopBoundCallback",  # exported for proxy template (Round 2 P0.5)
    "current_run_context", "install", "run_context",
]
```

**Out of scope.** Async hook bodies (Slices 2–5; Slice 1 ships
stubs raising `NotImplementedError`). `install()` body (Slice 2).
**Sync hook bodies beyond the fail-closed `log_pre_api_call`
override**: Slice 1 ships `log_pre_api_call` raising
`SpendGuardConfigError` (ADR-005 fail-closed override — Round 2
P0.7); `log_success_event` and `log_failure_event` are NOT
overridden (post-wire; would only mask errors). NG6 still applies
for "first-class sync support"; the override is the fail-closed
guard, not a sync implementation. `prompt_text` enrichment (covered
in Slice 2's `decision_context_json` work).

**Tests.** `TEST_PLAN.md#tests-for-slice-1`. Smoke: importable,
`__all__` complete, dataclasses frozen+slotted, optional-import error
mentions extra name, hooks raise `NotImplementedError`.

**Codex review focus.**
1. Optional-import shape — failure mode must mention `[litellm]` extra;
   no other transitive import fails first.
2. `__all__` matches DESIGN.md §6 list (10 names including
   `ResolverContext`); anything else is private API.
3. `BudgetBinding.unit: Any` vs `common_pb2.UnitRef` — `Any`
   deliberately avoids proto re-export coupling; document.

**Acceptance.** See `ACCEPTANCE.md#slice-1`.
`python -c "from spendguard.integrations.litellm import
SpendGuardLiteLLMCallback"` works with LiteLLM installed and fails with
documented message without. `mypy --strict` + `ruff check` clean. **Codex
loop reaches `STOPPING-RULE-MET` at round N≥2 per REVIEW_STANDARDS.md
§3.4** (P0 fix: r1-only acceptance was a protocol violation).

---

### Slice 2 — Pre-call hook + reservation

**Goal.** Implement `async_pre_call_hook`: build a `ResolverContext`
from the hook's arguments, call `budget_resolver(ctx)`, project claims
via `claim_estimator(ctx)`, call `request_decision`, stash
`decision_id`, `reservation_ids` (tuple, **plural**), and
`decision_context_json` fields on `data["spendguard"]`. Raise typed
exceptions on deny / sidecar-unreachable. After stashing, **pop
`data["spendguard"]` before returning** to prevent serialization to
the provider wire (P1.5 fix from Phase 0 review).

**Files touched.** `sdk/python/src/spendguard/integrations/litellm.py`
(edit; ~150 lines).

**Line budget.** 150 lines. Hard cap 250.

**Inputs.** Slice 1 symbols. `client.request_decision(trigger=
"LLM_CALL_PRE", ...)` signature (see `client.py:374`); `DecisionOutcome`
shape (see `client.py:127` — `reservation_ids` is `tuple[str, ...]`,
NOT a singular `reservation_id`).

**Outputs.** Working pre-call hook; `install()` factory; env reads
(`SPENDGUARD_LITELLM_FAIL_OPEN`, `SPENDGUARD_LITELLM_TTL_SECONDS`) at
**construction only** (P1.3 fix); `_build_resolver_ctx(...)` helper;
`_build_decision_context(...)` helper producing the
`decision_context_json` field bundle from DESIGN.md §8.2a.

**Code skeleton.**

```python
import logging, os
from typing import Any

log = logging.getLogger("spendguard.integrations.litellm")

# ResolverContext + decision_context_json builders are pure functions
# tested independently of the sidecar (Tier 1 unit tests).
def _build_resolver_ctx(
    *, user_api_key_dict, data: dict, call_type: str,
) -> ResolverContext:
    return ResolverContext(
        data=data,
        user_api_key_dict=user_api_key_dict,
        call_type=call_type,
    )

def _build_decision_context(
    *, ctx: ResolverContext, binding: BudgetBinding,
    litellm_call_id: str, prompt_hash: str,
) -> dict[str, Any]:
    """Returns the dict the sidecar persists into
    canonical_events.decision_context_json (DESIGN.md §8.2a)."""
    p = binding.pricing
    uak = ctx.user_api_key_dict
    return {
        "integration": "litellm",
        "litellm_call_id": litellm_call_id,
        "model": ctx.data.get("model"),
        "pricing_version": getattr(p, "pricing_version", ""),
        "price_snapshot_hash_hex": getattr(p, "price_snapshot_hash_hex", ""),
        "fx_rate_version": getattr(p, "fx_rate_version", ""),
        "unit_conversion_version": getattr(p, "unit_conversion_version", ""),
        "prompt_hash": prompt_hash,
        "call_type": ctx.call_type,
        "stream": bool(ctx.data.get("stream", False)),
        # Round 2 P0.2 fix: `mode` field gives ACCEPTANCE Q2 a real
        # filter for proxy-step join. `user_api_key_dict is None` is
        # the canonical signal for direct vs proxy.
        "mode": "proxy" if uak is not None else "direct",
        "team_id": getattr(uak, "team_id", None) if uak else None,
    }

async def async_pre_call_hook(self, user_api_key_dict, cache, data, call_type):
    rctx = _build_resolver_ctx(
        user_api_key_dict=user_api_key_dict, data=data, call_type=call_type,
    )
    binding = self._budget_resolver(rctx)
    if binding is None:
        raise SpendGuardConfigError(
            "budget_resolver returned None; resolver MUST yield a BudgetBinding "
            "(see DESIGN.md ADR-001 — no global default fallback)"
        )

    litellm_call_id = data.get("litellm_call_id")
    if not litellm_call_id:
        # Round 4 P1.1 fix: fail-closed when LiteLLM does not stamp
        # an ID. Minting a local UUID broke commit/release lookup
        # downstream (callback hooks receive kwargs without our minted
        # ID) and audit reconciliation against LiteLLM_SpendLogs.
        # The reliable LiteLLM versions (>=1.50) always stamp this;
        # absence indicates a broken setup that should be surfaced.
        raise SpendGuardConfigError(
            "data['litellm_call_id'] missing — LiteLLM did not stamp a "
            "call id. SpendGuard cannot maintain commit-lookup or "
            "audit-chain reconciliation without it. Verify "
            "`litellm>=1.50` and that the callback runs in the "
            "supported path (acompletion or proxy)."
        )
    litellm_call_id = str(litellm_call_id)
    llm_call_id = str(derive_uuid_from_signature(
        f"litellm:{litellm_call_id}", scope="llm_call_id"))
    decision_id = str(derive_uuid_from_signature(
        f"litellm:{litellm_call_id}", scope="decision_id"))

    # Run-context fallback: if no enclosing run_context(), derive run_id
    # deterministically from litellm_call_id (P1.6 fix — prevents
    # idempotency leak across retries).
    ctx_obj = current_run_context()
    run_id = ctx_obj.run_id if ctx_obj else str(
        derive_uuid_from_signature(f"litellm:{litellm_call_id}", scope="run_id"))
    step_id = (ctx_obj.step_id if ctx_obj and ctx_obj.step_id
               else f"litellm:{litellm_call_id[:16]}")

    idempotency_key = derive_idempotency_key(
        tenant_id=self._client.tenant_id,
        session_id=self._client.session_id,
        run_id=run_id, step_id=step_id, llm_call_id=llm_call_id,
        trigger="LLM_CALL_PRE",
    )

    prompt_hash = _compute_prompt_hash(data.get("messages", []))
    decision_context = _build_decision_context(
        ctx=rctx, binding=binding, litellm_call_id=litellm_call_id,
        prompt_hash=prompt_hash,
    )

    # Compute estimator ONCE; reused for request_decision AND stash
    # (Round 2 P1.1 fix — single call avoids divergence).
    estimator_claims = self._claim_estimator(rctx)
    # Round 3 P1.2 fix: enforce single-claim contract BEFORE the
    # sidecar wire (was previously only rejected at commit time).
    if len(estimator_claims) != 1:
        raise SpendGuardConfigError(
            f"claim_estimator returned {len(estimator_claims)} claims; "
            "v1 contract requires exactly 1 (DESIGN.md §6 ClaimEstimator).")

    try:
        outcome = await self._client.request_decision(
            trigger="LLM_CALL_PRE",
            run_id=run_id, step_id=step_id, llm_call_id=llm_call_id,
            tool_call_id="", decision_id=decision_id, route="llm.call",
            projected_claims=estimator_claims,
            idempotency_key=idempotency_key,
            projected_unit=binding.unit,
            # Round 4 P0.1 fix: decision_context_json kwarg is EXPLICITLY
            # passed. Slice 1 extends client.request_decision to accept
            # this kwarg and fold it into the runtime_metadata Struct.
            # TEST_PLAN §2.2 test_decision_context_json_fields asserts
            # all 12 fields land in canonical_events row.
            decision_context_json=decision_context,
        )
    except DecisionDenied:
        raise   # LiteLLM treats raised exception as block
    except SpendGuardError as exc:
        if self._fail_open_dev:  # set at __init__ from env
            log.warning(
                "spendguard: SPENDGUARD_LITELLM_FAIL_OPEN=1 — allowing call "
                "despite sidecar error %r (DEV ONLY)", exc)
            return data
        raise SpendGuardSidecarUnavailable(f"sidecar pre-call failed: {exc}") from exc

    # Round 4 P0.2 fix: validate reservation_ids cardinality BEFORE
    # returning to LiteLLM. Earlier design validated at commit time,
    # which meant provider was contacted before any error surfaced.
    # Single-reservation v1 contract MUST fail-closed pre-wire.
    if len(outcome.reservation_ids) != 1:
        raise SpendGuardConfigError(
            f"sidecar returned {len(outcome.reservation_ids)} reservations; "
            "v1 expects exactly 1 (DESIGN.md §6). Multi-budget reservation "
            "is not supported. Failing closed before provider HTTP request.")

    # Stash on a SIDE CHANNEL — not on `data`, to prevent serialization
    # to the provider HTTP wire. Keyed by litellm_call_id (P1.5 fix).
    # IMPORTANT: include the estimator output (already computed above)
    # so Slice 4's streaming reconciler can fall back to it when
    # `response_obj.usage` is missing (Round 2 Phase 0 review P1.1 fix).
    self._stash[litellm_call_id] = {
        "decision_id": outcome.decision_id,
        "reservation_ids": tuple(outcome.reservation_ids),  # PLURAL (P0.7 fix)
        "llm_call_id": llm_call_id,
        "run_id": run_id, "step_id": step_id,
        "binding": binding,
        "audit_decision_event_id": outcome.audit_decision_event_id,
        "decision_context": decision_context,
        "stream": decision_context["stream"],
        "estimator_claims": estimator_claims,
        "mode": decision_context["mode"],  # for Slices 3+ to assert
    }
    # Slices 3/4/5 retrieve via self._get_stash(kwargs) which keys on
    # kwargs["litellm_call_id"] without popping; commit/release path
    # pops via self._pop_stash(kwargs) AFTER sidecar ack (Round 3 P0.6
    # — prevents stash loss on RPC timeout retries). No data mutation,
    # nothing on the wire.
    return data

def install(*, client, budget_resolver, claim_estimator,
            claim_reconciler, fail_closed: bool = True):
    import litellm
    cb = SpendGuardLiteLLMCallback(
        client=client, budget_resolver=budget_resolver,
        claim_estimator=claim_estimator, claim_reconciler=claim_reconciler,
        fail_closed=fail_closed)
    litellm.callbacks = (litellm.callbacks or []) + [cb]
    return cb
```

**Out of scope.** Success commit (Slice 3). Streaming reconciler
(Slice 4). Failure release (Slice 5). `prompt_text` SDK-side
enrichment (Slice 3 surfaces via `prompt_hash` only — the field name
is correct here, the SDK's `compute_prompt_hash` helper is reused).

**Tests.** `TEST_PLAN.md#tests-for-slice-2`. Resolver None →
`SpendGuardConfigError`; allow → stash populated (including
`reservation_ids` tuple); deny → `DecisionDenied` propagates; sidecar
fail + `fail_closed=True` → `SpendGuardSidecarUnavailable`; **`data`
returned to LiteLLM contains NO `spendguard` key** (P1.5 invariant).

**Codex review focus.**
1. `reservation_ids` is stashed as a tuple, not singular —
   verify Slices 3/4/5 consume it correctly.
2. Idempotency-key reuse across retries — ADR-002 says reserve per
   attempt; confirm each LiteLLM retry mints a fresh `litellm_call_id`
   so derived `decision_id` is distinct (REVIEW_STANDARDS §9.6 only
   forbids reusing the SAME `decision_id` across attempts — distinct
   attempts are correct).
3. Deny path typed exception with cause — does LiteLLM rewrap
   `DecisionDenied`? Trace `reason_codes` survival to caller.
4. Pre-call hook does not block the event loop — resolver/estimator
   are sync callables; document the "CPU-only" contract.
5. `data` mutation safety (P1.5) — assert `"spendguard" not in
   returned_data`; stash lives in `self._stash`, NOT on `data`.

**Acceptance.** `ACCEPTANCE.md#slice-2`. Allow round-trip works
against real sidecar fixture; deny produces non-empty `reason_codes`
on `DecisionDenied`; no `SpendGuardClient` mocks (cross-slice
invariant §4); Codex loop reaches `STOPPING-RULE-MET` at N≥2.

---

### Slice 3 — Success commit + reconciler (non-streaming)

**Goal.** Implement `async_log_success_event` for **non-streaming**
calls: rehydrate stash, run reconciler against `response_obj.usage`,
call `client.emit_llm_call_post(outcome="SUCCESS", ...)` with the
**first** `reservation_id` from the stashed tuple. Streaming branch
(Slice 4) adds the `stream=True` case.

**Files touched.** `sdk/python/src/spendguard/integrations/litellm.py`
(edit; ~95 lines).

**Line budget.** 95 lines. Hard cap 250.

**Inputs.** Slice 2 stash; `client.emit_llm_call_post` signature (see
`client.py:707` — note `reservation_id` is a singular string;
SpendGuard's commit API doesn't support multi-reservation commits in
v1, so we pass `stash["reservation_ids"][0]` and assert
`len(reservation_ids) == 1`); `PricingFreeze` carried in
`BudgetBinding`.

**Outputs.** Success hook (non-streaming branch); `_get_stash(kwargs)`
+ `_pop_stash(kwargs)` helpers (NOT a combined `_extract_stash` —
Round 3 P0.6: pop only AFTER commit acks); `_provider_event_id(response_obj)`
helper.

**Code skeleton.**

```python
async def async_log_success_event(self, kwargs, response_obj, start_time, end_time):
    stash = self._get_stash(kwargs)
    if stash is None:
        return  # pre-call didn't fire; silent no-op
    if stash["stream"]:
        return await self._async_log_success_streaming(stash, kwargs, response_obj)

    binding: BudgetBinding = stash["binding"]
    rctx = _build_resolver_ctx(
        user_api_key_dict=kwargs.get("user_api_key_dict"),
        data=kwargs, call_type=kwargs.get("call_type", ""),
    )
    real_claims = self._claim_reconciler(rctx, response_obj)
    if len(real_claims) != 1:
        raise SpendGuardConfigError(
            f"reconciler returned {len(real_claims)} claims; v1 contract is exactly 1 "
            "(see DESIGN.md §6 ClaimReconciler)")
    real_amount = real_claims[0].amount_atomic

    reservation_ids = stash["reservation_ids"]
    if len(reservation_ids) != 1:
        raise SpendGuardConfigError(
            f"v1 expects exactly 1 reservation per call; sidecar returned "
            f"{len(reservation_ids)}. This indicates a multi-budget reservation "
            "which is not yet supported (DESIGN.md §6).")

    try:
        await self._client.emit_llm_call_post(
            run_id=stash["run_id"], step_id=stash["step_id"],
            llm_call_id=stash["llm_call_id"],
            decision_id=stash["decision_id"],
            reservation_id=reservation_ids[0],
            provider_reported_amount_atomic=real_amount,
            unit=binding.unit, pricing=binding.pricing,
            provider_event_id=self._provider_event_id(response_obj),
            outcome="SUCCESS",
        )
    except SpendGuardError:
        if self._fail_open_dev:
            log.warning("spendguard: commit failed under fail-open; reservation will TTL-sweep")
            # KEEP stash so a retry can find it (Round 3 P0.6).
            return
        raise   # KEEP stash so retry can find it; reservation TTL-sweeps if commit never lands
    self._pop_stash(kwargs)  # only on successful ack

def _get_stash(self, kwargs):
    """Look up the stash by litellm_call_id WITHOUT popping. Slice 3/5
    pop only AFTER commit/release succeeds, so a partial-commit-timeout
    retry can still find the stash (Round 3 P0.6 fix — previous
    pop-on-extract design lost retry state)."""
    call_id = kwargs.get("litellm_call_id")
    return self._stash.get(call_id) if call_id else None

def _pop_stash(self, kwargs):
    """Remove the stash entry. Called by Slice 3/5 only after the
    sidecar acknowledges the commit/release (or the operator opts
    into best-effort fail-open via env)."""
    call_id = kwargs.get("litellm_call_id")
    if call_id:
        self._stash.pop(call_id, None)

@staticmethod
def _provider_event_id(response_obj):
    return str(getattr(response_obj, "id", "") or "")
```

**Out of scope.** Streaming branch (Slice 4). Failure release
(Slice 5). Multi-reservation commits (deferred to v2 per the explicit
single-reservation check above).

**Tests.** `TEST_PLAN.md#tests-for-slice-3`. Happy commit →
`INVOICE_COMMITTED` in fixture; 2 claims → `SpendGuardConfigError`;
missing stash → silent no-op; 2 reservations stashed →
`SpendGuardConfigError`; commit RPC fail + `fail_closed` → raises.

**Codex review focus.**
1. Stash lookup keying — `kwargs["litellm_call_id"]` is reliable
   across LiteLLM versions (verify against 1.50 + 1.59 source).
2. `len(reservation_ids) != 1` is the right v1 stance — confirm with
   stakeholder.
3. `reservation_ids[0]` indexing — defensive against empty tuple
   (sidecar contract says ≥1 on ALLOW outcome).
4. `_get_stash` does NOT pop; `_pop_stash` only fires after the
   sidecar acks (Round 3 P0.6). This means stream-fail-then-retry
   keeps the stash, but it also means the SDK relies on the TTL
   sweep in cross-slice invariant 4 to avoid leaks. Verify.

**Acceptance.** `ACCEPTANCE.md#slice-3`. End-to-end: pre-call ALLOW
+ success commit produces matching chain `RESERVATION_CREATED` →
`INVOICE_COMMITTED`; re-run with same idempotency_key returns same
`invoice_id`.

---

### Slice 4 — Streaming reconciler

**Goal.** Implement the **streaming branch** of
`async_log_success_event`: worst-case estimator at pre-call is already
in Slice 2 (estimator returns a single claim with the max-token cost);
this slice adds (a) the `async def _async_log_success_streaming(...)`
branch which reads `response_obj.usage` at end-of-stream and reconciles
the same way as non-streaming, (b) `SPENDGUARD_LITELLM_TTL_SECONDS`
plumbing into the pre-call hook as `ttl_seconds` field on the
reservation request, (c) the documented client-disconnect contract:
reservation TTL-sweeps on no `async_log_success_event` (covered by
Slice 5 `async_log_failure_event` for catchable cases).

**Files touched.** `sdk/python/src/spendguard/integrations/litellm.py`
(edit; ~80 lines).

**Line budget.** 80 lines. Hard cap 250.

**Inputs.** Slice 3 stash + reconciler shape. `kwargs["stream"]` /
`stash["stream"]` bool. LiteLLM streaming response object exposes
`.usage` at end-of-stream (verified in LiteLLM 1.50+).

**Outputs.** `_async_log_success_streaming(stash, kwargs, response_obj)`
method; TTL env read at construction; clear logging on streaming
commit.

**Code skeleton.**

```python
async def _async_log_success_streaming(self, stash, kwargs, response_obj):
    """End-of-stream commit path. Difference from non-streaming:
    response_obj.usage may be missing on some provider/version
    combinations; in that case the callback (NOT the reconciler)
    falls back to the stashed estimator output (Round 3 P0.7 fix —
    reconciler signature doesn't see the stash, so the fallback
    has to live in the callback)."""
    binding: BudgetBinding = stash["binding"]
    rctx = _build_resolver_ctx(
        user_api_key_dict=kwargs.get("user_api_key_dict"),
        data=kwargs, call_type=kwargs.get("call_type", ""),
    )
    usage = getattr(response_obj, "usage", None)
    if usage is None:
        log.warning(
            "spendguard: streaming response has no .usage frame; "
            "falling back to estimator value for llm_call_id=%s",
            stash["llm_call_id"])
        real_claims = stash["estimator_claims"]  # use stashed estimate
    else:
        real_claims = self._claim_reconciler(rctx, response_obj)
    if len(real_claims) != 1:
        raise SpendGuardConfigError(
            f"streaming reconciler returned {len(real_claims)} claims; v1 contract is exactly 1")

    reservation_ids = stash["reservation_ids"]
    log.info(
        "spendguard: streaming commit llm_call_id=%s reservation=%s amount=%s",
        stash["llm_call_id"], reservation_ids[0],
        real_claims[0].amount_atomic)

    try:
        await self._client.emit_llm_call_post(
            run_id=stash["run_id"], step_id=stash["step_id"],
            llm_call_id=stash["llm_call_id"],
            decision_id=stash["decision_id"],
            reservation_id=reservation_ids[0],
            provider_reported_amount_atomic=real_claims[0].amount_atomic,
            unit=binding.unit, pricing=binding.pricing,
            provider_event_id=self._provider_event_id(response_obj),
            outcome="SUCCESS",
        )
    except SpendGuardError as exc:
        if self._fail_open_dev:
            log.warning(
                "spendguard: streaming commit failed under fail-open; "
                "reservation will TTL-sweep llm_call_id=%s err=%r",
                stash["llm_call_id"], exc)
            return
        # Round 4 P0.6 fix: wrap as SpendGuardSidecarUnavailable
        # so NF5 typed-exception contract holds at the commit boundary.
        raise SpendGuardSidecarUnavailable(
            f"sidecar unavailable at streaming commit boundary: {exc}"
        ) from exc
    self._pop_stash(kwargs)
```

**Out of scope.** Chunk-level commit (DESIGN.md ADR-003 deferred to
v2). Mid-stream sidecar disconnect chaos test (ACCEPTANCE.md NF5
covered by Slice 5 retry/release + dedicated test in TEST_PLAN §2.5).
Network-layer SSE reset reconciliation (deferred — TTL-sweep is the
durable backstop).

**Tests.** `TEST_PLAN.md#tests-for-slice-4`. Stream allow → end-of-stream
commit fires with `response.usage` reconciled amount; stream + early
caller cancel → `async_log_failure_event` releases (Slice 5 territory
but tested here against this slice's pre-call); stream + missing
`response.usage` → reconciler fallback exercised.

**Codex review focus.**
1. `response_obj.usage` shape under streaming — does LiteLLM populate
   it on the final chunk for all providers? (Anthropic specifically.)
2. Cancellation mid-stream — failure event fires; verify no race
   between commit-attempt and cancel.
3. TTL passing — `SPENDGUARD_LITELLM_TTL_SECONDS` reaches the sidecar's
   `RequestDecision.Inputs` field (verify via `client.py` source).
4. Long-running stream TTL exhaustion before commit — sidecar
   auto-releases; commit becomes a no-op (idempotency) or raises
   gracefully.

**Acceptance.** `ACCEPTANCE.md#slice-4`. Streaming
`litellm.acompletion(..., stream=True)` reserves on start, streams
chunks, commits at end-of-stream with **real** usage (not estimator
worst-case); explicit assertion that commit amount ≠ estimator amount.

---

### Slice 5 — Failure release + retry handling

**Goal.** Implement `async_log_failure_event`: rehydrate stash, call
`client.emit_llm_call_post(outcome="FAILURE"|"CANCELLED")` with the
stashed `reservation_ids[0]` to release the reservation. Implements
ADR-002 with the per-attempt-distinct-decision_id semantics clarified
in DESIGN.md §5 retry row.

**Files touched.** `sdk/python/src/spendguard/integrations/litellm.py`
(edit; ~65 lines).

**Line budget.** 65 lines. Hard cap 250.

**Inputs.** Slice 2 stash; `emit_llm_call_post` proven against
`outcome="SUCCESS"` in Slice 3 — same call, different enum.

**Outputs.** Failure hook;
`_classify_failure(exception) -> "CANCELLED"|"FAILURE"`.

**Code skeleton.**

```python
import asyncio

async def async_log_failure_event(self, kwargs, response_obj, start_time, end_time):
    stash = self._get_stash(kwargs)
    if stash is None:
        return  # no reservation ever created

    binding: BudgetBinding = stash["binding"]
    outcome = self._classify_failure(kwargs.get("exception") or response_obj)
    reservation_ids = stash["reservation_ids"]

    if len(reservation_ids) != 1:
        log.warning(
            "spendguard: failure-event has %d reservations; v1 expects 1 — releasing first",
            len(reservation_ids))

    try:
        await self._client.emit_llm_call_post(
            run_id=stash["run_id"], step_id=stash["step_id"],
            llm_call_id=stash["llm_call_id"],
            decision_id=stash["decision_id"],
            reservation_id=reservation_ids[0] if reservation_ids else "",
            provider_reported_amount_atomic="0",
            unit=binding.unit, pricing=binding.pricing,
            provider_event_id=str(getattr(response_obj, "id", "") or ""),
            outcome=outcome,
        )
    except SpendGuardError:
        # Best-effort release. TTL sweep is the durable contract.
        # Deliberately do NOT re-raise — would mask the original
        # LiteLLM exception that triggered this callback.
        log.warning("spendguard: release RPC failed; reservation will TTL-sweep")
        # KEEP stash so a retry can find it (Round 3 P0.6).
        return
    self._pop_stash(kwargs)  # only on successful release ack

@staticmethod
def _classify_failure(exception):
    # asyncio.CancelledError shape varies across versions; check both
    # the class and the typed-string fallback some LiteLLM versions
    # emit via kwargs["exception"]: str.
    if isinstance(exception, asyncio.CancelledError):
        return "CANCELLED"
    if isinstance(exception, str) and "cancelled" in exception.lower():
        return "CANCELLED"
    return "FAILURE"
```

**Out of scope.** Custom retry-budget logic (LiteLLM owns retry).
Stream-mid-cancel beyond TTL sweep (covered by Slice 4 streaming
path's failure-event fallback). Surfacing release outcome to caller
(LiteLLM has no such hook).

**Tests.** `TEST_PLAN.md#tests-for-slice-5`. Provider 500 →
`outcome=FAILURE`; `CancelledError` → `CANCELLED`; release RPC
failure swallowed; 3-attempt retry → 3 reservations, 3 releases.

**Codex review focus.**
1. Swallowing release errors masks bugs — argue both ways; verify
   TTL sweep is sufficient durability.
2. `kwargs["exception"]` may be a string in some LiteLLM versions —
   verify shape across 1.50 → 1.59; the `isinstance(str)` branch above
   defends against this.
3. Retry storm leaks reservations — ADR-002 + DESIGN.md §5 clarify
   that distinct `litellm_call_id` per attempt yields distinct
   `decision_id`; reservations don't pile under the same id. Verify
   TTL > LiteLLM retry-backoff ceiling.
4. Nested cancellation — client Ctrl-C *during* the release RPC; the
   exception swallow above handles it.
5. Success-then-failure double-emission — server-side idempotency
   dedupes; verify path exists.

**Acceptance.** `ACCEPTANCE.md#slice-5`. Failure path emits
`RESERVATION_RELEASED` with reason=FAILURE; cancellation with
reason=CANCELLED; 3-attempt retry produces 3 reserve/release pairs.
Cumulative line count Slices 1–5 ≤ 550.

---

### Slice 6 — Demo `litellm_real` ALLOW + DENY

**Goal.** Add `DEMO_MODE=litellm_real` to `run_demo.py` with steps 1+2
of the 4-step demo (ACCEPTANCE.md §5.1): real
`litellm.acompletion()` ALLOW with end-to-end ledger assertions, plus
over-budget DENY. **Counting HTTP endpoint mandatory** (P0.11 fix —
`mock_response` is BANNED for the deny assertion).

**Files touched.**
- `deploy/demo/demo/run_demo.py` (edit: `run_litellm_real_mode()`
  step 1+2 + counting HTTP listener + 1-line dispatch in `main()`).
- `deploy/demo/Dockerfile` (verify existing `[all]` extra covers
  `[litellm]`; else 1 line).
- `deploy/demo/verify_step_litellm_real.sql` (NEW — Q1/Q3 from
  ACCEPTANCE.md §5.1).

**Line budget.** 170 lines. Hard cap 250.

**Inputs.** All SDK slices (1–5); existing demo plumbing — handshake
retry loop, env reader `_env()`, Postgres in compose, sidecar UDS;
`run_agt_composite_mode` shape (line 617).

**Outputs.** `run_litellm_real_mode()` covering steps 1+2 (with hooks
for Slice 9 to append steps 3+4); `_start_counting_provider(...)`
helper; `verify_step_litellm_real.sql` with the Q1/Q3 queries from
ACCEPTANCE.md §5.1; `main()` dispatch branch.

**Out of scope.** Steps 3 (STREAM) and 4 (PROXY) — those land in
Slice 9. Q2 cross-join (depends on proxy mode → Slice 9). Deny-mode
the litellm_deny variant (Slice 7).

**Tests.** `TEST_PLAN.md#tests-for-slice-6`. The demo IS the test
(per `feedback_demo_quality_gate.md`).

**Codex review focus.**
1. Demo actually hits the wire — `assert resp.usage.completion_tokens
   > 0`, not just `resp` truthy.
2. Counting HTTP endpoint correctness — positive control: ALLOW step
   should hit the counter ≥1; DENY step should leave it at the
   pre-deny snapshot.
3. `canonical_events` chain SQL — Q1 event-type counts qualified
   correctly (per DESIGN.md §8.2 naming convention: full
   `DECISION_ALLOWED`/`INVOICE_COMMITTED`).
4. Env-var coverage — `_env()` raises on missing; enumerate every
   read and add to compose.
5. `install()` leaks `litellm.callbacks` — explicit tear-down at end
   of demo.

**Acceptance.** `ACCEPTANCE.md#slice-6`. `DEMO_MODE=litellm_real
make demo-up` exits 0 with step-1 + step-2 logs visible. Steps 3+4
log lines absent (deferred to Slice 9). **Demo as quality gate.**

---

### Slice 7 — Demo `litellm_deny` (3 fail-closed sub-steps)

**Goal.** Add `DEMO_MODE=litellm_deny` covering all three fail-closed
scenarios per ACCEPTANCE.md §5.2: (a) budget exhausted, (b) sidecar
offline, (c) resolver returns None. Each sub-step asserts the
provider-request counter remains at 0. Counting HTTP endpoint
mandatory (same constraint as Slice 6).

**Files touched.** `deploy/demo/demo/run_demo.py` (edit:
`run_litellm_deny_mode()` ~140 LOC + 1-line dispatch +
`verify_step_litellm_deny.sql` NEW).

**Line budget.** 140 lines. Hard cap 250.

**Inputs.** Slice 6 demo harness + counting endpoint helper; existing
`run_deny_mode()` (line 1438) for budget exhaustion pattern.

**Outputs.** `run_litellm_deny_mode()` driving 3 sub-steps;
`verify_step_litellm_deny.sql`.

**Out of scope.** Approval-required path (out of scope v1 entirely).
Streaming + proxy deny variants (Shape B fail-closed semantics are
identical across direct/stream/proxy; one mode covers them all).

**Tests.** `TEST_PLAN.md#tests-for-slice-7`.

**Codex review focus.**
1. Counter actually counts — positive-control allow before sub-steps
   to verify wiring (e.g. a one-off pre-step ALLOW call).
2. `DecisionDenied` vs LiteLLM-wrapped — catch both `DecisionDenied`
   and any LiteLLM `BadRequestError` rewrap.
3. Budget exhaustion pollution across sub-steps — re-seed per
   sub-step or use distinct tenants.
4. Race between exhaustion ack and reservation attempt — use
   sidecar's confirmation, not fire-and-forget.

**Acceptance.** `ACCEPTANCE.md#slice-7`. `DEMO_MODE=litellm_deny make
demo-up` exits 0; counting endpoint shows 0 upstream requests across
all 3 sub-steps; canonical_events shows the right denial events for
each.

---

### Slice 8 — Proxy callback template + recipe

**Goal.** Ship operator-facing recipe for running SpendGuard with the
LiteLLM proxy: `PROXY_RECIPE.md` + copy-pasteable
`spendguard_litellm_proxy_callback.py` example using the
**`_LoopBoundCallback`** lazy-init pattern (Round 2 P0.5 fix —
event-loop affinity of `SpendGuardClient` means we must bootstrap on
LiteLLM's serving loop, via SDK-provided `_LoopBoundCallback`) +
`proxy_config.yaml` snippet + documented proxy auth/team seeding flow
(Round 2 P0.6 fix). **No SDK code change in this slice** beyond
exporting `_LoopBoundCallback` from the integration module (added in
Slice 1's `__all__`). Unblocks Slice 9 step 4 PROXY.

**Files touched.**
- `docs/specs/litellm-integration/PROXY_RECIPE.md` (NEW; ~80 lines)
- `deploy/demo/litellm_proxy/spendguard_litellm_proxy_callback.py`
  (NEW; ~50 lines — operator-owned exemplar, not SDK).
- `deploy/demo/litellm_proxy/proxy_config.yaml` (NEW; ~30 lines)
- `sdk/python/src/spendguard/integrations/litellm.py` — **NO change**.

**Path convention (P2 fix).** Always `deploy/demo/litellm_proxy/...`
(not `deploy/demo/litellm`). TEST_PLAN.md mirrors this path.

**Line budget.** 160 lines. Hard cap 250.

**Inputs.** Working SDK callback (Slices 1–5), demonstrably exercised
(Slices 6–7).

**Outputs.**
- `PROXY_RECIPE.md` covering: (a) **`litellm_settings.callbacks`
  string dotted-path form** for proxy YAML — `callbacks:
  spendguard_litellm_proxy_callback.handler_instance` (Round 4 P1.5
  fix: proxy YAML uses string form; Python direct mode uses list of
  instances; ACCEPTANCE F1 documents both surfaces unambiguously),
  (b) multi-tenant `_resolve` using `team_id`/`key_alias` via
  `ResolverContext.user_api_key_dict` (ADR-001 Option 2), (c) Shape A
  fallback (`api_base` → SpendGuard egress proxy), (d) TTL tuning,
  fail-closed posture, `LiteLLM_SpendLogs ⨝ canonical_events` join
  story (proxy-mode only — per DESIGN.md §8.3).
- Example module from DESIGN.md §7.2 (instantiates the
  SDK-provided `_LoopBoundCallback` — no `asyncio.run` at import
  time; lazy bind on first request — Round 3 P0.4 fix).
- `proxy_config.yaml` parseable by `litellm --config`.

**Out of scope.** SDK-internal client lifecycle (handled by
`_LoopBoundCallback` lazy init in Slice 1; this slice just uses it).
Auto-instrumenter `spendguard.instrument_litellm()` (DESIGN.md §10
v2). Demo mode itself (Slice 9 owns the proxy demo step).

**Tests.** `TEST_PLAN.md#tests-for-slice-8`. Example imports cleanly;
YAML parses as valid LiteLLM config.

**Codex review focus.**
1. Template uses SDK-provided `_LoopBoundCallback` (no `asyncio.run`,
   no `asyncio.get_event_loop` at import time — Round 3 P0.4 fix).
   Client is created lazily on the LiteLLM serving event loop on
   first inbound request.
2. Resolver returning None is a foot-gun — Slice 2 raises
   `SpendGuardConfigError`; recipe must call this out loudly.
3. Shape A + Shape B simultaneously → double-counting (DESIGN.md
   §3.3); caveat present and correct.
4. Env-var name `SPENDGUARD_BUDGET_FOR_TEAM_{team_id}` (not
   `SPENDGUARD_BUDGET_team-a` — P2 fix; pick ONE form).

**Acceptance.** `ACCEPTANCE.md#slice-8`. Recipe builds; example
imports; YAML parses. Slice 9 step 4 uses this output.

---

### Slice 9 — Demo `litellm_real` STREAM + PROXY

**Goal.** Extend `run_litellm_real_mode()` from Slice 6 with steps 3
(STREAM) and 4 (PROXY), making `DEMO_MODE=litellm_real` a full 4-step
acceptance demo per ACCEPTANCE.md §5.1. Depends on Slice 4 (streaming
reconciler) and Slice 8 (proxy template).

**Files touched.**
- `deploy/demo/demo/run_demo.py` (edit — append step 3 + step 4 to
  `run_litellm_real_mode()`).
- `deploy/demo/verify_step_litellm_real.sql` (edit — add Q2 cross-join
  for the proxy step).
- `deploy/demo/litellm_proxy/Dockerfile` or compose service (NEW —
  spin up a LiteLLM proxy subprocess for step 4).

**Line budget.** 200 lines. Hard cap 250. **HIGHEST-RISK SLICE FOR
LINE BUDGET.** Pre-authorized split if breached: 9a (STREAM step) +
9b (PROXY step + Q2).

**Inputs.** Slice 6 `run_litellm_real_mode()` shell; Slice 4 streaming
reconciler in SDK; Slice 8 proxy template; counting endpoint helper.

**Outputs.** Steps 3 + 4 of the 4-step demo; final "PASS — all 4
steps OK" line; Q2 LiteLLM_SpendLogs cross-join SQL.

**Out of scope.** SDK changes (all done in Slices 1–5). New env vars
beyond what Slices 4 + 8 introduce.

**Tests.** `TEST_PLAN.md#tests-for-slice-9`. The demo IS the test.

**Codex review focus.**
1. Step 3 STREAM commit amount ≠ estimator worst-case (verify
   reconciler took effect).
2. Step 4 PROXY: HTTP POST to proxy subprocess, `team_id` correctly
   propagated via the auth flow documented in PROXY_RECIPE.md
   (Round 2 P0.6: NOT a header-only `team_id`; the resolver reads
   `user_api_key_dict.team_id` populated by the proxy's auth layer
   using a key created via `/team/new` + `/key/generate`),
   canonical_events row created, LiteLLM_SpendLogs row ALSO created
   (proxy mode is the only mode that writes SpendLogs — DESIGN.md
   §8.3), Q2 join returns ≥1 matched row.
3. Tear-down: stop proxy subprocess cleanly; release counting
   listener.
4. **`LiteLLM_SpendLogs` table/column casing** (Round 2 P3.2) —
   verify against the pinned LiteLLM version (`pyproject.toml`
   `[litellm]` extra) before relying on the cross-join SQL. Quoted
   identifier `"LiteLLM_SpendLogs"` assumes PascalCase; pin a
   regression test if the table is lowercase in the actual schema.

**Acceptance.** `ACCEPTANCE.md#slice-9`. `DEMO_MODE=litellm_real make
demo-up` outputs the full 4-line step trace + `PASS — all 4 steps OK`.

---

### Slice 10 — Docs site + final Codex pass

**Goal.** Ship the user-facing docs page + sibling Related-footer
updates + README mention required by ACCEPTANCE.md D1–D3, then run
the final whole-integration adversarial Codex pass required by
ACCEPTANCE.md C2.

**Files touched.**
- `docs/site/docs/integrations/litellm.md` (NEW; ~120 lines) — three
  paths (A=callback in-process, B=proxy + operator-owned callback
  module, C=Shape A egress-proxy fallback).
- `docs/site/docs/integrations/agt.md` (edit; +1 line Related footer
  for litellm).
- `docs/site/docs/integrations/langchain.md` (edit; +1).
- `docs/site/docs/integrations/openai-agents.md` (edit; +1).
- `docs/site/docs/integrations/pydantic-ai.md` (edit; +1).
- `docs/site/docs/quickstart.md` (edit; +1 line integration list).
- `README.md` (edit; +1 line integration list).
- `docs/specs/litellm-integration/review-logs/final-pass.md` (NEW —
  per ACCEPTANCE.md C2; created by the final Codex run).

**Line budget.** 180 lines. Hard cap 250.

**Inputs.** All Slices 1–9 merged; demos green.

**Outputs.** Public docs page; sibling Related footers updated;
README + quickstart updated; **final Codex pass produces zero new P0
findings** (ACCEPTANCE.md C2).

**Out of scope.** New API surface (the docs describe what shipped, no
new code). Per-provider deep-dives (one generic page; provider-specific
guidance is v2).

**Tests.** `TEST_PLAN.md#tests-for-slice-10` — link checker + render
check via existing docs site CI; the final Codex pass is a one-shot
run per ACCEPTANCE.md §9.4.

**Codex review focus.**
1. Three paths clearly enumerated (A/B/C); each has a code snippet
   + when-to-pick + caveat.
2. Quickest validation block matches ACCEPTANCE.md §5.1 verbatim.
3. Greenfield example pip-installs in a clean venv with no SDK
   surface (`docs/site/docs/integrations/agt.md` precedent).

**Acceptance.** `ACCEPTANCE.md#slice-10` + ACCEPTANCE.md C2 final
pass green. **Integration ships when this slice merges.**

---

## 3. Anti-patterns specific to this integration

Lessons applied from `agt.py`, `langchain.py`, `openai_agents.py`,
`pydantic_ai.py`, and the egress-proxy specs:

- **Don't import LiteLLM at module top level outside the guarded
  block.** Use the `try / except ImportError / raise with helpful
  message` pattern from `agt.py:99-111`.
- **Don't `except Exception` in callbacks.** Catch only
  `DecisionDenied`, `SpendGuardError`, `SpendGuardSidecarUnavailable`;
  propagate the rest.
- **Don't introduce a new abstraction without two existing users.**
  Two operator examples (in-process + proxy) — covered. Anything
  beyond `ResolverContext` / `BudgetBinding` is speculative; defer.
- **Don't write to `LiteLLM_SpendLogs`.** Their table, their writes.
  We own only `canonical_events` + `invoices`.
- **Don't bypass `SpendGuardClient`'s public API.** Use
  `request_decision` + `emit_llm_call_post`, not raw gRPC.
- **Don't add sync hooks** (ADR-005 / NG6). Async-only v1.
- **Don't catch `CancelledError` in pre-call.** Cancellation must
  unwind; reservation released by failure-event hook or TTL.
- **Don't read env vars per call.** Read once at `__init__`. Even
  `SPENDGUARD_LITELLM_FAIL_OPEN` is read once (P1.3 fix).
- **Don't bake provider knowledge into the SDK.**
  `response_obj.usage.completion_tokens` is uniform.
- **Don't ship `spendguard.instrument_litellm()` in v1** (DESIGN.md
  §10 v2).
- **Don't stash on `data`.** Use `self._stash` keyed by
  `litellm_call_id` (P1.5 fix); leaking SpendGuard fields onto the
  provider HTTP wire is a P0 defect.
- **Don't pass `mock_response` for the deny demo** (P0.11 fix). The
  provider-counter==0 assertion goes vacuous.

---

## 4. Cross-slice invariants

Must hold in every slice:

- **Every callback handles `asyncio.CancelledError`** — propagate in
  pre-call, classify in failure-event.
- **Every reservation has a matching commit OR release path** —
  Slice 2 reserves; binds to exactly one of Slice 3/4 commit, Slice 5
  release, or sidecar TTL sweep. No leaks.
- **No test mocks `SpendGuardClient`** — real SDK against sidecar
  fixture (`feedback_demo_quality_gate.md`).
- **No top-level `import litellm`** — always inside `try` or function
  body.
- **All public symbols in `__all__`** — anything else is private.
- **Idempotency keys are deterministic** — inputs pinned at Slice 2.
- **Frozen pricing flows pre → commit unchanged** — `PricingFreeze`
  stashed at reservation is what commit uses (issue #59).
- **No new `canonical_events` event types** — LiteLLM is just another
  caller (DESIGN.md §8.2).
- **`__init__.py` exports are stable** — Slice 1 adds 1 comment.
- **Demo modes (6/7/9) are real and self-asserting** — exit non-zero
  on any unexpected condition.
- **Stash is on `self._stash`, NEVER on `data`** (P1.5 / cross-slice).
- **Stash entries are bounded by reservation TTL** (Round 2 P1.5
  fix; Round 3 P1.3 — sweep ownership clarified): **Slice 1 adds**
  the background `_StashSweeper` `asyncio.Task` in the
  `SpendGuardLiteLLMCallback.__init__` (lazy-started on first hook
  invocation via the same `_init_lock` pattern as
  `_LoopBoundCallback`). The sweeper runs every
  `min(SPENDGUARD_LITELLM_TTL_SECONDS, 60)` seconds and drops
  entries whose `inserted_at` exceeds the TTL. Sidecar TTL-sweep
  handles the ledger side; this sweep handles the SDK memory side.
  Test: `tests/test_litellm_skeleton.py::test_stash_sweep_drops_stale_entries`
  (unit; mocked clock) + `tests/integration/test_litellm_proxy_subprocess.py::test_stash_does_not_grow_unbounded`
  (integration; real proxy lifecycle).
- **`reservation_ids` is treated as a tuple** (P0.7); singular
  `reservation_id` is a Phase-0-pre-fix bug.
- **Typed-deny exception is `DecisionDenied`** EVERYWHERE (P0.8).

---

## 5. Total line-budget rollup

Includes SDK, tests, SQL verify files, Makefile, docs site, proxy
template (P1.1 fix — was missing from pre-Phase-0 rollup).

| Slice | Files | Slice budget | Cumulative |
|-------|-------|--------------|------------|
| 1 | `litellm.py` + `errors.py` + `__init__.py` + `pyproject.toml` + `client.py` (decision_context_json kwarg) | 208 | 208 |
| 2 | `litellm.py` (edit) | 150 | 358 |
| 3 | `litellm.py` (edit) | 95 | 453 |
| 4 | `litellm.py` (edit) | 80 | 533 |
| 5 | `litellm.py` (edit) | 65 | 598 |
| 6 | `run_demo.py` + `verify_step_litellm_real.sql` | 170 | 720 |
| 7 | `run_demo.py` + `verify_step_litellm_deny.sql` | 140 | 860 |
| 8 | `PROXY_RECIPE.md` + example callback + YAML | 160 | 1020 |
| 9 | `run_demo.py` (extend) + verify SQL (Q2) + proxy compose | 200 | 1220 |
| 10 | `docs/site/.../litellm.md` + sibling footers + README + final pass | 180 | 1448 |

**Total:** 1448 lines (over the 1400 target by 48 — within
the 250-per-slice hard cap and acceptable given the Round 1-4
spec corrections that added the `decision_context_json` field
plumbing, `_LoopBoundCallback`, stash sweeper, and 12-field audit
context). **Per-slice hard cap:** 250 (worst: Slice 1 @ 208 or
Slice 9 @ 200, 42-line margin). **Pre-Phase-0 draft total:** 915
(under-counted: missed tests, SQL, docs site — P1.1).

**Test budget** (peer agent; not in this rollup; tracked in
`TEST_PLAN.md` line budgets per `tests-for-slice-N`): ≤200 LOC per
slice's test file; total ≤1500 LOC across all tiers.

**If any slice exceeds 250 mid-implementation:** stop, split, update
this doc. Don't quietly exceed.

---

## 6. Risk assessment

- **Highest line-budget risk:** Slice 9 (200 / 250). Mitigation:
  pre-authorized split into 9a (STREAM) + 9b (PROXY + Q2).
- **Highest Codex review risk:** Slice 2 (5 attack vectors; touches
  sidecar wire + exception propagation + stash discipline + env-var
  semantics). Plan ≥3 Codex rounds; escalate if 5 rounds leave P1
  unresolved.
- **Streaming + proxy compounding risk:** Slice 9 combines Slice 4
  streaming with Slice 8 proxy infrastructure in a single demo
  function. If any earlier slice has a wire-level bug, it surfaces
  here.
- **Deferral pressure risk:** Slice 6 demo needs `OPENAI_API_KEY` or
  recorded fixture; CI may lack both. Fallback: mark as `make
  demo-litellm-real` local-only with recorded cassette.
- **Scope-creep watch:** Slice 8 is where operators will ask for
  "while you're here, also auto-instrument" or "also support the
  router pattern." Refuse; file v2 issues.

---

## 7. Codex review cadence

Per `feedback_codex_review.md` + REVIEW_STANDARDS.md §3.4, **up to 5
rounds per slice**, **minimum 2** (N≥2 mandatory per §3.4 (C)).
Suggested attack-vector progression:

1. **General** — `/codex review` on diff.
2. **Adversarial** — `/codex challenge` against attack vectors listed.
3. **Async correctness** — cancellation, retries, event-loop blocking.
4. **Multi-tenant/proxy** (Slices 2, 6, 8, 9 only) — cross-tenant
   leak, `team_id` spoofing, idempotency collisions.
5. **Final** — re-run 1+2 against integrated diff. P1 unresolved →
   escalate, don't ship.

The stopping rule (§3.4) is satisfied by ANY round N≥2 with zero new
P0 and zero new P1 in critical path. Don't loop 5 rounds when 2 are
clean; don't stop at 1 when even findings-free.

---

## 8. Branch & commit strategy

- Branch: `feat/litellm-integration` off `main`.
- One commit per slice: `feat(litellm): slice N — <one-line goal>`.
- **Split into two PRs** (Phase 0 Round 4 user direction — scope
  rebalance to ship SDK + fail-closed faster):
  - **PR1 (v1.0): Slices 1-7** — SDK skeleton + pre-call + non-stream
    commit + streaming reconciler + failure release + 4-step
    `litellm_real` demo steps 1-2 + 3-step `litellm_deny` demo.
    Ships the core gate. ~860 LOC.
  - **PR2 (v1.1): Slices 8-10** — proxy callback template + recipe +
    `litellm_real` demo steps 3 STREAM + 4 PROXY + docs site +
    final whole-integration Codex pass. Ships proxy-mode + streaming
    demo gate. ~590 LOC.
  PR1 acceptance: F1/F2/F5(ALLOW+DENY)/F6 + 2-mode demos. PR2
  acceptance: F3/F4/F7/F5(full 4-step)/D1-D4/C2.
- No squash — preserve slice history for future bisect.

---

## 9. Definition of done

- All 10 slices on `main`, CI green.
- `DEMO_MODE=litellm_real` (4 steps) and `DEMO_MODE=litellm_deny`
  (3 sub-steps) both exit 0 end-to-end.
- `pip install spendguard-sdk[litellm]` works in clean venv; import of
  `SpendGuardLiteLLMCallback` succeeds.
- `docs/site/docs/integrations/litellm.md` published; sibling Related
  footers updated.
- Total integration ≤ 1400 lines.
- No P0/P1 unresolved Codex findings; **final whole-integration Codex
  pass (ACCEPTANCE.md C2) zero new P0**.
- `TEST_PLAN.md` checklist 100% green; `ACCEPTANCE.md` gates all
  checked.
- v1 non-goals (DESIGN.md §2.2) visibly unimplemented with v2 roadmap
  issue per item (DESIGN.md §10).
