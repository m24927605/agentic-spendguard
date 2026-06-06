# D27 — Implementation

Backlinks: [`design.md`](./design.md), [`tests.md`](./tests.md), [`acceptance.md`](./acceptance.md), [`review-standards.md`](./review-standards.md).

Closest prior impl files in-tree:
- [`sdk/python/src/spendguard/integrations/langchain.py`](../../../../sdk/python/src/spendguard/integrations/langchain.py)
- [`sdk/python/src/spendguard/integrations/openai_agents.py`](../../../../sdk/python/src/spendguard/integrations/openai_agents.py)
- [`sdk/python/src/spendguard/integrations/agt.py`](../../../../sdk/python/src/spendguard/integrations/agt.py) — sync callback pattern via `client.request_decision_sync`
- [`sdk/python/src/spendguard/integrations/_default_estimator.py`](../../../../sdk/python/src/spendguard/integrations/_default_estimator.py)

## 1. Module layout

```
sdk/python/src/spendguard/integrations/
├── llamaindex.py                       # NEW — primary module (D27)
└── _default_estimator.py               # MODIFIED — add llamaindex_default_claim_estimator
sdk/python/pyproject.toml               # MODIFIED — add [llamaindex] extra
sdk/python/tests/integrations/
├── conftest.py                         # MODIFIED — add llamaindex stub fixture
├── test_llamaindex_unit.py             # NEW — unit (mock LlamaIndex types)
├── test_llamaindex_integration.py      # NEW — recorded-fixture integration
└── fixtures/llamaindex/                # NEW — recorded LLM event payloads
    ├── openai_gpt_4o_mini_allow.json
    ├── openai_gpt_4o_mini_deny.json
    ├── anthropic_sonnet_allow.json
    ├── gemini_flash_allow.json
    └── bedrock_converse_allow.json
deploy/demo/demo/run_demo.py            # MODIFIED — add agent_real_llamaindex branch
deploy/demo/tests/                      # existing
└── test_agent_real_llamaindex_demo.py  # NEW — demo regression
Makefile                                # MODIFIED — DEMO_MODE=agent_real_llamaindex target
docs/site/docs/integrations/llamaindex.md # NEW — user-facing docs page (2-path matrix)
README.md                               # MODIFIED — adapter table row
```

## 2. Extras + import guard

`pyproject.toml`:

```toml
[project.optional-dependencies]
llamaindex = [
  "llama-index-core>=0.12",
]
```

Note: provider sub-packages (`llama-index-llms-openai` etc.) are **NOT** declared as adapter dependencies. Operators install whichever vendor they use. The adapter loads against `llama-index-core` only.

Module-level guard mirrors `langchain.py`:

```python
try:
    from llama_index.core.callbacks.base_handler import BaseCallbackHandler
    from llama_index.core.callbacks.schema import CBEventType, EventPayload
except ImportError as exc:  # pragma: no cover
    raise ImportError(
        "spendguard.integrations.llamaindex requires the [llamaindex] extra. "
        "Install with: pip install 'spendguard-sdk[llamaindex]'"
    ) from exc
```

If LlamaIndex reorganizes between minor versions, fall back to `from llama_index.core.callbacks import ...` with an explicit `try/except ImportError` per path. No broad `except`.

## 3. Public surface (`llamaindex.py`)

### 3.1 Types

```python
RunIdFn = Callable[[Mapping[str, Any]], str]
"""Override for deriving run_id from event metadata."""

ClaimEstimator = Callable[[Mapping[str, Any]], list[common_pb2.BudgetClaim]]
"""Project BudgetClaim list from an event payload dict."""
```

### 3.2 Denied exception

```python
class SpendGuardLlamaIndexDenied(SpendGuardError):
    """Raised from on_event_start to short-circuit the LLM call.

    LlamaIndex has no documented "skip event" return channel; raising
    is the documented stop signal. Propagates as the call's terminal
    exception, observable by the user's own try/except + the framework
    tracer.
    """
    def __init__(self, reason_codes: list[str], decision_id: str = "") -> None:
        self.reason_codes = reason_codes
        self.decision_id = decision_id
        super().__init__(
            f"SpendGuard denied LLM call: {','.join(reason_codes) or 'BUDGET_EXHAUSTED'}"
        )
```

### 3.3 Pending-call state

