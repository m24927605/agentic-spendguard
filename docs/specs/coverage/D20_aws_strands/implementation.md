# D20 — Implementation

**Reads:** [`design.md`](design.md), [`acceptance.md`](acceptance.md), [`review-standards.md`](review-standards.md).
**Touches:** Python SDK + demo orchestration + public docs. No Rust changes. No proto changes. No DB schema changes.

## 1. Module layout

```
sdk/python/src/spendguard/integrations/
├── strands.py                          # NEW — D20 (~450-550 LOC)
├── _default_estimator.py               # existing — +strands_default_claim_estimator (~50 LOC)
└── ...

sdk/python/tests/integrations/
├── test_strands.py                     # NEW — unit tests with mock Strands runtime (~400 LOC)
├── test_strands_real.py                # NEW — integration with real strands + pytest-httpx (~300 LOC)
└── fixtures/strands/                   # NEW — recorded Bedrock + OpenAI + LiteLLM response fixtures
    ├── bedrock_anthropic_3_5_sonnet.json
    ├── openai_gpt_4o_mini.json
    └── litellm_gemini_1_5_pro.json

deploy/demo/strands/                    # NEW
├── spendguard_strands_bootstrap.py     # env-driven Agent factory
└── README.md                           # demo-mode notes

deploy/demo/
├── Makefile                            # +DEMO_MODE=agent_real_strands / agent_real_strands_deny branches
├── verify_step_strands.sql             # NEW — SQL gate (covers both modes)
└── demo/run_demo.py                    # +run_strands_real_mode(), +run_strands_deny_mode()

docs/site/docs/integrations/
└── aws-strands.md                      # NEW — public docs page

sdk/python/pyproject.toml               # +`strands` extra
```

## 2. Slice breakdown

### Slice 1 — Module skeleton + extra + dataclasses (S)

**Files:** `sdk/python/src/spendguard/integrations/strands.py` (new), `sdk/python/pyproject.toml` (extra), `sdk/python/tests/integrations/test_strands.py` (new partial).

