# D19 — Tests

Backlinks: [`design.md`](./design.md), [`implementation.md`](./implementation.md), [`acceptance.md`](./acceptance.md), [`review-standards.md`](./review-standards.md).

## 1. Coverage matrix

| ID | Test name | Layer | Purpose |
|----|-----------|-------|---------|
| U01 | `test_import_error_when_google_adk_missing` | Unit | Module import raises `ImportError` with `pip install 'spendguard-sdk[adk]'` substring when `google.adk` is unavailable. |
| U02 | `test_callback_init_defaults_estimator_for_gemini_model` | Unit | When `claim_estimator=None`, `_default_estimator.adk_default_claim_estimator` is wired with the Gemini family for `"gemini-2.0-flash"`. |
| U03 | `test_callback_init_defaults_estimator_for_litellm_openai` | Unit | When the model is `LiteLlm("openai/gpt-4o-mini")` the default estimator picks the OpenAI family (prefix strip). |
| U04 | `test_callback_init_defaults_estimator_for_unknown_warns_once` | Unit | Unknown model → chars/4 fallback + single `warnings.warn` per (model, process). |
| U05 | `test_call_dispatch_request_routes_to_before` | Unit | `await cb(ctx, LlmRequest(...))` invokes `_before` and returns `None` on ALLOW. |
| U06 | `test_call_dispatch_response_routes_to_after` | Unit | `await cb(ctx, LlmResponse(...))` invokes `_after` and returns `None`. |
| U07 | `test_before_allow_stashes_reservation_in_state` | Unit | After ALLOW, `ctx.state["spendguard.reservation_id"]` and the three companion keys are populated. |
| U08 | `test_before_deny_returns_llm_response_and_marks_state` | Unit | `DecisionDenied(reason_codes=["BUDGET_EXHAUSTED"])` → returns `LlmResponse(error_code="SPENDGUARD_DENY", error_message=...)` and sets `ctx.state["spendguard.denied"] = True`. No reservation_id is stored. |
| U09 | `test_before_uses_invocation_id_as_default_run_id` | Unit | Without `run_id_fn`, `request_decision(run_id=ctx.invocation_id, ...)` is called. |
| U10 | `test_before_uses_run_id_fn_override` | Unit | With `run_id_fn=lambda c: "fixed-run"`, `request_decision(run_id="fixed-run", ...)` is called. |
| U11 | `test_after_commit_extracts_gemini_total_token_count` | Unit | `LlmResponse(usage_metadata=GeminiUsage(total_token_count=42))` → `emit_llm_call_post(estimated_amount_atomic="42", ...)`. |
| U12 | `test_after_commit_extracts_gemini_split_tokens` | Unit | `prompt_token_count=10 + candidates_token_count=15` → `estimated_amount_atomic="25"`. |
| U13 | `test_after_commit_extracts_openai_total_tokens` | Unit | `usage_metadata.total_tokens=99` (LiteLlm OpenAI shape) → `estimated_amount_atomic="99"`. |
| U14 | `test_after_commit_falls_back_to_zero_on_missing_usage` | Unit | No `usage_metadata` → commit still fires with `estimated_amount_atomic="0"`. |
| U15 | `test_after_skips_commit_when_denied_flag_set` | Unit | `ctx.state["spendguard.denied"] = True` → `_after` is a no-op (no `emit_llm_call_post`, no `release_reservation`). |
| U16 | `test_after_skips_commit_when_pre_state_missing` | Unit | If `ctx.state` lacks `reservation_id` (e.g. _before never ran), `_after` returns without RPCs. |
| U17 | `test_signature_stable_across_repeated_calls` | Unit | Two calls with the same `LlmRequest.contents + model` produce identical `signature` (and thus identical `step_id` / `llm_call_id` / `decision_id`). |
| U18 | `test_signature_differs_when_model_changes` | Unit | Same `contents`, different `model` → different signature. |
| U19 | `test_deny_response_contains_reason_codes` | Unit | Multiple reason codes → `error_message` contains all of them, comma-joined. |
| U20 | `test_extract_provider_event_id_falls_back_to_empty` | Unit | LlmResponse without `response_id` / `id` → `provider_event_id=""` in commit. |
| I01 | `test_integration_allow_flow_with_recorded_gemini_fixture` | Integration | Replays `fixtures/adk/gemini_2_0_flash_allow.json` through a mock ADK loop: PRE reserve fires → mock model returns recorded LlmResponse → POST commit fires with `total_tokens=42`. |
| I02 | `test_integration_deny_flow_with_recorded_gemini_fixture` | Integration | Replays `fixtures/adk/gemini_2_0_flash_deny.json`: sidecar returns DENY → callback returns `LlmResponse(error_code="SPENDGUARD_DENY")` → mock ADK runner observes the deny response → mock Gemini transport is **never** called (assert call count = 0). |
| I03 | `test_integration_allow_flow_with_recorded_litellm_fixture` | Integration | Replays `fixtures/adk/litellm_gpt_4o_mini_allow.json`: PRE reserve → mock LiteLlm-wrapped model returns OpenAI-shape LlmResponse → POST commit fires with `total_tokens=99` (from OpenAI shape). |
| I04 | `test_integration_run_id_derived_from_invocation_id` | Integration | ADK runner assigns `invocation_id="run-abc"`; sidecar receives `run_id="run-abc"` in both PRE and POST. |
| I05 | `test_integration_concurrent_runs_dont_cross_state` | Integration | Two `asyncio.gather`-ed runs with distinct `CallbackContext` instances: each commits its own `reservation_id`; no state leakage. |
| D01 | `test_demo_agent_real_adk_allow_path` | Demo | `make demo-up DEMO_MODE=agent_real_adk` boots; the driver makes one ALLOW call against the live Gemini API; SQL verify shows `decision_id` + `reservation_id` + `commit_id` in `audit_outbox` with `outcome='SUCCESS'`. |
| D02 | `test_demo_agent_real_adk_deny_path` | Demo | Same boot; the driver forces a DENY (budget set to 0); SQL verify shows decision row with `verdict='DENY'` and **no** corresponding commit row; mock-Gemini HTTP counter (egress stub) is 0 on that call. |

