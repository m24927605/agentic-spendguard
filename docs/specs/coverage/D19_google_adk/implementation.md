# D19 — Implementation

Backlinks: [`design.md`](./design.md), [`tests.md`](./tests.md), [`acceptance.md`](./acceptance.md), [`review-standards.md`](./review-standards.md).

Closest prior impl files in-tree:
- [`sdk/python/src/spendguard/integrations/langchain.py`](../../../../sdk/python/src/spendguard/integrations/langchain.py)
- [`sdk/python/src/spendguard/integrations/openai_agents.py`](../../../../sdk/python/src/spendguard/integrations/openai_agents.py)
- [`sdk/python/src/spendguard/integrations/_default_estimator.py`](../../../../sdk/python/src/spendguard/integrations/_default_estimator.py)

## 1. Module layout

```
sdk/python/src/spendguard/integrations/
├── adk.py                            # NEW — primary module (D19)
└── _default_estimator.py             # MODIFIED — add adk_default_claim_estimator dispatcher
sdk/python/pyproject.toml             # MODIFIED — add [adk] extra
sdk/python/tests/integrations/
├── test_adk_unit.py                  # NEW — unit (mock ADK types)
├── test_adk_integration.py           # NEW — recorded-fixture integration
└── fixtures/adk/                     # NEW — recorded LlmRequest/LlmResponse JSON
    ├── gemini_2_0_flash_allow.json
    ├── gemini_2_0_flash_deny.json
    └── litellm_gpt_4o_mini_allow.json
deploy/demo/demo/run_demo.py          # MODIFIED — add agent_real_adk branch
Makefile                              # MODIFIED — DEMO_MODE=agent_real_adk target
docs/site/docs/integrations/adk.md    # NEW — user-facing docs page
README.md                             # MODIFIED — adapter table row
```

## 2. Extras + import guard

`pyproject.toml`:

```toml
[project.optional-dependencies]
adk = [
  "google-adk>=1.0",
]
```

Module-level guard mirrors `langchain.py`:

```python
try:
    from google.adk.agents.callback_context import CallbackContext
    from google.adk.models import LlmRequest, LlmResponse
except ImportError as exc:  # pragma: no cover
    raise ImportError(
        "spendguard.integrations.adk requires the [adk] extra. "
        "Install with: pip install 'spendguard-sdk[adk]'"
    ) from exc
```

If ADK reorganizes the import path between minor versions, fall back to `from google.adk import ...` after the canonical path fails — but **only** with an explicit `try/except ImportError` per path (no broad `except`).

## 3. Public surface (`adk.py`)

### 3.1 Types

```python
RunIdFn = Callable[["CallbackContext"], str]
"""Override for deriving run_id from CallbackContext. Default: ctx.invocation_id."""

ClaimEstimator = Callable[[Any], list[common_pb2.BudgetClaim]]
"""Project BudgetClaim list from an `LlmRequest` (or its `contents`)."""
```

### 3.2 Class skeleton

```python
class SpendGuardAdkCallback:
    """Single instance, two slots — register to both before_ and after_model_callback.

    Stateless across requests; per-request reservation_id is stashed in
    `callback_context.state["spendguard.reservation_id"]`. Multiple
    concurrent agent runs are safe because ADK constructs a fresh
    CallbackContext per Runner.run_async invocation.
    """

    _STATE_RSV_KEY = "spendguard.reservation_id"
    _STATE_DECISION_KEY = "spendguard.decision_id"
    _STATE_STEP_KEY = "spendguard.step_id"
    _STATE_LLM_CALL_KEY = "spendguard.llm_call_id"
    _STATE_DENIED_KEY = "spendguard.denied"

    def __init__(
        self,
        *,
        client: SpendGuardClient,
        budget_id: str,
        window_instance_id: str,
        unit: Any,            # common_pb2.UnitRef
        pricing: Any,         # common_pb2.PricingFreeze
        claim_estimator: ClaimEstimator | None = None,
        run_id_fn: RunIdFn | None = None,
    ) -> None: ...

    async def __call__(
        self,
        callback_context: "CallbackContext",
        payload: "LlmRequest | LlmResponse",
    ) -> "LlmResponse | None":
        if isinstance(payload, LlmRequest):
            return await self._before(callback_context, payload)
        # else: LlmResponse → POST path
        await self._after(callback_context, payload)
        return None
```