```python
# sdk/python/src/spendguard/integrations/strands.py
"""AWS Strands Agents SDK integration via HookProvider.

Validated against `aws-strands-agents>=1.0,<2`. The SDK exposes a
typed event-bus hook system: `HookProvider.register_hooks(registry)`
binds callbacks to `BeforeInvocationEvent`, `AfterInvocationEvent`,
`BeforeToolEvent`, `AfterToolEvent`, `MessageEvent`. SpendGuard
registers `before_invocation` (reserve) and `after_invocation`
(commit/release) only — tool/message events are out of scope for v1.

Coverage is enforced at the agent-runtime boundary, so it works for
every model backend Strands supports natively: Bedrock, OpenAI,
Anthropic, Gemini, Ollama, LiteLLM. The same provider instance gates
all of them identically.

Integration shape::

    from strands import Agent
    from strands.models.bedrock import BedrockModel
    from spendguard import SpendGuardClient
    from spendguard.integrations.strands import SpendGuardHookProvider
    from spendguard._proto.spendguard.common.v1 import common_pb2

    client = SpendGuardClient(socket_path=..., tenant_id=...)
    await client.connect()
    await client.handshake()

    guard = SpendGuardHookProvider(
        client=client,
        budget_id="...",
        window_instance_id="...",
        unit=common_pb2.UnitRef(...),
        pricing=common_pb2.PricingFreeze(...),
        claim_reconciler=lambda invocation, result: [common_pb2.BudgetClaim(...)],
    )

    agent = Agent(
        model=BedrockModel(model_id="anthropic.claude-3-5-sonnet-20241022-v2:0"),
        hooks=[guard],
    )
    result = await agent.invoke_async(prompt="Hello")

POC scope:
  - End-of-invocation commit only; intra-invocation streaming deltas
    (`on_message`) inherit the parent reservation.
  - Tool-call gating bundled into the parent invocation (per-tool
    budgets deferred to D20.1).
  - DEGRADE → fail-closed by default; SPENDGUARD_STRANDS_FAIL_OPEN=1
    allows the call (dev only).
"""

from __future__ import annotations

import asyncio
import contextvars
import hashlib
import logging
import os
from collections.abc import AsyncIterator, Callable
from contextlib import asynccontextmanager
from dataclasses import dataclass, field
from typing import Any

from ..client import DecisionOutcome, SpendGuardClient
from ..errors import (
    DecisionDenied, SidecarUnavailable, SpendGuardConfigError, SpendGuardError,
)
from ..ids import (
    derive_idempotency_key, derive_uuid_from_signature, new_uuid7,
)
from ..prompt_hash import compute as compute_prompt_hash

try:
    from strands.hooks import (  # type: ignore[import-not-found]
        HookProvider, HookRegistry,
        BeforeInvocationEvent, AfterInvocationEvent,
    )
except ImportError as exc:  # pragma: no cover
    raise ImportError(
        "spendguard.integrations.strands requires the [strands] extra. "
        "Install with: pip install 'spendguard-sdk[strands]'"
    ) from exc

try:
    from spendguard._proto.spendguard.common.v1 import common_pb2
except ImportError as exc:  # pragma: no cover
    raise ImportError(
        "spendguard proto stubs missing. Run `make proto` first."
    ) from exc


log = logging.getLogger("spendguard.integrations.strands")


_RUN_CONTEXT: contextvars.ContextVar[StrandsRunContext | None] = (
    contextvars.ContextVar("spendguard_strands_run_context", default=None)
)


@dataclass(frozen=True, slots=True)
class StrandsRunContext:
    run_id: str
    step_id: str | None = None


@asynccontextmanager
async def run_context(ctx: StrandsRunContext) -> AsyncIterator[StrandsRunContext]:
    token = _RUN_CONTEXT.set(ctx)
    try:
        yield ctx
    finally:
        _RUN_CONTEXT.reset(token)


def current_run_context() -> StrandsRunContext | None:
    return _RUN_CONTEXT.get()


ClaimEstimator = Callable[[Any], list[Any]]
"""Receives Strands `Invocation`; returns BudgetClaim list (single-element v1 contract)."""

ClaimReconciler = Callable[[Any, Any], list[Any]]
"""Receives (Invocation, InvocationResult); returns reconciled BudgetClaim list."""


@dataclass(slots=True)
class _PendingInvocation:
    """Stash entry, keyed by Strands invocation_id."""
    decision_id: str
    reservation_ids: tuple[str, ...]
    llm_call_id: str
    run_id: str
    step_id: str
    estimator_claim_snapshot: Any  # frozen primitives, see litellm.py:373-386
```

**Tests in slice 1:** `test_import_error_message_when_strands_missing`, `test_construct_with_minimal_args`, `test_construct_rejects_negative_budget`, `test_strands_run_context_lifecycle`.

### Slice 2 — `before_invocation` reserve + DENY/DEGRADE fail-closed + stash (M)

