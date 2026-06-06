# D27 — LlamaIndex `CallbackManager` Adapter

**Status:** Spec — Tier 3, build plan §2.3.
**Owner:** AI Engineer.
**Priors:** [`langchain.py`](../../../../sdk/python/src/spendguard/integrations/langchain.py) (callback-as-class); [`openai_agents.py`](../../../../sdk/python/src/spendguard/integrations/openai_agents.py) (PRE/POST). **Transitive:** [`D12`](../D12_litellm_sdk_shim/design.md) covers the LiteLLM-routed path.

## 1. Problem

LlamaIndex (Apache-2.0, ~47k stars) is the dominant RAG framework. Its provider integrations (`llama-index-llms-openai`, `-anthropic`, `-gemini`, `-bedrock`) call vendor SDKs directly — bypassing every existing SpendGuard adapter. The `-litellm` package routes through LiteLLM (D12 covers transitively), but `VectorStoreIndex(...).as_query_engine().query(...)` against `OpenAI(model="gpt-4o-mini")` has zero pre-call refusal today.

LlamaIndex's extension surface is `llama_index.core.callbacks.CallbackManager`. Handlers extend `BaseCallbackHandler` and receive `on_event_start(event_type, payload, event_id, parent_id, **kwargs)` + `on_event_end(...)`. `CBEventType.LLM` carries prompt/messages on start and the response on end — the substrate we need for reserve / commit. `Settings.callback_manager` is global; `CallbackManager([handler])` propagates to every `LLM`, no model subclassing.

## 2. Goals

1. New `spendguard.integrations.llamaindex` module exporting `SpendGuardLlamaIndexHandler` — a `BaseCallbackHandler` subclass gating `CBEventType.LLM` only.
2. PRE: `request_decision(trigger="LLM_CALL_PRE")`. On `DecisionDenied`, raise `SpendGuardLlamaIndexDenied` — LlamaIndex has no "skip event" return channel; raising IS the stop signal.
3. POST: extract usage from `EventPayload.RESPONSE` and call `emit_llm_call_post(outcome="SUCCESS")`.
4. Vendor coverage by response shape: openai / anthropic / gemini / bedrock-converse. No class-name parsing.
5. Two-path matrix documented: **LiteLLM-routed** → D12 transitive (non-goal). **Direct** → `Settings.callback_manager` registration (D27).
6. New extras `spendguard-sdk[llamaindex]` requiring `llama-index-core >= 0.12`. Import error names the install command.
7. `DEMO_MODE=agent_real_llamaindex`: `OpenAI(model="gpt-4o-mini")` + `VectorStoreIndex` end-to-end, proving ALLOW commits and DENY short-circuits **before** the OpenAI HTTP call.

## 3. Non-goals

- **TypeScript LlamaIndex.TS port.** Different callback shape; Frontend Developer deliverable.
- **Streaming intra-chunk gating.** `CBEventType.CHUNK` is observational; we commit at turn boundary (parity with LangChain / openai-agents).
- **`CBEventType.EMBEDDING` / `RETRIEVE` gating.** Out-of-budget by SpendGuard policy.
- **Re-gating the LiteLLM path.** Operators install D12; D27's PRE still fires for the LlamaIndex event but D12's contextvar recursion guard prevents double-reservation on the inner LiteLLM call.

## 4. Architecture

```
Settings.callback_manager = CallbackManager([SpendGuardLlamaIndexHandler(...)])
  ↓ query_engine.query("...")
  ↓ LLM._llm_predict / _chat
on_event_start(event_type=CBEventType.LLM, payload={MESSAGES|PROMPT, SERIALIZED}, event_id="evt-uuid", parent_id="...")
  → _on_llm_start
     → client.request_decision_sync(LLM_CALL_PRE, claims=…)
     → ALLOW: stash reservation in self._state[event_id]
     → DENY:  raise SpendGuardLlamaIndexDenied(reason_codes=…)
  ↓ (if ALLOW) provider HTTP
on_event_end(event_type=CBEventType.LLM, payload={RESPONSE}, event_id="evt-uuid")
  → _on_llm_end → emit_llm_call_post(SUCCESS, total_tokens=…) → cleanup self._state[event_id]
```

## 5. Key decisions