```python
@dataclass(slots=True)
class _PendingCall:
    reservation_id: str
    decision_id: str
    step_id: str
    llm_call_id: str
    run_id: str
    signature: str
```

### 3.4 Handler skeleton

```python
class SpendGuardLlamaIndexHandler(BaseCallbackHandler):
    """LlamaIndex BaseCallbackHandler that gates CBEventType.LLM events.

    Drop-in via:
        Settings.callback_manager = CallbackManager([handler])

    Filters all non-LLM events. Stateless across requests; per-event
    state keyed by event_id in self._state.
    """

    def __init__(
        self,
        *,
        client: SpendGuardClient,
        budget_id: str,
        window_instance_id: str,
        unit: Any,             # common_pb2.UnitRef
        pricing: Any,          # common_pb2.PricingFreeze
        claim_estimator: ClaimEstimator | None = None,
        run_id_fn: RunIdFn | None = None,
    ) -> None:
        super().__init__(
            event_starts_to_ignore=[],
            event_ends_to_ignore=[],
        )
        self._client = client
        self._budget_id = budget_id
        self._window_instance_id = window_instance_id
        self._unit = unit
        self._pricing = pricing
        self._run_id_fn = run_id_fn
        self._trace_id: str | None = None
        self._state: dict[str, _PendingCall] = {}

        if claim_estimator is None:
            from ._default_estimator import llamaindex_default_claim_estimator
            # model lookup deferred to per-call payload inspection
            self._claim_estimator_factory = llamaindex_default_claim_estimator(
                budget_id=budget_id,
                window_instance_id=window_instance_id,
                unit=unit,
                model="",  # resolved per-event via payload[EventPayload.SERIALIZED]
            )
        else:
            self._claim_estimator_factory = claim_estimator
```

### 3.5 Event entry points

```python
def on_event_start(
    self,
    event_type: "CBEventType",
    payload: dict[str, Any] | None = None,
    event_id: str = "",
    parent_id: str = "",
    **kwargs: Any,
) -> str:
    if event_type != CBEventType.LLM:
        return event_id
    self._on_llm_start(payload or {}, event_id, parent_id)
    return event_id

def on_event_end(
    self,
    event_type: "CBEventType",
    payload: dict[str, Any] | None = None,
    event_id: str = "",
    **kwargs: Any,
) -> None:
    if event_type != CBEventType.LLM:
        return
    self._on_llm_end(payload or {}, event_id)

def start_trace(self, trace_id: str | None = None) -> None:
    self._trace_id = trace_id

def end_trace(
    self,
    trace_id: str | None = None,
    trace_map: dict[str, list[str]] | None = None,
) -> None:
    if trace_id is not None and trace_id == self._trace_id:
        self._trace_id = None
```

### 3.6 `_on_llm_start` (PRE)

```python
def _on_llm_start(
    self,
    payload: dict[str, Any],
    event_id: str,
    parent_id: str,
) -> None:
    run_id = self._resolve_run_id(payload, parent_id)
    signature = self._signature_for(payload)
    llm_call_id = str(derive_uuid_from_signature(signature, scope="llm_call_id"))
    decision_id = str(derive_uuid_from_signature(signature, scope="decision_id"))
    step_id = f"{run_id}:li-call:{signature[:16]}"
    idempotency_key = derive_idempotency_key(
        tenant_id=self._client.tenant_id,
        session_id=self._client.session_id,
        run_id=run_id,
        step_id=step_id,
        llm_call_id=llm_call_id,
        trigger="LLM_CALL_PRE",
    )

    claims = self._claim_estimator_factory(payload)

    try:
        outcome = self._client.request_decision_sync(
            trigger="LLM_CALL_PRE",
            run_id=run_id,
            step_id=step_id,
            llm_call_id=llm_call_id,
            tool_call_id="",
            decision_id=decision_id,
            route="llm.call",
            projected_claims=claims,
            idempotency_key=idempotency_key,
        )
    except DecisionDenied as exc:
        raise SpendGuardLlamaIndexDenied(
            reason_codes=list(getattr(exc, "reason_codes", []) or []),
            decision_id=decision_id,
        ) from exc

    if outcome.reservation_ids:
        self._state[event_id] = _PendingCall(
            reservation_id=outcome.reservation_ids[0],
            decision_id=outcome.decision_id,
            step_id=step_id,
            llm_call_id=llm_call_id,
            run_id=run_id,
            signature=signature,
        )
```