```python
class SpendGuardHookProvider(HookProvider):
    """Strands HookProvider that reserves before each agent invocation
    and commits/releases after. Works with any Model backend (Bedrock,
    OpenAI, Anthropic, Gemini, Ollama, LiteLLM)."""

    def __init__(
        self,
        *,
        client: SpendGuardClient,
        budget_id: str,
        window_instance_id: str,
        unit: Any,
        pricing: Any,
        claim_estimator: ClaimEstimator | None = None,
        claim_reconciler: ClaimReconciler,
        fail_closed: bool = True,
    ) -> None:
        if not budget_id:
            raise SpendGuardConfigError("budget_id required")
        if not window_instance_id:
            raise SpendGuardConfigError("window_instance_id required")
        if not getattr(unit, "unit_id", "") or "":
            raise SpendGuardConfigError("unit.unit_id required (DESIGN.md §6)")
        self._client = client
        self._budget_id = budget_id
        self._window_instance_id = window_instance_id
        self._unit = unit
        self._pricing = pricing
        if claim_estimator is None:
            from ._default_estimator import strands_default_claim_estimator
            claim_estimator = strands_default_claim_estimator(
                budget_id=budget_id,
                window_instance_id=window_instance_id,
                unit=unit,
            )
        self._claim_estimator = claim_estimator
        self._claim_reconciler = claim_reconciler
        self._fail_closed = fail_closed
        self._fail_open_dev: bool = (
            os.environ.get("SPENDGUARD_STRANDS_FAIL_OPEN") == "1"
        )
        if self._fail_open_dev:
            log.warning(
                "spendguard: SPENDGUARD_STRANDS_FAIL_OPEN=1 — fail-open; "
                "sidecar errors will allow agent invocations. DEV ONLY."
            )
        self._stash: dict[str, _PendingInvocation] = {}

    def register_hooks(self, registry: HookRegistry) -> None:
        """Strands' bus contract: bind callbacks to event types."""
        registry.add_callback(BeforeInvocationEvent, self.before_invocation)
        registry.add_callback(AfterInvocationEvent, self.after_invocation)

    async def before_invocation(self, event: BeforeInvocationEvent) -> None:
        inv = event.invocation
        invocation_id = getattr(inv, "invocation_id", None)
        if not invocation_id:
            # Strands GA contract pins this; fail-closed if missing.
            raise SpendGuardConfigError(
                "Strands Invocation has no invocation_id — pinned contract "
                "as of aws-strands-agents>=1.0. Verify SDK version."
            )
        invocation_id = str(invocation_id)

        estimator_claims = self._claim_estimator(inv)
        if len(estimator_claims) != 1:
            raise SpendGuardConfigError(
                f"claim_estimator returned {len(estimator_claims)} claims; "
                "v1 contract requires exactly 1 (DESIGN.md §6)."
            )
        # Validate claim aligns with provider binding (mirrors litellm.py).
        self._validate_claim(estimator_claims[0], source="claim_estimator")

        ctx_obj = current_run_context()
        run_id = (ctx_obj.run_id if ctx_obj else str(
            derive_uuid_from_signature(
                f"strands:{invocation_id}", scope="run_id")))
        step_id = (ctx_obj.step_id if ctx_obj and ctx_obj.step_id
                   else f"strands:{invocation_id[:16]}")
        llm_call_id = str(derive_uuid_from_signature(
            f"strands:{invocation_id}", scope="llm_call_id"))
        decision_id = str(derive_uuid_from_signature(
            f"strands:{invocation_id}", scope="decision_id"))
        idempotency_key = derive_idempotency_key(
            tenant_id=self._client.tenant_id,
            session_id=self._client.session_id,
            run_id=run_id, step_id=step_id, llm_call_id=llm_call_id,
            trigger="LLM_CALL_PRE",
        )

        try:
            outcome: DecisionOutcome = await self._client.request_decision(
                trigger="LLM_CALL_PRE",
                run_id=run_id, step_id=step_id, llm_call_id=llm_call_id,
                tool_call_id="", decision_id=decision_id, route="llm.call",
                projected_claims=estimator_claims,
                idempotency_key=idempotency_key,
                projected_unit=self._unit,
                decision_context_json=self._build_decision_context(
                    inv, invocation_id),
            )
        except DecisionDenied:
            raise  # Strands runtime surfaces as HookExecutionError
        except SpendGuardError as exc:
            if self._fail_open_dev:
                log.warning(
                    "spendguard: SPENDGUARD_STRANDS_FAIL_OPEN=1 — allowing "
                    "invocation despite sidecar error %r (DEV ONLY).", exc)
                return
            raise SidecarUnavailable(
                f"sidecar pre-invocation failed: {exc}") from exc

        if getattr(outcome, "decision", "") == "DEGRADE":
            if self._fail_open_dev:
                log.warning(
                    "spendguard: DEGRADE outcome under fail-open; "
                    "allowing invocation (DEV ONLY).")
                return
            raise SidecarUnavailable(
                "sidecar returned DEGRADE; Strands provider fails closed.")

        if len(outcome.reservation_ids) != 1:
            raise SpendGuardConfigError(
                f"sidecar returned {len(outcome.reservation_ids)} reservations; "
                "v1 expects 1 (DESIGN.md §6).")

        from types import SimpleNamespace
        snap = SimpleNamespace(
            amount_atomic=str(getattr(estimator_claims[0], "amount_atomic", "")),
            unit=SimpleNamespace(unit_id=str(self._unit.unit_id)),
        )
        self._stash[invocation_id] = _PendingInvocation(
            decision_id=outcome.decision_id,
            reservation_ids=tuple(outcome.reservation_ids),
            llm_call_id=llm_call_id,
            run_id=run_id, step_id=step_id,
            estimator_claim_snapshot=snap,
        )
```

