# D22 — Tests

## 1. Layout

```
sdk/python/tests/integrations/
├── test_agno_pre_post.py
└── test_agno_default_estimator.py
```

Both files mark `pytestmark = pytest.mark.asyncio`. Existing harness `MockSpendGuardClient` from `sdk/python/tests/integrations/_mock_sidecar.py` is reused — no new mock infra.

## 2. Unit tests — `test_agno_pre_post.py` (≥ 20 cases)

| # | Case | Asserts |
|---|---|---|
| 1 | `pre_hook` calls `client.request_decision(trigger=LLM_CALL_PRE)` once with the right budget_id / window_instance_id / unit | Single `request_decision` invocation; argument equality |
| 2 | `pre_hook` raises `DecisionDenied` on STOP outcome | `pytest.raises(DecisionDenied)` |
| 3 | `pre_hook` raises `DecisionStopped` on STOP_RUN_PROJECTION | `pytest.raises(DecisionStopped)` |
| 4 | `pre_hook` records inflight entry keyed by `(run_id, signature)` | `_SHARED_INFLIGHT` contains key after call |
| 5 | `pre_hook` evicts oldest entry when map exceeds `_INFLIGHT_MAX` | Map size capped, oldest key absent |
| 6 | Calling `pre_hook` outside `run_context()` raises a clear `RuntimeError` mentioning `run_context(...)` | message contains "run_context" |
| 7 | Custom `call_signature_fn` is honoured (different sig → different inflight key) | Two distinct keys for two inputs |
| 8 | `claim_estimator` omitted → `agno_default_claim_estimator` factory dispatched and uses `agent.model.id` | `request_decision.projected_claims[0].amount_atomic` non-zero |
| 9 | Custom `claim_estimator` overrides default factory | Default factory never imported (assert via `unittest.mock.patch`) |
| 10 | `post_hook` calls `emit_llm_call_post(outcome="SUCCESS")` when run_response.metrics has total_tokens | One call; outcome=SUCCESS; total matches |
| 11 | `post_hook` reports `PROVIDER_ERROR` when `run_response.event == "RunError"` | outcome=PROVIDER_ERROR; total=0 |
| 12 | `post_hook` reports `PROVIDER_ERROR` when `run_response.error` is truthy | outcome=PROVIDER_ERROR |
| 13 | `post_hook` no-ops when inflight slot missing (logs warning once) | `emit_llm_call_post` NOT called; `caplog` captures warning |
| 14 | `post_hook` pops the inflight slot after commit | Map no longer contains key |
| 15 | Pre/post pair derives identical signatures from same `(agent, run_input)` | Inflight lookup hits |
| 16 | `pre_hook` async function declares `(agent, run_input)` parameter names verbatim | `inspect.signature(pre_callable).parameters` == `{"agent", "run_input"}` |
| 17 | `post_hook` async function declares `(agent, run_response)` parameter names | `inspect.signature(post_callable).parameters` == `{"agent", "run_response"}` |
| 18 | Two parallel runs with distinct `run_id` keep independent inflight slots | Two slots present concurrently; no cross-talk |
| 19 | `pre_hook` derives idempotency key via `derive_idempotency_key(trigger="LLM_CALL_PRE")` | Patched `derive_idempotency_key` called with exact kwargs |
| 20 | Streaming path (run_response with `is_stream=True`, partial metrics) commits SUCCESS once at completion event | Single `emit_llm_call_post`; `total_tokens` from final metrics |
| 21 | `pre_hook` propagates `ApprovalRequired` unchanged from `request_decision` | `pytest.raises(ApprovalRequired)`; inflight NOT populated |
| 22 | Missing `agent.model.id` (raw object) → default estimator falls back to family-default tokenizer without raising | `request_decision` still receives non-empty claim |

## 3. Integration test — real Agno `Agent` against stubbed OpenAI client

`test_agno_pre_post.py::test_real_agent_with_stub_openai` runs in CI:

1. Build a `MockSpendGuardClient` that auto-ALLOWS with one reservation.
2. Monkey-patch `agno.models.openai.OpenAIChat`'s internal `AsyncOpenAI` so `.chat.completions.create(...)` returns a canned `ChatCompletion` with `usage.total_tokens=87` and `id="chatcmpl-test"`.
3. Build `Agent(model=OpenAIChat(id="gpt-4o-mini"), pre_hooks=[pre()], post_hooks=[post()])`.
4. `async with run_context(RunContext(run_id="r-1")): await agent.arun("hello")`.
5. Assert:
   - `MockSpendGuardClient.request_decision` called exactly once with `route="llm.call"`, `trigger="LLM_CALL_PRE"`.
   - `MockSpendGuardClient.emit_llm_call_post` called exactly once with `estimated_amount_atomic="87"`, `outcome="SUCCESS"`, `provider_event_id == "chatcmpl-test"` (or the Agno-derived `run_id` when provider id is not surfaced).
   - The monkey-patched OpenAI client was called exactly once (no double dispatch).

A second variant `test_real_agent_deny_short_circuits` configures the mock to STOP — asserts the stubbed OpenAI client is **never** invoked and `Agent.arun` raises `DecisionDenied`.

A third variant `test_real_agent_provider_error` makes the OpenAI stub raise; asserts `emit_llm_call_post(outcome="PROVIDER_ERROR")`.

## 4. Default-estimator tests — `test_agno_default_estimator.py`

| # | Case | Asserts |
|---|---|---|
| 1 | `agno_default_claim_estimator` with `model="gpt-4o-mini"` and a str `run_input` builds one DEBIT claim with `input_tokens + output_tokens` | claim.amount_atomic parses to int ≥ input_tokens |
| 2 | Same with `run_input=[{"role": "user", "content": "..."}]` | identical magnitude as case 1 |
| 3 | Same with arbitrary object → str-coerced | claim emitted; no exception |
| 4 | Estimator reads `agent.model.id` at call time, overriding the constructor `model=""` | tokenizer for `gpt-4o-mini` used; `estimator_for_model` patched-spy fires with `"gpt-4o-mini"` |
| 5 | Estimator constructed with `model="claude-3-5-sonnet"` and agent without a `.model` attr → uses constructor model | `estimator_for_model("claude-3-5-sonnet")` spy fires |

## 5. Demo regression

`Makefile` adds `demo-agent-real-agno` target that runs `deploy/demo/demo/run_demo.py` with `DEMO_MODE=agent_real_agno` against the local sidecar; the slice 4 acceptance gate executes it inside `examples/agno-prehooks/docker-compose.yml` for an end-to-end PASS.

`scripts/check-demo-modes.py` (already enforces demo-mode dispatcher consistency) is extended in slice 4 to recognise `agent_real_agno` alongside `agent_real_langchain` / `agent_real_openai_agents` / `agent_real_agt`.

## 6. Coverage gate

`pytest --cov=spendguard.integrations.agno --cov-fail-under=85` runs in `sdk/python/tests/`. Branches for `_extract_usage` (RunError / metrics-shape variants) and inflight eviction (FIFO at boundary) MUST be hit by the table above.

## 7. CI matrix

The slice-4 demo runs against `agno==1.0.0` and `agno==1.0.*` (latest minor at slice ship time) in two parallel jobs to catch upstream signature drift early.
