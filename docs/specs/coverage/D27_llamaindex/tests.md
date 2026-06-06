# D27 — Tests

Backlinks: [`design.md`](./design.md), [`implementation.md`](./implementation.md), [`acceptance.md`](./acceptance.md), [`review-standards.md`](./review-standards.md).

## 1. Coverage matrix

| ID | Test name | Layer | Purpose |
|----|-----------|-------|---------|
| U01 | `test_import_error_when_llama_index_core_missing` | Unit | Module import raises `ImportError` with `pip install 'spendguard-sdk[llamaindex]'` substring when `llama_index.core` is unavailable. |
| U02 | `test_handler_init_defaults_estimator_for_openai_model` | Unit | When `claim_estimator=None`, `_default_estimator.llamaindex_default_claim_estimator` is wired with the OpenAI family for `model="gpt-4o-mini"`. |
| U03 | `test_handler_init_defaults_estimator_for_anthropic_model` | Unit | `model="claude-3-5-sonnet"` selects Anthropic family. |
| U04 | `test_handler_init_defaults_estimator_for_gemini_model` | Unit | `model="gemini-1.5-flash"` selects Gemini family. |
| U05 | `test_handler_init_defaults_estimator_for_bedrock_model` | Unit | `model="anthropic.claude-3-sonnet-20240229-v1:0"` selects Bedrock family. |
| U06 | `test_handler_init_defaults_estimator_for_unknown_warns_once` | Unit | Unknown model → chars/4 fallback + single `warnings.warn` per (model, process). |
| U07 | `test_non_llm_events_are_no_op` | Unit | `on_event_start(CBEventType.EMBEDDING, ...)`, `CBEventType.RETRIEVE`, `CBEventType.CHUNK`, `CBEventType.QUERY` all return `event_id` unchanged and make zero sidecar calls. |
| U08 | `test_on_event_start_allow_stashes_pending_call` | Unit | After ALLOW, `self._state[event_id]` is a `_PendingCall` with non-empty `reservation_id`, `decision_id`, `step_id`, `llm_call_id`. |
| U09 | `test_on_event_start_deny_raises_spendguard_denied` | Unit | `DecisionDenied(reason_codes=["BUDGET_EXHAUSTED"])` → raises `SpendGuardLlamaIndexDenied(reason_codes=["BUDGET_EXHAUSTED"])`. `self._state` is empty after. |
| U10 | `test_on_event_start_returns_event_id_unchanged` | Unit | Returns `event_id` (per base class contract) on both ALLOW and non-LLM paths. |
| U11 | `test_run_id_resolution_uses_trace_id_when_set` | Unit | After `handler.start_trace("trace-xyz")`, `request_decision_sync(run_id="trace-xyz", ...)` is called. |
| U12 | `test_run_id_resolution_falls_back_to_parent_id` | Unit | No trace + `parent_id="par-1"` → `request_decision_sync(run_id="par-1", ...)`. |
| U13 | `test_run_id_resolution_uses_run_id_fn_override` | Unit | `run_id_fn=lambda p: "fixed-run"` wins over trace and parent_id. |
| U14 | `test_run_id_resolution_derives_uuid_when_no_inputs` | Unit | No trace, no parent_id, no run_id_fn → derived UUID from signature; identical for identical payload. |
| U15 | `test_on_event_end_commit_extracts_openai_total_tokens` | Unit | `response.raw={"usage": {"total_tokens": 42}}` → `emit_llm_call_post_sync(estimated_amount_atomic="42", ...)`. |
| U16 | `test_on_event_end_commit_extracts_anthropic_input_output_tokens` | Unit | `response.raw={"usage": {"input_tokens": 10, "output_tokens": 15}}` → `estimated_amount_atomic="25"`. |
| U17 | `test_on_event_end_commit_extracts_gemini_total_token_count` | Unit | `response.raw={"usage_metadata": {"total_token_count": 33}}` → `estimated_amount_atomic="33"`. |
| U18 | `test_on_event_end_commit_extracts_bedrock_converse_tokens` | Unit | `response.raw={"usage": {"inputTokens": 7, "outputTokens": 8}}` → `estimated_amount_atomic="15"`. |
| U19 | `test_on_event_end_falls_back_to_zero_on_missing_usage` | Unit | `response.raw={}` → commit still fires with `estimated_amount_atomic="0"`. |
| U20 | `test_on_event_end_no_op_when_state_missing` | Unit | If `self._state` lacks `event_id` (e.g. DENY raised in start, or non-LLM event), `on_event_end` returns silently. No RPCs. |
| U21 | `test_on_event_end_cleans_up_state` | Unit | After commit, `self._state` no longer contains `event_id` (memory hygiene). |
| U22 | `test_signature_stable_across_repeated_calls` | Unit | Two `on_event_start` invocations with identical payload (model + messages) produce identical `signature` → identical `decision_id` / `llm_call_id`. |
| U23 | `test_signature_differs_when_model_changes` | Unit | Same messages, different `payload[EventPayload.SERIALIZED]["model"]` → different signature. |
| U24 | `test_concurrent_events_dont_cross_state` | Unit | Two `on_event_start` calls with distinct `event_id` → two distinct `_PendingCall` entries; each `on_event_end` commits its own reservation_id. |
| U25 | `test_start_trace_and_end_trace_lifecycle` | Unit | `start_trace("t1")` sets internal trace; `end_trace("t1")` clears it; `end_trace("t2")` (mismatched id) does NOT clear. |
| I01 | `test_integration_openai_allow_flow_with_recorded_fixture` | Integration | Replays `fixtures/llamaindex/openai_gpt_4o_mini_allow.json` through a mock LlamaIndex event loop: PRE reserve fires → mock LLM returns recorded response → POST commit fires with `total_tokens=42`. |
| I02 | `test_integration_openai_deny_flow_with_recorded_fixture` | Integration | Replays `openai_gpt_4o_mini_deny.json`: sidecar returns DENY → handler raises `SpendGuardLlamaIndexDenied` → mock LLM transport is **never** called (assert call count = 0). |
| I03 | `test_integration_anthropic_allow_flow_with_recorded_fixture` | Integration | `anthropic_sonnet_allow.json`: PRE reserve → mock Anthropic response → POST commit with `total_tokens = input + output`. |
| I04 | `test_integration_gemini_allow_flow_with_recorded_fixture` | Integration | `gemini_flash_allow.json`: PRE reserve → mock Gemini response → POST commit with `total_token_count`. |
| I05 | `test_integration_bedrock_allow_flow_with_recorded_fixture` | Integration | `bedrock_converse_allow.json`: PRE reserve → mock Bedrock Converse response → POST commit with `inputTokens + outputTokens`. |
| I06 | `test_integration_vector_index_query_end_to_end` | Integration | Builds a `VectorStoreIndex.from_documents([Document(text=...)])` with `MockLLM` as `Settings.llm`, runs `.as_query_engine().query("...")`. Sidecar receives exactly 1 PRE + 1 POST for the synthesis call (retriever events filtered). |
| I07 | `test_integration_run_id_derived_from_start_trace` | Integration | `Settings.callback_manager.start_trace_with_id("run-abc")` (LlamaIndex API); sidecar receives `run_id="run-abc"` in both PRE and POST. |
| I08 | `test_integration_concurrent_query_engines_dont_cross_state` | Integration | Two `concurrent.futures`-dispatched `query_engine.query(...)` calls (LlamaIndex sync API) — each commits its own reservation_id; no state leakage; final `self._state` is empty. |
| D01 | `test_demo_agent_real_llamaindex_allow_path` | Demo | `make demo-up DEMO_MODE=agent_real_llamaindex_stub` boots; driver runs one ALLOW query against the MockLLM; SQL verify shows `decision_id` + `reservation_id` + `commit_id` in `audit_outbox` with `outcome='SUCCESS'`. |
| D02 | `test_demo_agent_real_llamaindex_deny_path` | Demo | Same boot; driver forces a DENY (budget set to 0); SQL verify shows decision row with `verdict='DENY'` and **no** paired commit row; MockLLM call counter = 0 on that query. |