`_validate_claim` and `_build_decision_context` mirror litellm.py:149-191 + 120-146 patterns. `decision_context["integration"] = "strands"` + `decision_context["model_id"] = inv.model.model_id` + `decision_context["model_backend"] = type(inv.model).__name__` for the 3-backend coverage matrix verification.

**Tests in slice 2:** `test_before_invocation_reserves`, `test_before_invocation_deny_raises`, `test_before_invocation_degrade_fails_closed`, `test_before_invocation_fail_open_allows_on_error`, `test_before_invocation_missing_invocation_id_raises`, `test_before_invocation_concurrent_stash_isolation`.

### Slice 3 — `after_invocation` commit/release + exception classification (M)

```python
    async def after_invocation(self, event: AfterInvocationEvent) -> None:
        invocation_id = str(getattr(event, "invocation_id", "") or "")
        pending = self._stash.pop(invocation_id, None)
        if pending is None:
            # before_invocation didn't fire (fail-open path or test skip).
            return

        inv = event.invocation
        exc = getattr(event, "exception", None)
        if exc is not None:
            outcome = self._classify_exception(exc)
            try:
                await self._client.emit_llm_call_post(
                    run_id=pending.run_id, step_id=pending.step_id,
                    llm_call_id=pending.llm_call_id,
                    decision_id=pending.decision_id,
                    reservation_id=pending.reservation_ids[0],
                    provider_reported_amount_atomic="0",
                    estimated_amount_atomic="0",
                    unit=self._unit, pricing=self._pricing,
                    provider_event_id="",
                    outcome=outcome,
                )
            except SpendGuardError as rel_exc:
                log.warning(
                    "spendguard: strands release RPC failed for "
                    "invocation_id=%s err=%r; reservation will TTL-sweep.",
                    invocation_id, rel_exc)
            return  # do NOT mask the original invocation exception

        result = event.result
        try:
            real_claims = self._claim_reconciler(inv, result)
        except Exception as rec_exc:
            # Reconciler bug — fall back to estimator snapshot, log loudly.
            log.warning(
                "spendguard: claim_reconciler raised %r for invocation_id=%s; "
                "falling back to estimator snapshot.", rec_exc, invocation_id)
            real_claims = [pending.estimator_claim_snapshot]
        if len(real_claims) != 1:
            raise SpendGuardConfigError(
                f"claim_reconciler returned {len(real_claims)} claims; "
                "v1 contract requires exactly 1.")
        real_claim = real_claims[0]
        self._validate_claim(real_claim, source="claim_reconciler")

        provider_event_id = self._extract_provider_event_id(result)
        try:
            await self._client.emit_llm_call_post(
                run_id=pending.run_id, step_id=pending.step_id,
                llm_call_id=pending.llm_call_id,
                decision_id=pending.decision_id,
                reservation_id=pending.reservation_ids[0],
                provider_reported_amount_atomic="",
                estimated_amount_atomic=str(real_claim.amount_atomic),
                unit=self._unit, pricing=self._pricing,
                provider_event_id=provider_event_id,
                outcome="SUCCESS",
            )
        except SpendGuardError as exc:
            if self._fail_open_dev:
                log.warning(
                    "spendguard: strands commit failed under fail-open; "
                    "reservation will TTL-sweep invocation_id=%s",
                    invocation_id)
                return
            raise

    @staticmethod
    def _classify_exception(exc: Any) -> str:
        if isinstance(exc, asyncio.CancelledError):
            return "CANCELLED"
        return "FAILURE"

    @staticmethod
    def _extract_provider_event_id(result: Any) -> str:
        # Strands normalizes across providers; .result.id is the canonical
        # field. Fall back to model_response.id for Bedrock raw.
        rid = getattr(result, "id", None) or getattr(
            getattr(result, "model_response", None), "id", None)
        return str(rid or "")
```