### 3.7 `_on_llm_end` (POST)

```python
def _on_llm_end(self, payload: dict[str, Any], event_id: str) -> None:
    pending = self._state.pop(event_id, None)
    if pending is None:
        # _on_llm_start never stashed state (DENY raised, or non-LLM
        # event reached us through a misconfig). Skip silently.
        return

    response = payload.get(EventPayload.RESPONSE)
    total_tokens = self._extract_total_tokens(response)
    provider_event_id = self._extract_provider_event_id(response)

    self._client.emit_llm_call_post_sync(
        run_id=pending.run_id,
        step_id=pending.step_id,
        llm_call_id=pending.llm_call_id,
        decision_id=pending.decision_id,
        reservation_id=pending.reservation_id,
        provider_reported_amount_atomic="",
        estimated_amount_atomic=str(total_tokens),
        unit=self._unit,
        pricing=self._pricing,
        provider_event_id=provider_event_id,
        outcome="SUCCESS",
    )
```

### 3.8 Usage extraction (vendor shapes)

```python
@staticmethod
def _extract_total_tokens(response: Any) -> int:
    raw = getattr(response, "raw", None)
    if raw is None:
        return 0

    # 1) OpenAI: response.raw["usage"]["total_tokens"]
    if isinstance(raw, Mapping):
        usage = raw.get("usage")
        if isinstance(usage, Mapping):
            total = usage.get("total_tokens")
            if isinstance(total, int) and total > 0:
                return total
            # 2) Anthropic: usage["input_tokens"] + ["output_tokens"]
            inp = usage.get("input_tokens")
            out = usage.get("output_tokens")
            if isinstance(inp, int) or isinstance(out, int):
                return int(inp or 0) + int(out or 0)
            # 4) Bedrock Converse: usage["inputTokens"] + ["outputTokens"]
            binp = usage.get("inputTokens")
            bout = usage.get("outputTokens")
            if isinstance(binp, int) or isinstance(bout, int):
                return int(binp or 0) + int(bout or 0)
        # 3) Gemini: usage_metadata.total_token_count
        meta = raw.get("usage_metadata")
        if isinstance(meta, Mapping):
            total = meta.get("total_token_count")
            if isinstance(total, int) and total > 0:
                return total
    return 0

@staticmethod
def _extract_provider_event_id(response: Any) -> str:
    raw = getattr(response, "raw", None)
    if isinstance(raw, Mapping):
        rid = raw.get("id") or raw.get("response_id")
        if isinstance(rid, str):
            return rid
    return ""
```

### 3.9 Signature + run_id resolution

```python
def _signature_for(self, payload: Mapping[str, Any]) -> str:
    serialized = payload.get(EventPayload.SERIALIZED) or {}
    model = (
        serialized.get("model") if isinstance(serialized, Mapping) else ""
    ) or ""
    messages = payload.get(EventPayload.MESSAGES) or payload.get(EventPayload.PROMPT) or ""
    body = f"{model}|{messages!r}"
    return hashlib.blake2b(body.encode("utf-8"), digest_size=16).hexdigest()

def _resolve_run_id(self, payload: Mapping[str, Any], parent_id: str) -> str:
    if self._run_id_fn is not None:
        return self._run_id_fn(payload)
    if self._trace_id:
        return self._trace_id
    if parent_id:
        return parent_id
    # Fallback: derive a stable UUID from the signature.
    return str(derive_uuid_from_signature(
        self._signature_for(payload), scope="run_id"
    ))
```

## 4. Default estimator (`_default_estimator.py` addition)

Add `llamaindex_default_claim_estimator(budget_id, window_instance_id, unit, model)` factory:

1. Inspects `payload[EventPayload.SERIALIZED].get("model")`. `"gpt-*"` / `"o1*"` → OpenAI family; `"claude-*"` → Anthropic; `"gemini-*"` → Gemini; `"anthropic.*"` / `"amazon.*"` → Bedrock; else → chars/4 fallback with `warnings.warn` once per (model, process).
2. Returns a callable `(payload) -> [BudgetClaim]` that walks `payload[EventPayload.MESSAGES]` (list of ChatMessage) or `payload[EventPayload.PROMPT]` (str) for text content.
3. Mirrors LangChain estimator's `chars // 4 → max(50, ...)` floor.