## 2. File layout

```
sdk/python/tests/integrations/
├── conftest.py                                # MODIFIED — add llamaindex_types fixture
├── test_llamaindex_unit.py                    # NEW — U01-U25
├── test_llamaindex_integration.py             # NEW — I01-I08
└── fixtures/llamaindex/
    ├── openai_gpt_4o_mini_allow.json
    ├── openai_gpt_4o_mini_deny.json
    ├── anthropic_sonnet_allow.json
    ├── gemini_flash_allow.json
    └── bedrock_converse_allow.json
deploy/demo/tests/                             # existing
└── test_agent_real_llamaindex_demo.py         # NEW — D01-D02
```

## 3. Mock LlamaIndex type strategy

Types we touch: `BaseCallbackHandler`, `CBEventType`, `EventPayload`. Unit tests **do not** depend on `llama-index-core` at runtime — they mock those three with a fallback `SimpleNamespace`-based stub when the package is missing:

```python
# conftest.py
@pytest.fixture
def llamaindex_types():
    try:
        from llama_index.core.callbacks.base_handler import BaseCallbackHandler
        from llama_index.core.callbacks.schema import CBEventType, EventPayload
        return BaseCallbackHandler, CBEventType, EventPayload
    except ImportError:
        # Fallback shims — enum-shape stubs
        class _StubEventType:
            LLM = "llm"; EMBEDDING = "embedding"; RETRIEVE = "retrieve"
            CHUNK = "chunk"; QUERY = "query"; NODE_PARSING = "node_parsing"
        class _StubPayload:
            MESSAGES = "messages"; PROMPT = "prompt"
            RESPONSE = "response"; SERIALIZED = "serialized"
        class _StubBase:
            def __init__(self, **_kw: Any) -> None: ...
            def on_event_start(self, *a: Any, **kw: Any) -> str: return ""
            def on_event_end(self, *a: Any, **kw: Any) -> None: ...
        return _StubBase, _StubEventType, _StubPayload
```