`isinstance` dispatch is preferred over arity-only dispatch — ADK passes positional `(ctx, payload)` to both slots. The type discriminates cleanly.

### 3.3 `_before` (PRE)

```python
async def _before(self, ctx: "CallbackContext", req: "LlmRequest") -> "LlmResponse | None":
    run_id = (self._run_id_fn(ctx) if self._run_id_fn else ctx.invocation_id)
    signature = self._signature_for(req)
    llm_call_id = str(derive_uuid_from_signature(signature, scope="llm_call_id"))
    decision_id = str(derive_uuid_from_signature(signature, scope="decision_id"))
    step_id = f"{run_id}:adk-call:{signature[:16]}"
    idempotency_key = derive_idempotency_key(
        tenant_id=self._client.tenant_id,
        session_id=self._client.session_id,
        run_id=run_id,
        step_id=step_id,
        llm_call_id=llm_call_id,
        trigger="LLM_CALL_PRE",
    )

    try:
        outcome = await self._client.request_decision(
            trigger="LLM_CALL_PRE",
            run_id=run_id,
            step_id=step_id,
            llm_call_id=llm_call_id,
            tool_call_id="",
            decision_id=decision_id,
            route="llm.call",
            projected_claims=self._claim_estimator(req),
            idempotency_key=idempotency_key,
        )
    except DecisionDenied as exc:
        # Stash deny marker so _after knows to skip commit + release any
        # partial-reservation state (defense-in-depth; deny carries no rsv).
        ctx.state[self._STATE_DENIED_KEY] = True
        return self._build_deny_response(exc)

    if outcome.reservation_ids:
        ctx.state[self._STATE_RSV_KEY] = outcome.reservation_ids[0]
    ctx.state[self._STATE_DECISION_KEY] = outcome.decision_id
    ctx.state[self._STATE_STEP_KEY] = step_id
    ctx.state[self._STATE_LLM_CALL_KEY] = llm_call_id
    return None  # continue to model
```

### 3.4 `_after` (POST)

```python
async def _after(self, ctx: "CallbackContext", resp: "LlmResponse") -> None:
    # If we returned a deny response in _before, ADK still calls _after
    # with our own synthetic response. Skip commit + emit nothing.
    if ctx.state.get(self._STATE_DENIED_KEY):
        return

    rsv_id = ctx.state.get(self._STATE_RSV_KEY)
    decision_id = ctx.state.get(self._STATE_DECISION_KEY)
    step_id = ctx.state.get(self._STATE_STEP_KEY)
    llm_call_id = ctx.state.get(self._STATE_LLM_CALL_KEY)
    if not (rsv_id and decision_id and step_id and llm_call_id):
        # Defensive: a partial state means _before never ran (e.g. operator
        # registered _after only). Don't commit something we didn't reserve.
        return

    total_tokens = self._extract_total_tokens(resp)
    provider_event_id = self._extract_provider_event_id(resp)
    run_id = self._run_id_fn(ctx) if self._run_id_fn else ctx.invocation_id

    await self._client.emit_llm_call_post(
        run_id=run_id,
        step_id=step_id,
        llm_call_id=llm_call_id,
        decision_id=decision_id,
        reservation_id=rsv_id,
        provider_reported_amount_atomic="",
        estimated_amount_atomic=str(total_tokens),
        unit=self._unit,
        pricing=self._pricing,
        provider_event_id=provider_event_id,
        outcome="SUCCESS",
    )
```

### 3.5 Deny response builder

```python
def _build_deny_response(self, exc: DecisionDenied) -> "LlmResponse":
    reason = ",".join(getattr(exc, "reason_codes", []) or ["BUDGET_EXHAUSTED"])
    return LlmResponse(
        error_code="SPENDGUARD_DENY",
        error_message=f"SpendGuard denied LLM call: {reason}",
    )
```

The exact `LlmResponse` constructor differs between ADK 1.0 and 1.4 (1.4 added `content=None` default); pass keyword args only. If a future ADK release renames `error_code`, version-check at module-load and adapt.

### 3.6 Usage extraction

