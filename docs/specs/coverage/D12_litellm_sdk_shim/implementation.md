# D12 — Implementation

**Reads:** [`design.md`](design.md), [`acceptance.md`](acceptance.md), [`review-standards.md`](review-standards.md).
**Touches:** Python SDK + demo orchestration + public docs. No Rust changes. No proto changes. No DB schema changes.

## 1. Module layout

```
sdk/python/src/spendguard/integrations/
├── litellm.py                          # existing 1141 LOC — small constructor patch only
├── litellm_guardrail.py                # D11 — UNCHANGED
├── _default_estimator.py               # existing — REUSED
└── litellm_shim.py                     # NEW — D12 (~450-550 LOC)

deploy/demo/litellm_sdk/                # NEW
├── spendguard_shim_bootstrap.py        # env-driven install() factory
└── README.md                           # demo-mode notes

deploy/demo/
├── Makefile                            # +DEMO_MODE=litellm_sdk_real / litellm_sdk_deny branches
├── verify_step_litellm_sdk.sql         # NEW — SQL gate (covers both modes)
└── demo/run_demo.py                    # +run_litellm_sdk_real_mode(), +run_litellm_sdk_deny_mode()

docs/site/docs/integrations/
└── litellm-sdk-shim.md                 # NEW — public docs page

sdk/python/pyproject.toml               # +`litellm-shim` extra (alias of `litellm`)

sdk/python/tests/integrations/
├── test_litellm_shim.py                # NEW — unit tests with mock litellm (~500 LOC)
├── test_litellm_shim_real.py           # NEW — integration with real litellm + pytest-httpx (~250 LOC)
└── test_crewai_via_shim.py             # NEW — transitive coverage smoke (~150 LOC)
```

## 2. Slice breakdown

### Slice 1 — Module skeleton + install/uninstall state machine + recursion guard (M)

**Files:** `sdk/python/src/spendguard/integrations/litellm_shim.py` (new), `sdk/python/tests/integrations/test_litellm_shim.py` (new — partial).