**Tests in slice 3:** `test_after_invocation_commits_success`, `test_after_invocation_releases_on_exception`, `test_after_invocation_cancelled_classification`, `test_after_invocation_no_pending_is_noop`, `test_after_invocation_reconciler_exception_falls_back_to_estimator`, `test_after_invocation_provider_event_id_from_result`.

### Slice 4 — Multi-backend tests (Bedrock + OpenAI + LiteLLM) (M)

Three integration tests, each constructing a real `Agent` with one of the three model backends and a recorded fixture. Strands' `Model` abstraction allows fixture-driven response without network.

```python
# test_strands_real.py
@pytest.mark.parametrize("backend", ["bedrock", "openai", "litellm"])
async def test_hook_provider_fires_for_backend(backend, fake_sidecar, httpx_mock):
    pytest.importorskip("strands")
    from strands import Agent
    fixture = _load_fixture(backend)
    if backend == "bedrock":
        from strands.models.bedrock import BedrockModel
        model = BedrockModel(model_id="anthropic.claude-3-5-sonnet-20241022-v2:0")
        httpx_mock.add_response(
            url=_BEDROCK_URL_RE, json=fixture["response"], status_code=200)
    elif backend == "openai":
        from strands.models.openai import OpenAIModel
        model = OpenAIModel(model="gpt-4o-mini", api_key="sk-test")
        httpx_mock.add_response(
            url="https://api.openai.com/v1/chat/completions",
            json=fixture["response"], status_code=200)
    else:  # litellm
        from strands.models.litellm import LiteLLMModel
        model = LiteLLMModel(model="gemini/gemini-1.5-pro", api_key="sk-test")
        httpx_mock.add_response(
            url=_GEMINI_URL_RE, json=fixture["response"], status_code=200)
    guard = SpendGuardHookProvider(
        client=fake_sidecar, budget_id=B, window_instance_id=W,
        unit=U, pricing=P, claim_reconciler=lambda inv, res: [_claim(res)])
    agent = Agent(model=model, hooks=[guard])

    result = await agent.invoke_async(prompt="Hello")

    assert fake_sidecar.reserve_call_count == 1
    assert fake_sidecar.commit_call_count == 1
    # Strict ordering: reserve happened before any provider HTTP.
    assert fake_sidecar.reserve_timestamp < httpx_mock.get_requests()[0].extensions["sg_recorded_at"]
    # decision_context records the right backend
    ctx = fake_sidecar.last_decision_context
    assert ctx["model_backend"] == {
        "bedrock": "BedrockModel", "openai": "OpenAIModel",
        "litellm": "LiteLLMModel"}[backend]
```

**Tests in slice 4:** the parametrized backend test above (×3 cells), plus `test_deny_blocks_provider` (parametrized ×3 — zero HTTP recorded per backend), plus `test_concurrent_invocations_no_stash_collision` (asyncio.gather of 5 invocations, each with a distinct invocation_id, all stash entries cleared correctly).

### Slice 5 — Demo modes + docs (M)

`deploy/demo/Makefile` adds:

```
else ifeq ($(DEMO_MODE),agent_real_strands)
	@echo "[demo] DEMO_MODE=agent_real_strands → Strands Agent + Bedrock-mock + ALLOW"
	$(COMPOSE) up -d --build \
	    postgres pki-init bundles-init canonical-seed-init manifest-init \
	    endpoint-catalog ledger canonical-ingest sidecar strands-driver
else ifeq ($(DEMO_MODE),agent_real_strands_deny)
	@echo "[demo] DEMO_MODE=agent_real_strands_deny → Strands Agent + DENY"
	$(COMPOSE) up -d --build \
	    ... strands-driver-deny
```

`deploy/demo/strands/spendguard_strands_bootstrap.py` (~120 LOC): reads env, constructs `SpendGuardClient` + `SpendGuardHookProvider`, runs a 3-step driver using Strands `Agent` against a Bedrock-mock httpx endpoint started in-process. Steps: (1) ALLOW Bedrock invocation, (2) ALLOW OpenAI-mock invocation (proves model-agnostic), (3) DENY Bedrock invocation (proves zero provider HTTP).