```python
@staticmethod
def _extract_total_tokens(resp: Any) -> int:
    usage = getattr(resp, "usage_metadata", None)
    if usage is None:
        return 0
    # 1) Gemini canonical:
    total = getattr(usage, "total_token_count", None)
    if isinstance(total, int) and total > 0:
        return total
    # 2) Gemini split:
    prompt = getattr(usage, "prompt_token_count", None) or 0
    cands = getattr(usage, "candidates_token_count", None) or 0
    if prompt or cands:
        return int(prompt) + int(cands)
    # 3) LiteLlm-wrapped OpenAI shape:
    total = getattr(usage, "total_tokens", None)
    if isinstance(total, int):
        return total
    return 0


@staticmethod
def _extract_provider_event_id(resp: Any) -> str:
    # Gemini: usage_metadata has no response_id; the LlmResponse itself
    # carries a `response_id` field on ADK >= 1.2. Fall back to "".
    rid = getattr(resp, "response_id", None) or getattr(resp, "id", None)
    return str(rid) if isinstance(rid, str) else ""
```

### 3.7 Signature

```python
def _signature_for(self, req: "LlmRequest") -> str:
    # ADK LlmRequest.contents is a list[Content]; coerce via repr for
    # stability + collapse so retries produce identical signatures.
    contents = repr(getattr(req, "contents", []))
    model = str(getattr(req, "model", "")) or ""
    payload = f"{model}|{contents}"
    return hashlib.blake2b(payload.encode("utf-8"), digest_size=16).hexdigest()
```

## 4. Default estimator (`_default_estimator.py` addition)

Add an `adk_default_claim_estimator(budget_id, window_instance_id, unit, model)` factory that:

1. Inspects `model` string. `"gemini-*"` → Gemini family; `"openai/*"` (LiteLlm prefix) → OpenAI family; `"anthropic/*"` → Anthropic family; else → chars/4 fallback with `warnings.warn` once per (model, process).
2. Returns a callable `(req) -> [BudgetClaim]` that walks `req.contents` for text parts and applies the family tokenizer.
3. Mirrors the LangChain estimator's `chars // 4 → max(50, …)` floor for unknown models.

## 5. Demo wiring

`deploy/demo/demo/run_demo.py` adds:

```python
if DEMO_MODE == "agent_real_adk":
    if not os.environ.get("GOOGLE_API_KEY"):
        print("[demo] FATAL: GOOGLE_API_KEY required for agent_real_adk", file=sys.stderr)
        sys.exit(2)
    from spendguard.integrations.adk import SpendGuardAdkCallback
    from google.adk.agents import LlmAgent
    from google.adk.runners import InMemoryRunner

    cb = SpendGuardAdkCallback(
        client=client, budget_id=BUDGET_ID,
        window_instance_id=WINDOW_ID, unit=UNIT, pricing=PRICING,
    )
    agent = LlmAgent(
        name="demo-adk-agent",
        model="gemini-2.0-flash",
        instructions="You are a budget-aware assistant.",
        before_model_callback=cb,
        after_model_callback=cb,
    )
    runner = InMemoryRunner(agent=agent)
    # Run twice: ALLOW (low budget room) then DENY (zero budget room)
    ...
```

`Makefile`:

```makefile
demo-up-agent-real-adk:
\t@DEMO_MODE=agent_real_adk $(MAKE) demo-up
```

(Existing `demo-up` target reads `DEMO_MODE`; the alias is for discoverability.)

## 6. Backward compat

- No proto changes. No DB migration. No existing public-API field rename.
- Existing `langchain` / `openai_agents` / `agt` integrations untouched.
- `_default_estimator.py` only **adds** the `adk_default_claim_estimator` symbol; existing symbols keep their signatures.

## 7. Public exports

```python
# adk.py
__all__ = [
    "ClaimEstimator",
    "RunIdFn",
    "SpendGuardAdkCallback",
]
```

No `RunContext` / `run_context` exports (unlike LangChain prior) — ADK's `CallbackContext` already supplies the run_id; contextvar is unnecessary.

## 8. README adapter table row

```
| Google ADK | Python | `pip install 'spendguard-sdk[adk]'` | `LlmAgent(model="gemini-2.0-flash", before_model_callback=cb, after_model_callback=cb)` |
```

## 9. Out-of-scope reminders (lock here, audit at review)

- No TS port (D19.5).
- No `before_tool_callback`/`after_tool_callback` wiring (future).
- No streaming intra-turn gating.
- No retry-advice / `Runner.run_live` adaptation.