- **`event_id` is the cross-call correlation key.** Per-event state in `self._state: dict[str, _PendingCall]`, cleaned in `on_event_end`. LlamaIndex guarantees unique event_ids per concurrent call.
- **Filter on `event_type == CBEventType.LLM` at handler entry.** All other event types are early-return no-ops (one enum compare for 80%+ events).
- **DENY raises, never returns.** Matches D11 LiteLLM proxy pattern. `SpendGuardLlamaIndexDenied(SpendGuardError)` carries `reason_codes`.
- **`run_id` defaults to `trace_id` → `parent_id` → derived UUID.** Operators override via `run_id_fn` for cross-framework correlation (mirrors D19).
- **Usage extraction by response shape.** OpenAI `response.raw["usage"]["total_tokens"]` → Anthropic `input_tokens + output_tokens` → Gemini `usage_metadata.total_token_count` → Bedrock Converse `usage.inputTokens + outputTokens` → 0. Commit still fires on miss for audit-chain completeness.
- **Default claim estimator** in `_default_estimator.py` as `llamaindex_default_claim_estimator(...)`. Dispatched off `payload[EventPayload.SERIALIZED]["model"]`; chars/4 fallback with `warnings.warn` once per (model, process).
- **No `Settings` mutation by SpendGuard.** Operator installs `Settings.callback_manager = CallbackManager([handler])` explicitly. No auto-registration.
- **Sync callbacks.** LlamaIndex invokes `on_event_start` synchronously from inside async LLM calls. We reuse `client.request_decision_sync` (already exists for AGT adapter); handler is fully sync — no async overload.

## 6. Slice plan

| Slice | Title | Size |
|-------|-------|------|
| `COV_D27_S1_module_skeleton` | `spendguard.integrations.llamaindex` module + `[llamaindex]` extra + import-error guard + `SpendGuardLlamaIndexDenied` | S |
| `COV_D27_S2_handler_class` | `SpendGuardLlamaIndexHandler(BaseCallbackHandler)` + event_type filter + per-event_id state + `start_trace`/`end_trace` hooks | M |
| `COV_D27_S3_pre_post_wiring` | `_on_llm_start` reserve + DENY raise; `_on_llm_end` commit/cleanup; usage extraction across 4 vendors | M |
| `COV_D27_S4_tests` | Unit (mock LlamaIndex types) + integration (recorded `llama-index-llms-openai` + `-anthropic` fixtures) | M |
| `COV_D27_S5_demo_and_docs` | `DEMO_MODE=agent_real_llamaindex` Makefile + driver (VectorStoreIndex over 1-doc corpus) + `docs/site/docs/integrations/llamaindex.md` (incl. 2-path matrix) + README row | M |

5 slices, S/M only, ~1100 LOC (~500 impl + 450 test + 150 docs/yaml). No proto / DB changes.

## 7. Interfaces

```python
class SpendGuardLlamaIndexHandler(BaseCallbackHandler):
    def __init__(self, *, client, budget_id, window_instance_id, unit,
                 pricing, claim_estimator=None, run_id_fn=None) -> None: ...
    def on_event_start(self, event_type, payload=None, event_id="",
                       parent_id="", **kwargs) -> str: ...
    def on_event_end(self, event_type, payload=None, event_id="",
                     **kwargs) -> None: ...
    def start_trace(self, trace_id=None) -> None: ...
    def end_trace(self, trace_id=None, trace_map=None) -> None: ...

class SpendGuardLlamaIndexDenied(SpendGuardError):
    reason_codes: list[str]
```

```python
from llama_index.core import Settings, VectorStoreIndex
from llama_index.core.callbacks import CallbackManager
from llama_index.llms.openai import OpenAI
from spendguard.integrations.llamaindex import SpendGuardLlamaIndexHandler

handler = SpendGuardLlamaIndexHandler(client=..., budget_id=..., ...)
Settings.callback_manager = CallbackManager([handler])
Settings.llm = OpenAI(model="gpt-4o-mini")
index = VectorStoreIndex.from_documents(docs)
response = index.as_query_engine().query("What is the budget cap?")
```

## 8. Open questions (locked at spec write)

1. **Callback API stability.** `BaseCallbackHandler` + `CBEventType.LLM` + `EventPayload.MESSAGES`/`RESPONSE` stable across 0.12.0–0.12.50. Locked floor `>= 0.12`.
2. **Stop signal.** `on_event_start` returns `str` (event_id passthrough); raising is the documented stop signal. Locked.
3. **Sync-from-async.** Reuse `client.request_decision_sync` from AGT adapter. Locked.
4. **Concurrency.** `event_id` uniqueness guaranteed by LlamaIndex; `self._state` dict keyed by it. Locked.