Keeps the unit suite green without the `[llamaindex]` extra. The integration suite (`test_llamaindex_integration.py`) skips entirely with `pytest.importorskip("llama_index.core")` when absent.

## 4. Recorded fixtures

Each fixture JSON has shape:

```json
{
  "model": "gpt-4o-mini",
  "payload_start": {
    "messages": [{"role": "user", "content": "What is the budget cap?"}],
    "serialized": {"model": "gpt-4o-mini", "class_name": "OpenAI"}
  },
  "payload_end": {
    "response": {
      "raw": {
        "id": "chatcmpl-abc",
        "usage": {"prompt_tokens": 12, "completion_tokens": 30, "total_tokens": 42}
      }
    }
  }
}
```

Anthropic / Gemini / Bedrock fixtures vary `payload_end.response.raw.usage` shape per design §5 / impl §3.8. Fixtures are **recorded once** (one-time live API capture, checked in). Recorder script `sdk/python/tests/integrations/fixtures/llamaindex/_record.py` is gated on `RECORD_FIXTURES=1` + the appropriate `*_API_KEY`. Default test run **never** hits live APIs.

## 5. Sidecar fake

Integration tests reuse the existing `FakeSpendGuardServer` (`sdk/python/tests/_fakes/fake_server.py`). No new fake.

PRE/POST verification asserts:

- `RequestDecisionRequest.trigger == LLM_CALL_PRE`
- `RequestDecisionRequest.route == "llm.call"`
- `RequestDecisionRequest.run_id == <expected>`
- `RequestDecisionRequest.projected_claims[0].direction == DEBIT`
- `EmitLlmCallPostRequest.outcome == "SUCCESS"`
- `EmitLlmCallPostRequest.estimated_amount_atomic == "<computed>"`

For DENY, no `EmitLlmCallPost` arrives. `release_reservation` is **not** expected (start raised before reservation was stashed).

## 6. Mock LLM strategy for integration tests

For I01-I05 we mock the provider LLM class (`OpenAI` / `Anthropic` / etc.) so `_predict` returns a hand-built `ChatResponse(raw=fixture["payload_end"]["response"]["raw"])`. The callback manager dispatch is the real LlamaIndex code path — only the provider HTTP is faked.

For I06-I08 we use the in-tree `llama_index.core.llms.MockLLM` which echoes a deterministic response shape with `raw={}` (empty usage) — exercises the `total_tokens=0` fallback path in `_extract_total_tokens` while still proving end-to-end query flow.

## 7. Demo regression

Demo tests live under `deploy/demo/tests/` and are invoked by `make demo-test`. `test_agent_real_llamaindex_demo.py`:

- Boots `demo-up DEMO_MODE=agent_real_llamaindex_stub` via subprocess (timeout 120s).
- Asserts log lines:
  - `[demo] agent_real_llamaindex run completed: ALLOW path`
  - `[demo] agent_real_llamaindex run completed: DENY path (model not called)`
- Runs canonical `verify.sql` and asserts:
  - `audit_outbox` has ≥ 2 rows with `trigger='LLM_CALL_PRE'` for the demo `session_id`.
  - ≥ 1 has `verdict='ALLOW'` with a paired commit row.
  - ≥ 1 has `verdict='DENY'` with **no** paired commit row.
- Stub MockLLM call counter on DENY path = 0.

## 8. Test execution

```bash
# Fast (unit only, no [llamaindex] extra required):
pytest sdk/python/tests/integrations/test_llamaindex_unit.py -v

# Full (requires [llamaindex] extra; skips otherwise):
pip install -e 'sdk/python[llamaindex]'
pytest sdk/python/tests/integrations/test_llamaindex_unit.py \
       sdk/python/tests/integrations/test_llamaindex_integration.py -v

# Demo regression (no API key required — stub variant):
DEMO_MODE=agent_real_llamaindex_stub make demo-up
make demo-test
make demo-down

# Live demo (requires OPENAI_API_KEY):
OPENAI_API_KEY=... DEMO_MODE=agent_real_llamaindex make demo-up && make demo-test
```

## 9. Anti-tests (explicitly out of scope)

- **No live provider APIs in CI.** Fixtures only.
- **No streaming intra-chunk tests.** Streaming is non-goal per design §3.
- **No embedding / retrieve gating tests.** Filter coverage (U07) is the only embedding-related test.
- **No multi-language tests.** Python only.
- **No backpressure / rate-limit tests on the sidecar side.** The adapter is a thin shim.
- **No transitive D12 coverage tests.** `llama-index-llms-litellm` users install D12; cross-installation interaction is tested in D12's own suite, not here.