## 5. Demo wiring

`deploy/demo/demo/run_demo.py` adds:

```python
if DEMO_MODE == "agent_real_llamaindex":
    if not os.environ.get("OPENAI_API_KEY"):
        print("[demo] FATAL: OPENAI_API_KEY required for agent_real_llamaindex", file=sys.stderr)
        sys.exit(2)
    from spendguard.integrations.llamaindex import (
        SpendGuardLlamaIndexHandler, SpendGuardLlamaIndexDenied
    )
    from llama_index.core import Settings, VectorStoreIndex, Document
    from llama_index.core.callbacks import CallbackManager
    from llama_index.llms.openai import OpenAI

    handler = SpendGuardLlamaIndexHandler(
        client=client, budget_id=BUDGET_ID,
        window_instance_id=WINDOW_ID, unit=UNIT, pricing=PRICING,
    )
    Settings.callback_manager = CallbackManager([handler])
    Settings.llm = OpenAI(model="gpt-4o-mini")

    docs = [Document(text="The budget cap is 100 atomic units per window.")]
    index = VectorStoreIndex.from_documents(docs)
    qe = index.as_query_engine()

    # ALLOW path
    response = qe.query("What is the budget cap?")
    print(f"[demo] agent_real_llamaindex run completed: ALLOW path response={response}")

    # DENY path — exhaust budget then retry
    _exhaust_budget(client, BUDGET_ID)
    try:
        qe.query("What is the budget cap?")
    except SpendGuardLlamaIndexDenied as exc:
        print(f"[demo] agent_real_llamaindex run completed: DENY path (model not called) reasons={exc.reason_codes}")
```

`Makefile`:

```makefile
demo-up-agent-real-llamaindex:
\t@DEMO_MODE=agent_real_llamaindex $(MAKE) demo-up
```

A no-API-key variant `agent_real_llamaindex_stub` for CI replaces the `OpenAI(...)` with a `MockLLM` from `llama-index-core` (pre-baked response) — same handler wiring proves end-to-end without secrets.

## 6. Backward compat

- No proto changes. No DB migration. No public-API field rename.
- Existing 6 shipped adapters untouched.
- `_default_estimator.py` modification is purely additive — existing symbols keep their signatures.

## 7. Public exports

```python
# llamaindex.py
__all__ = [
    "ClaimEstimator",
    "RunIdFn",
    "SpendGuardLlamaIndexDenied",
    "SpendGuardLlamaIndexHandler",
]
```

No `RunContext` / `run_context` exports — LlamaIndex's per-event `event_id` + `start_trace`/`end_trace` already supply run_id; contextvar is unnecessary.

## 8. README adapter table row

```
| LlamaIndex | Python | `pip install 'spendguard-sdk[llamaindex]'` | `Settings.callback_manager = CallbackManager([SpendGuardLlamaIndexHandler(client=...)])` |
```

## 9. Two-path coverage matrix (docs)

| User pattern | Coverage | Install |
|--------------|----------|---------|
| `llama-index-llms-litellm` (`LiteLLM(...)`) | **D12** (LiteLLM SDK shim) | `pip install 'spendguard-sdk'` + `spendguard_litellm_shim.install(...)` |
| `llama-index-llms-openai` (`OpenAI(...)`) | **D27** | `pip install 'spendguard-sdk[llamaindex]'` |
| `llama-index-llms-anthropic` | **D27** | (same) |
| `llama-index-llms-gemini` / `-google-genai` | **D27** | (same) |
| `llama-index-llms-bedrock-converse` | **D27** | (same) |

Operators using mixed setups install BOTH D12 + D27. D12's contextvar recursion guard prevents double-reservation on the LiteLLM-routed inner call.

## 10. Out-of-scope reminders (lock here, audit at review)

- No TS LlamaIndex.TS port.
- No streaming intra-chunk gating (`CBEventType.CHUNK` is observational).
- No `CBEventType.EMBEDDING` / `RETRIEVE` gating.
- No `Settings` mutation by the SDK; operator wires `Settings.callback_manager` explicitly.
- No async overload — handler is fully sync, reuses `client.request_decision_sync` + `emit_llm_call_post_sync`.