## 2. File layout

```
sdk/python/tests/integrations/
├── conftest.py                       # MODIFIED — add adk fixture
├── test_adk_unit.py                  # NEW — U01-U20
├── test_adk_integration.py           # NEW — I01-I05
└── fixtures/adk/
    ├── gemini_2_0_flash_allow.json
    ├── gemini_2_0_flash_deny.json
    └── litellm_gpt_4o_mini_allow.json
deploy/demo/tests/                    # existing
└── test_agent_real_adk_demo.py       # NEW — D01-D02
```

## 3. Mock ADK type strategy

ADK types we touch: `CallbackContext`, `LlmRequest`, `LlmResponse`. Unit tests **do not** depend on `google-adk` at runtime — they mock those three with `unittest.mock.MagicMock(spec=...)` resolved at import time. If `google-adk` isn't installed, the spec falls back to a `SimpleNamespace`-based stub:

```python
# conftest.py
@pytest.fixture
def mock_adk_types():
    try:
        from google.adk.agents.callback_context import CallbackContext
        from google.adk.models import LlmRequest, LlmResponse
        return CallbackContext, LlmRequest, LlmResponse
    except ImportError:
        # Fallback: shape-compatible stubs
        class StubCtx: state: dict[str, Any]; invocation_id: str
        class StubReq: contents: list; model: str
        class StubResp:
            usage_metadata: Any; response_id: str | None
            error_code: str | None; error_message: str | None
        return StubCtx, StubReq, StubResp
```

This keeps the unit suite green even if a developer runs `pytest` without the `[adk]` extra. The integration suite (`test_adk_integration.py`) skips entirely with `pytest.importorskip("google.adk")` if the package isn't present.

## 4. Recorded fixtures

Each fixture JSON has shape:

```json
{
  "request": {
    "model": "gemini-2.0-flash",
    "contents": [{"role": "user", "parts": [{"text": "..."}]}]
  },
  "response": {
    "usage_metadata": {"prompt_token_count": 12, "candidates_token_count": 30, "total_token_count": 42},
    "response_id": "recorded-response-001",
    "candidates": [{"content": {"parts": [{"text": "..."}]}}]
  }
}
```

Fixtures are **recorded once** (one-time live API call captured to disk + checked in), not regenerated per test. The recorder script is `sdk/python/tests/integrations/fixtures/adk/_record.py` — gated on `RECORD_FIXTURES=1` env var and `GOOGLE_API_KEY` presence. Default test run **never** hits the live API.

## 5. Sidecar fake

Integration tests run against the existing `FakeSpendGuardServer` already used by LangChain / openai_agents integration tests (`sdk/python/tests/_fakes/fake_server.py`). No new fake.

PRE/POST verification asserts:

- `RequestDecisionRequest.trigger == LLM_CALL_PRE`
- `RequestDecisionRequest.route == "llm.call"`
- `RequestDecisionRequest.run_id == <expected>`
- `RequestDecisionRequest.projected_claims[0].direction == DEBIT`
- `EmitLlmCallPostRequest.outcome == "SUCCESS"`
- `EmitLlmCallPostRequest.estimated_amount_atomic == "<computed>"`

For DENY, no `EmitLlmCallPost` arrives, and a `release_reservation` may arrive but is **optional** (since deny carries no reservation_id, defense-in-depth release is also valid as no-op).

## 6. Demo regression

Demo tests live under `deploy/demo/tests/` and are invoked by the existing `make demo-test` target. The new `test_agent_real_adk_demo.py`:

- Boots `demo-up DEMO_MODE=agent_real_adk` via subprocess (timeout 120s).
- Asserts log line `[demo] agent_real_adk run completed: ALLOW path`.
- Asserts log line `[demo] agent_real_adk run completed: DENY path (model not called)`.
- Runs the canonical `verify.sql` (existing SQL surface) and asserts:
  - `audit_outbox` has at least 2 rows with `trigger='LLM_CALL_PRE'` for the demo `session_id`.
  - At least 1 has `verdict='ALLOW'` with a paired commit row.
  - At least 1 has `verdict='DENY'` with **no** paired commit row.
- Stop counts the mock-egress stub HTTP hit count on the DENY path — must be 0.

## 7. Test execution

```bash
# Fast (unit only, no [adk] extra required):
pytest sdk/python/tests/integrations/test_adk_unit.py -v

# Full (requires [adk] extra; skips otherwise):
pip install -e 'sdk/python[adk]'
pytest sdk/python/tests/integrations/test_adk_unit.py \
       sdk/python/tests/integrations/test_adk_integration.py -v

# Demo regression (requires Docker + GOOGLE_API_KEY):
DEMO_MODE=agent_real_adk make demo-up
make demo-test
make demo-down
```

## 8. Anti-tests (explicitly out of scope)

- **No live Gemini API in CI.** Fixtures only.
- **No streaming intra-turn tests.** Streaming is non-goal per design §3.
- **No tool callback tests.** Tool gating is non-goal.
- **No multi-language tests.** Python only.
- **No backpressure / rate-limit tests on the sidecar side.** Sidecar already has those; the adapter is a thin shim.
