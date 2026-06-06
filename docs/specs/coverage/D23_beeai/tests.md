# D23 — BeeAI Framework `Emitter` adapter — Tests

All Python tests live under `sdk/python/tests/integrations/`. Demo regression lives under `deploy/demo/`.

## 1. Slice 1 — module skeleton & extras

### `tests/test_beeai_missing_extra.py`

- **t1**: Monkey-patch `sys.modules` to nuke `beeai_framework`. `import spendguard.integrations.beeai` raises `ImportError` whose message contains the install hint `pip install 'spendguard-sdk[beeai]'`. Same shape as `test_litellm_missing_extra.py`.

### `tests/integrations/test_beeai_skeleton.py`

- **t2**: `from spendguard.integrations.beeai import subscribe_spendguard, RunContext, run_context, BeeAiStartEvent` succeeds when `beeai-framework` is installed.
- **t3**: `subscribe_spendguard` is callable; signature matches §7 of `design.md` (introspect via `inspect.signature`). Kw-only args: `budget_id`, `window_instance_id`, `unit`, `pricing`, `claim_estimator`, `call_signature_fn`, `route`.
- **t4**: `pyproject.toml` parsed: `beeai` extra exists and pins `beeai-framework>=0.3,<1.0`.

## 2. Slice 2 — subscribe helper

### `tests/integrations/test_beeai_subscribe.py`

- **t5**: Construct a `MagicMock` agent with `agent.emitter = MagicMock(spec=Emitter)`. Call `subscribe_spendguard(agent, client, ...)`. Assert `agent.emitter.match` was called exactly once with two args (predicate, handler). Assert returned value is callable (unsubscribe). Assert `agent.emitter.match.return_value` flows back as the returned unsubscribe.
- **t6**: Predicate accepts `EventMeta(name="start", path="agent.react.llm.uuid.start")` → `True`; rejects `EventMeta(name="newToken", path=...)` → `False`; rejects `EventMeta(name="start", path="agent.react.tool.uuid.start")` → `False` (no `llm` segment).
- **t7**: Inflight map `InflightMap(capacity=3)`: `put` 4 entries, oldest evicted, `_LOG.warning` called exactly once (idempotent).
- **t8**: Inflight `pop` for a key that was put returns the entry; second `pop` returns `None` (single-use semantics).
- **t9**: `current_run_context()` raises `RuntimeError` when `subscribe_spendguard`'s handler runs outside `run_context(...)`. Triggered by manually invoking the handler with a fake `EventMeta(name="start", ...)`.

## 3. Slice 3 — reserve / commit / release wiring

### `tests/integrations/test_beeai_reserve_commit.py`

Drive the handler with a stub `SpendGuardClient` whose `request_decision` / `emit_llm_call_post` capture call args.

- **t10**: `start` → handler awaits `client.request_decision` with `trigger="LLM_CALL_PRE"`, `route="llm.call"` (default), `step_id` matches `f"{run_id}:beeai:{path_without_segment}"`, `projected_claims` equals what `claim_estimator(BeeAiStartEvent)` returned. After `start` resolves, inflight map has exactly 1 entry keyed by the stripped path.
- **t11**: `start` → DENY → `client.request_decision` raises `DecisionDenied`; the BeeAI handler **must** let it propagate (do not catch). Inflight map still empty.
- **t12**: `success` after `start` (same path with `.start` → `.success`) → handler awaits `client.emit_llm_call_post(outcome="SUCCESS", estimated_amount_atomic="42", ...)` (42 from `data.usage.total_tokens`). Inflight map empty afterwards.
- **t13**: `error` after `start` → handler awaits `client.emit_llm_call_post(outcome="PROVIDER_ERROR", estimated_amount_atomic="0", ...)`. Inflight map empty.
- **t14**: `success` **without** a prior `start` (e.g. unsubscribe happened mid-call) → handler is a no-op; no client call, no exception.
- **t15**: DEGRADE return from `request_decision` (mutation patch present) → handler logs warning, treats `apply_patch=False` (parity with LangChain `APPLY_FAILED`), still records inflight so the eventual `success` commits.
- **t16**: `usage` field missing from `success` data → `estimated_amount_atomic="0"`, no exception.
- **t17**: Provider-event-id resolution cascade: `data.id` → `data.response_id` → `""`. Each branch covered.
- **t18**: Idempotency key shape: `derive_idempotency_key` is called with `trigger="LLM_CALL_PRE"` and the same `tenant_id` / `session_id` from the client. Captured kwargs equal the LangChain adapter's pattern bit-for-bit.