`deploy/demo/verify_step_strands.sql`: 6 SQL assertions analogous to `verify_step_litellm_sdk.sql`, with `decision_context->>'integration' = 'strands'` + `decision_context->>'model_backend' IN ('BedrockModel', 'OpenAIModel', 'LiteLLMModel')`.

`docs/site/docs/integrations/aws-strands.md`: 1-minute install snippet, model-backend coverage matrix (Bedrock / OpenAI / Anthropic / Gemini / Ollama / LiteLLM with "v1 verified" / "v1 covered, untested" / "deferred" status), explicit limitations section.

`README.md` adapter table — add row "AWS Strands | Python | `pip install 'spendguard-sdk[strands]'`".

## 3. Backwards compatibility

| Surface | Action |
|---------|--------|
| Existing integrations (`langchain`, `litellm`, `openai_agents`, `agt`, `pydantic_ai`) | UNTOUCHED. |
| `_default_estimator.py` | Additive — new `strands_default_claim_estimator` function. Existing estimators unchanged. |
| `pyproject.toml` extras | Additive — new `strands` extra. Existing extras unchanged. |
| Demo Makefile | Additive — new `agent_real_strands` + `agent_real_strands_deny` branches. No existing branch edited. |
| `_proto/` | Untouched. |
| Rust crates | Untouched. |
| DB migrations | Untouched. |

## 4. Failure modes (must be tested)

| Mode | Expected | Test |
|------|----------|------|
| `strands` not installed | `ImportError` at module import with install hint | `test_import_error_message` |
| Strands `Invocation.invocation_id` missing | `SpendGuardConfigError` with version-pin guidance | `test_missing_invocation_id_raises` |
| Sidecar DENY in `before_invocation` | `DecisionDenied` propagates; Strands runtime aborts before provider HTTP | `test_deny_blocks_provider` (×3 backends) |
| Sidecar DEGRADE | `SidecarUnavailable`; provider not hit | `test_degrade_fail_closed` |
| `SPENDGUARD_STRANDS_FAIL_OPEN=1` + DEGRADE | allow + WARN log + NO commit row | `test_fail_open_skips_commit` |
| Provider raises mid-invocation | `after_invocation` sees `event.exception`; `outcome=FAILURE` emitted; original exception propagated | `test_provider_exception_releases` |
| `asyncio.CancelledError` mid-invocation | `outcome=CANCELLED`; release fires | `test_cancellation_releases` |
| 5 concurrent `agent.invoke_async()` via `asyncio.gather` | 5 reserves + 5 commits, no stash collision | `test_concurrent_invocations_no_stash_collision` |
| Reconciler throws | Fallback to estimator snapshot + WARN log | `test_reconciler_exception_falls_back` |
| `event.result.usage` is None (streaming) | Fallback to estimator snapshot | `test_no_usage_frame_falls_back` |
| Agent model swap mid-run | New invocation_id stashes independently | `test_model_swap_mid_run_isolated_stash` |

## 5. Code size estimate

| File | Impl LOC | Test LOC |
|------|----------|----------|
| `strands.py` | ~450 | — |
| `_default_estimator.py` (+strands fn) | +50 | — |
| `test_strands.py` (unit) | — | ~400 |
| `test_strands_real.py` (×3 backends) | — | ~300 |
| `fixtures/strands/*.json` | — | ~150 |
| `spendguard_strands_bootstrap.py` | ~120 | — |
| `verify_step_strands.sql` | — | ~80 |
| Demo driver additions to `run_demo.py` | ~150 | — |
| `aws-strands.md` | ~250 | — |
| README adapter row | +1 | — |

Total: ~1020 impl + ~930 test.

## 6. Out of scope

Everything in design.md §3. Plus: no changes to LiteLLM integration files (Strands' LiteLLM-backend path goes through Strands' own `LiteLLMModel`, not through `litellm.acompletion` — D20 covers it via the hook layer regardless). Plus: no per-tool budgets. Plus: no `on_message` streaming-token gating. Plus: no Strands TS SDK (handled by D08 family).