```python
# sdk/python/src/spendguard/integrations/litellm_shim.py
"""LiteLLM SDK monkey-patch shim.

Closes the gap left open by LiteLLM Issue #8842: `async_pre_call_hook`
only fires on the proxy path. Direct `litellm.acompletion()` callers
have no pre-call gate. This shim monkey-patches the SDK entry points
so SpendGuard reserves BEFORE the provider HTTP request leaves the
process, regardless of how the call was issued.

Unblocks transitive coverage for CrewAI, DSPy, SmolAgents, AWS Strands,
BeeAI, AutoGen, Atomic Agents — all route through `litellm.acompletion`.

Public surface:
    install(*, client, budget_resolver, claim_reconciler, ...) -> None
    uninstall() -> None
    is_installed() -> bool
"""

from __future__ import annotations

import asyncio
import contextvars
import logging
from dataclasses import dataclass, field
from typing import Any, Callable

from ..client import SpendGuardClient
from ..errors import SpendGuardConfigError
from .litellm import (
    BudgetResolver, ClaimEstimator, ClaimReconciler,
    SpendGuardDirectAcompletion,
)

try:
    import litellm
    from litellm import Router
except ImportError as exc:  # pragma: no cover
    raise ImportError(
        "spendguard.integrations.litellm_shim requires LiteLLM (>=1.50). "
        "Install: pip install 'spendguard-sdk[litellm-shim]'"
    ) from exc


log = logging.getLogger("spendguard.integrations.litellm_shim")


class SpendGuardShimAlreadyInstalled(SpendGuardConfigError):
    """install() called while shim already active with a different config."""


class SpendGuardShimSyncInAsyncContext(SpendGuardConfigError):
    """litellm.completion() called from inside a running event loop."""


# Re-entry guard. Set inside the wrapper; checked at the top so any
# LiteLLM-internal call that routes back through litellm.acompletion
# (e.g. fallback chain) short-circuits to the original.
_IN_FLIGHT: contextvars.ContextVar[bool] = contextvars.ContextVar(
    "spendguard_shim_in_flight", default=False,
)


@dataclass
class _InstallState:
    core: SpendGuardDirectAcompletion
    config_signature: str           # stable hash for idempotent re-install
    patch_router: bool
    patch_sync: bool
    originals: list[tuple[Any, str, Callable[..., Any]]] = field(
        default_factory=list,
    )
    patched_subclasses: list[type] = field(default_factory=list)


_INSTALL_STATE: _InstallState | None = None


def is_installed() -> bool:
    return _INSTALL_STATE is not None


def install(
    *,
    client: SpendGuardClient,
    budget_resolver: BudgetResolver,
    claim_estimator: ClaimEstimator | None = None,
    claim_reconciler: ClaimReconciler,
    patch_router: bool = True,
    patch_sync: bool = True,
) -> None:
    global _INSTALL_STATE
    new_sig = _compute_config_signature(
        client, budget_resolver, claim_estimator, claim_reconciler,
        patch_router, patch_sync,
    )
    if _INSTALL_STATE is not None:
        if _INSTALL_STATE.config_signature == new_sig:
            return  # idempotent no-op
        raise SpendGuardShimAlreadyInstalled(
            "spendguard_litellm_shim.install() already active with a "
            "different config. Call uninstall() first."
        )
    core = SpendGuardDirectAcompletion(
        client=client,
        budget_resolver=budget_resolver,
        claim_estimator=claim_estimator,
        claim_reconciler=claim_reconciler,
    )
    state = _InstallState(
        core=core, config_signature=new_sig,
        patch_router=patch_router, patch_sync=patch_sync,
    )
    _patch_acompletion(state)         # Slice 2
    _patch_atext_completion(state)    # Slice 2
    if patch_sync:
        _patch_completion(state)      # Slice 3
        _patch_text_completion(state) # Slice 3
    if patch_router:
        _patch_router(state)          # Slice 4
    _INSTALL_STATE = state
    log.info(
        "spendguard_litellm_shim installed: %d entry points patched",
        len(state.originals),
    )


def uninstall() -> None:
    global _INSTALL_STATE
    if _INSTALL_STATE is None:
        return
    # Walk in reverse so Router subclasses restore before Router itself.
    for owner, attr, original in reversed(_INSTALL_STATE.originals):
        setattr(owner, attr, original)
    _INSTALL_STATE = None
    log.info("spendguard_litellm_shim uninstalled")
```

**Tests in slice 1:** `test_install_idempotent_same_config`, `test_install_different_config_raises`, `test_uninstall_restores_originals`, `test_is_installed_lifecycle`, `test_in_flight_recursion_guard_short_circuits`.

### Slice 2 — Patch acompletion + atext_completion (M)

```python
def _patch_acompletion(state: _InstallState) -> None:
    original = litellm.acompletion
    state.originals.append((litellm, "acompletion", original))

    async def _wrapper(**kwargs: Any) -> Any:
        if _IN_FLIGHT.get():
            return await original(**kwargs)
        token = _IN_FLIGHT.set(True)
        try:
            return await state.core(
                _original_acompletion=original, **kwargs,
            )
        finally:
            _IN_FLIGHT.reset(token)

    litellm.acompletion = _wrapper  # type: ignore[assignment]


def _patch_atext_completion(state: _InstallState) -> None:
    original = litellm.atext_completion
    state.originals.append((litellm, "atext_completion", original))

    async def _wrapper(**kwargs: Any) -> Any:
        if _IN_FLIGHT.get():
            return await original(**kwargs)
        token = _IN_FLIGHT.set(True)
        try:
            # text_completion uses `prompt` instead of `messages`;
            # core knows how to handle both via its data dict.
            return await state.core(
                _original_acompletion=original, **kwargs,
            )
        finally:
            _IN_FLIGHT.reset(token)

    litellm.atext_completion = _wrapper  # type: ignore[assignment]
```