### `tests/integrations/test_beeai_default_e2e.py`

Mirrors `test_langchain_default_e2e.py`. Uses a real `beeai_framework.agents.react.ReActAgent` with a `beeai_framework.backend.dummy.DummyChatModel` (the framework's built-in test double in 0.3) so no provider HTTP is needed and the test is hermetic.

- **t19**: Default `claim_estimator=None` → `_default_estimator` is auto-installed; `subscribe_spendguard` returns; `agent.run("hello")` under `run_context(RunContext(run_id=...))` completes; the captured-claim list is non-empty and uses the dispatched-by-model estimator for `gpt-4o-mini`-class (or `dummy`-class fallback).
- **t20**: Same flow with a `BUDGET_DENY` simulated in the stub client → `agent.run("...")` raises `DecisionDenied`; `DummyChatModel.create` is **never called** (assert via mock counter on the dummy model). This is the critical safety property — pre-call gating works.
- **t21**: Two sequential `agent.run` calls under the same `run_context` → two reserves + two commits; inflight map back to size 0 after both complete.

## 4. Slice 4 — demo + docs

### `deploy/demo/demo/verify_beeai.sql`

- **v1**: `SELECT count(*) FROM decision_outbox WHERE run_id = :run_id AND trigger = 'LLM_CALL_PRE' AND decision = 'ALLOW';` ⇒ 1 for ALLOW mode; `decision = 'DENY'` ⇒ 1 for deny mode.
- **v2**: `SELECT count(*) FROM outcome_outbox WHERE run_id = :run_id AND outcome = 'SUCCESS';` ⇒ 1 for ALLOW mode; ⇒ 0 for deny mode.
- **v3**: `SELECT count(*) FROM ledger_movements WHERE reservation_id = :reservation_id AND kind = 'COMMIT';` ⇒ 1 for ALLOW mode.

### `make demo-up DEMO_MODE=agent_real_beeai` (ALLOW)

- **r1**: `[demo] beeai run OK output='...' run_id=...` printed.
- **r2**: `psql -f verify_beeai.sql` returns expected `v1` / `v2` / `v3` row counts.
- **r3**: BeeAI's `DummyChatModel` (or `OpenAIChatModel` if `OPENAI_API_KEY` set) called exactly once on the ALLOW path.

### `make demo-up DEMO_MODE=agent_real_beeai_deny` (DENY)

- **r4**: `[demo] FATAL: DecisionDenied raised — pre-call gate fired as expected` printed; exit code `0` (deny is the success path here).
- **r5**: `decision_outbox` row with `decision='DENY'` exists; `outcome_outbox` has zero rows for that run_id.
- **r6**: The chat model's `create` method was never invoked — assert via env-injected counter or by inspecting whether the upstream stub container received any requests (`docker logs` zero matches for `/v1/chat/completions`).

## 5. Cross-cutting

- **lint**: `ruff check sdk/python/src/spendguard/integrations/beeai.py sdk/python/src/spendguard/integrations/_beeai_inflight.py` clean.
- **type**: `mypy sdk/python/src/spendguard/integrations/beeai.py` clean (strict opt-out matches sibling integrations).
- **import-error**: When the `beeai` extra is missing, `from spendguard.integrations.beeai import ...` raises `ImportError` with the install hint. Tested in §1 t1.
- **bound-test**: `InflightMap(capacity=10_000)` does not leak memory under 100k put/pop alternation. `pytest -q tests/integrations/test_beeai_subscribe.py::test_capacity_bound_memory`.
- **regression**: `pytest sdk/python/tests/integrations -q` total still passes (LangChain / openai_agents / agt suites untouched).