**Key change to `SpendGuardDirectAcompletion`:** add `_original_acompletion: Callable[..., Awaitable[Any]] | None = None` to `__call__` kwargs (slice 2 includes a 5-LOC patch to `litellm.py:957-1126` core, gated such that today's callers without the kwarg still hit `litellm.acompletion` directly — backwards compatible).

Without this change, the existing line 1067 (`response = await litellm.acompletion(**litellm_kwargs)`) would re-enter the patched wrapper → infinite recursion. The contextvar guard catches it, but using the saved original is cleaner and observable.

**Tests in slice 2:** `test_acompletion_patched_routes_through_core`, `test_acompletion_reserve_fires_before_provider_http` (via pytest-httpx ordering), `test_atext_completion_patched`, `test_acompletion_passthrough_when_in_flight`.

### Slice 3 — Patch sync completion + text_completion (S)

```python
def _patch_completion(state: _InstallState) -> None:
    original = litellm.completion
    state.originals.append((litellm, "completion", original))

    def _wrapper(**kwargs: Any) -> Any:
        try:
            asyncio.get_running_loop()
            raise SpendGuardShimSyncInAsyncContext(
                "litellm.completion() called from inside a running event "
                "loop is unsafe (asyncio.run() would deadlock). Use "
                "await litellm.acompletion(...) instead.",
            )
        except RuntimeError:
            # no running loop — safe to bridge sync → async
            pass
        if _IN_FLIGHT.get():
            return original(**kwargs)
        return asyncio.run(_async_dispatch(state, kwargs))

    litellm.completion = _wrapper  # type: ignore[assignment]


async def _async_dispatch(
    state: _InstallState, kwargs: dict[str, Any],
) -> Any:
    token = _IN_FLIGHT.set(True)
    try:
        return await state.core(
            _original_acompletion=litellm.acompletion, **kwargs,
        )
    finally:
        _IN_FLIGHT.reset(token)
```

`text_completion` patch mirrors `completion`.

**Tests in slice 3:** `test_completion_in_async_context_raises`, `test_completion_sync_works`, `test_text_completion_sync_works`.

### Slice 4 — Patch Router (S)

```python
def _patch_router(state: _InstallState) -> None:
    original_a = Router.acompletion
    state.originals.append((Router, "acompletion", original_a))

    async def _router_acompletion(self: Router, **kwargs: Any) -> Any:
        if _IN_FLIGHT.get():
            return await original_a(self, **kwargs)
        token = _IN_FLIGHT.set(True)
        try:
            async def _bound_original(**kw: Any) -> Any:
                return await original_a(self, **kw)
            return await state.core(
                _original_acompletion=_bound_original, **kwargs,
            )
        finally:
            _IN_FLIGHT.reset(token)

    Router.acompletion = _router_acompletion  # type: ignore[assignment]

    # Walk live subclasses and re-patch any that overrode acompletion.
    for sub in Router.__subclasses__():
        if "acompletion" in sub.__dict__:
            sub_original = sub.acompletion
            state.originals.append((sub, "acompletion", sub_original))
            # ... same pattern ...
            state.patched_subclasses.append(sub)
```

`Router.completion` sync patch mirrors.

**Tests in slice 4:** `test_router_acompletion_patched`, `test_router_subclass_at_install_time_patched`, `test_router_subclass_created_after_install_inherits_patched`.

### Slice 5 — Unit tests with mock litellm + pytest-httpx ordering (M)

Full unit suite. Mocks `litellm.acompletion` via `monkeypatch.setattr` to a no-op `AsyncMock` after recording the call order. Uses `_fake_sidecar.py` fixture from D11's test surface for the SpendGuard RPC mock.

**Ordering assertion (THE load-bearing test):**

```python
async def test_reserve_fires_before_provider_http(
    fake_sidecar, monkeypatch, httpx_mock,
):
    # Record order: (1) sidecar reserve, (2) provider HTTPS
    order: list[str] = []

    async def _record_reserve(*a, **k):
        order.append("reserve")
        return _allow_outcome()
    fake_sidecar.request_decision = _record_reserve

    httpx_mock.add_callback(
        lambda req: (order.append("provider"), httpx.Response(200, json={
            "id": "test", "choices": [{"message": {
                "role": "assistant", "content": "ok"}}],
            "usage": {"prompt_tokens": 5, "completion_tokens": 10,
                      "total_tokens": 15},
        }))[1],
        url="https://api.openai.com/v1/chat/completions",
    )

    litellm_shim.install(client=fake_client, budget_resolver=R, claim_reconciler=C)
    try:
        await litellm.acompletion(
            model="gpt-4o-mini",
            messages=[{"role": "user", "content": "hi"}],
            api_key="sk-test",
        )
    finally:
        litellm_shim.uninstall()

    assert order == ["reserve", "provider"], (
        f"INV-2 broken: order was {order}"
    )
```

### Slice 6 — Integration with real litellm + pytest-httpx + CrewAI smoke (M)

`test_litellm_shim_real.py`: imports real `litellm`, mocks the OpenAI HTTP endpoint via `pytest-httpx`, runs `await litellm.acompletion(model="gpt-4o-mini", ...)` after `install()`, asserts the request is recorded by `pytest-httpx` (provider HTTP reached) AND the fake-sidecar recorded a `RequestDecision` BEFORE the httpx record (ordering check with `asyncio.Event`).

`test_crewai_via_shim.py` (the transitive coverage proof):

```python
async def test_crewai_kickoff_triggers_spendguard_reserve(
    fake_sidecar, httpx_mock,
):
    """Load-bearing acceptance gate: a real CrewAI Agent calls litellm
    internally; the shim alone enforces budgets without CrewAI knowing
    SpendGuard exists."""
    pytest.importorskip("crewai")
    from crewai import Agent, Task, Crew

    httpx_mock.add_response(
        url="https://api.openai.com/v1/chat/completions",
        json=_OK_RESPONSE, status_code=200,
    )
    litellm_shim.install(client=fake_client, budget_resolver=R, claim_reconciler=C)
    try:
        agent = Agent(role="tester", goal="say hi", backstory="t")
        task = Task(description="say hi", expected_output="hi", agent=agent)
        crew = Crew(agents=[agent], tasks=[task])
        await crew.kickoff_async()
    finally:
        litellm_shim.uninstall()

    assert fake_sidecar.reserve_call_count >= 1, (
        "CrewAI kickoff did not trigger SpendGuard reserve via shim"
    )
```

If CrewAI is not installed in the test env, `pytest.importorskip` makes the test skip — but the demo-mode driver (slice 7) installs it so CI exercises this path.

### Slice 7 — Demo modes + docs (M)

`deploy/demo/Makefile` adds:

```
else ifeq ($(DEMO_MODE),litellm_sdk_real)
	@echo "[demo] DEMO_MODE=litellm_sdk_real → SDK monkey-patch + ALLOW"
	$(COMPOSE) up -d --build \
	    postgres pki-init bundles-init canonical-seed-init manifest-init \
	    endpoint-catalog ledger canonical-ingest sidecar litellm-sdk-driver
else ifeq ($(DEMO_MODE),litellm_sdk_deny)
	@echo "[demo] DEMO_MODE=litellm_sdk_deny → SDK monkey-patch + DENY"
	$(COMPOSE) up -d --build \
	    ... litellm-sdk-driver-deny
```

`deploy/demo/litellm_sdk/spendguard_shim_bootstrap.py` (new ~80 LOC): reads env, calls `litellm_shim.install(...)`, runs a small async driver: 1 ALLOW call (asserts stub counter +1) + 1 DENY call (asserts stub counter unchanged) + 1 STREAM call. Same shape as the existing `run_litellm_direct_mode`.

`deploy/demo/verify_step_litellm_sdk.sql` (new) asserts at least 1 reserve + 1 commit + 1 denied row for `decision_context->>'mode' = 'sdk'` (new mode literal distinguishing shim from proxy / direct-wrapper / egress).

`docs/site/docs/integrations/litellm-sdk-shim.md` (new): 1-minute install snippet, decision matrix (D02 egress / D11 guardrail / D12 shim), explicit limitations section listing the 3 non-goals.

`README.md` adapter table — add row "LiteLLM SDK shim | Python | `pip install 'spendguard-sdk[litellm-shim]'`".

## 3. Backwards compatibility

| Surface | Action |
|---------|--------|
| `litellm.py::SpendGuardDirectAcompletion` 5-LOC patch (accept `_original_acompletion` kwarg) | Strictly additive; existing callers without the kwarg fall back to today's `litellm.acompletion` call. Test `test_direct_acompletion_unchanged_baseline` pins. |
| `litellm.py::SpendGuardLiteLLMCallback` | Untouched. |
| D11 `litellm_guardrail.py` | Untouched. |
| Existing PyPI extra `[litellm]` | Floor stays at 1.50. New `[litellm-shim]` extra requires 1.50. |
| `DEMO_MODE=litellm_real / litellm_deny / litellm_direct` | Untouched; new modes are additive. |

## 4. Failure modes (must be tested)

| Mode | Expected | Test |
|------|----------|------|
| LiteLLM not installed | `ImportError` at module import with install hint | `test_import_error_message` |
| `install()` twice with same config | No-op + DEBUG log | `test_install_idempotent_same_config` |
| `install()` twice with different config | `SpendGuardShimAlreadyInstalled` | `test_install_different_config_raises` |
| `litellm.completion()` from inside `pytest.mark.asyncio` test | `SpendGuardShimSyncInAsyncContext` | `test_completion_in_async_context_raises` |
| Sidecar DENY | `DecisionDenied` propagates; httpx_mock records ZERO provider calls | `test_deny_blocks_provider` |
| Sidecar DEGRADE (fail-closed) | `SidecarUnavailable`; provider not hit | `test_degrade_fail_closed` |
| `SPENDGUARD_LITELLM_FAIL_OPEN=1` + DEGRADE | call allowed; WARN log; NO commit row | `test_fail_open_skips_commit` |
| Internal litellm fallback chain re-calls `litellm.acompletion` mid-flight | Re-entry sees `_IN_FLIGHT=True`; calls original; no double-reserve | `test_recursion_guard_no_double_reserve` |
| Provider raises mid-call | `emit_llm_call_post(outcome=FAILURE)` fires; reservation released | `test_provider_exception_releases` |
| `asyncio.CancelledError` mid-call | `outcome=CANCELLED`; release fires | `test_cancellation_releases` |
| `stream=True` returns iterator | wrapper intercepts iterator; commits at exhaustion via estimator-fallback | `test_streaming_commits_at_exhaustion` |
| `uninstall()` after install | `litellm.acompletion is original`; subsequent calls bypass shim | `test_uninstall_restores` |

## 5. Code size estimate

| File | Impl LOC | Test LOC |
|------|----------|----------|
| `litellm_shim.py` | ~500 | — |
| `litellm.py` patch (accept `_original_acompletion`) | +5 | — |
| `test_litellm_shim.py` | — | ~500 |
| `test_litellm_shim_real.py` | — | ~250 |
| `test_crewai_via_shim.py` | — | ~150 |
| `spendguard_shim_bootstrap.py` | ~80 | — |
| `verify_step_litellm_sdk.sql` | — | ~80 |
| Demo driver additions to `run_demo.py` | ~200 | — |
| `litellm-sdk-shim.md` | ~250 | — |
| README adapter row | +1 | — |

Total: ~1100 impl + ~900 test.

## 6. Out of scope

Everything in design.md §3. Plus: no changes to `litellm_guardrail.py`. Plus: no `_proto` changes. Plus: no new control-plane API. Plus: no patching of `embedding` / `image_generation` / `transcription` SDK entry points.
