# D36 — Tests

**Reads:** [`design.md`](design.md), [`implementation.md`](implementation.md), [`acceptance.md`](acceptance.md).

## 1. Test inventory

| Tier | Suite | Path | Count |
|------|-------|------|-------|
| Unit | `test_component_skeleton.py` | `plugins/langflow/tests/test_component_skeleton.py` | 6 |
| Unit | `test_build_model.py` | `plugins/langflow/tests/test_build_model.py` | 9 |
| Unit | `test_run_context_autobind.py` | `plugins/langflow/tests/test_run_context_autobind.py` | 5 |
| Unit | `test_install_script.py` | `plugins/langflow/tests/test_install_script.py` | 5 |
| Demo regression | `verify_step_langflow.sql` | `deploy/demo/verify_step_langflow.sql` | 6 SQL assertions |
| Demo driver | `run_demo.py::run_langflow_real_mode` | `deploy/demo/demo/run_demo.py` | 3 steps × 4 assertions |

Total new test surface: ~370 LOC tests + ~100 LOC SQL gates + ~150 LOC demo driver.

## 2. Unit tests

The unit suite uses the `_fake_sidecar.py` fixture already in `sdk/python/tests/integrations/` (re-exported into the plugin test tree). The Langflow base class (`Component`) is imported from the real `langflow` package (no monkey-patching). The wrapped LangChain model is `langchain_core.language_models.fake_chat_models.FakeListChatModel` so no outbound HTTP fires.

### 2.1 `test_component_skeleton.py` (6)

| # | Name | What it asserts |
|---|------|-----------------|
| C01 | `test_import_floor_raises_on_old_langflow` | When `langflow` version metadata indicates < 1.8.0, component module raises `ImportError` with install hint pointing at `pip install 'langflow>=1.8.0'`. |
| C02 | `test_class_introspection_lists_eight_inputs` | `SpendGuardChatModelWrapper.inputs` lists exactly the 8 inputs (`inner`, `sidecar_uds_path`, `tenant_id`, `budget_id`, `window_instance_id`, `unit_token_kind`, `model_family`, `claim_estimator_chars_per_token`) with correct types. |
| C03 | `test_inner_handle_input_type` | The `inner` input is a `HandleInput` with `input_types == ["LanguageModel"]` (Langflow's standard chat-model handle). |
| C04 | `test_output_is_languagemodel_handle` | The output is a single `Output(name="model", method="build_model", types=["LanguageModel"])`. |
| C05 | `test_required_inputs_marked_required` | `inner`, `sidecar_uds_path`, `tenant_id`, `budget_id`, `window_instance_id` carry `required=True`. The 3 advanced inputs carry `advanced=True`. |
| C06 | `test_display_metadata_present` | `display_name == "SpendGuard Budget Gate"`; `icon == "shield"`; `documentation` URL present; `description` ≥ 50 chars. |

### 2.2 `test_build_model.py` (9)

| # | Name | What it asserts |
|---|------|-----------------|
| B01 | `test_build_returns_spendguard_chat_model` | With `inner=FakeListChatModel(responses=["hi"])` and fake-sidecar UDS, `build_model()` returns a `SpendGuardChatModel` whose `.inner is the FakeListChatModel`. |
| B02 | `test_build_calls_connect_and_handshake` | Fake sidecar records exactly one `connect()` + one `handshake()` call before `build_model()` returns. |
| B03 | `test_build_propagates_canvas_inputs_into_client` | `SpendGuardClient.tenant_id == self.tenant_id`; the constructed `unit_ref.unit_id == f"{model_family}.{unit_token_kind}"`. |
| B04 | `test_build_default_estimator_uses_chars_per_token_input` | With `claim_estimator_chars_per_token=8` and a 32-char message, the estimator produces `max(50, 32 // 8) = 50` (floor); with 800 chars produces `100`. |
| B05 | `test_missing_uds_raises_valueerror` | Empty `sidecar_uds_path` input + `SPENDGUARD_SIDECAR_UDS` unset → `ValueError` whose message names both the canvas input and the env var. |
| B06 | `test_uds_env_fallback` | Empty `sidecar_uds_path` input + `SPENDGUARD_SIDECAR_UDS=/tmp/fake.sock` env → build succeeds using env value. |
| B07 | `test_running_loop_raises` | Calling `build_model()` from inside a running `asyncio` loop → `RuntimeError` whose message contains `"running event loop"` and tells the operator to file a Langflow-version bug. |
| B08 | `test_invoke_after_build_routes_through_sidecar` | `await built_model.ainvoke([HumanMessage("hello")])` → fake sidecar logs `request_decision` (PRE) then `emit_llm_call_post` (POST) in order. `asyncio.Event` strict-order check. |
| B09 | `test_invoke_deny_does_not_call_inner` | Fake sidecar configured to DENY → `built_model.ainvoke(...)` raises `DecisionDenied` AND the wrapped `FakeListChatModel._agenerate` was never invoked (regression for INV-1). |

### 2.3 `test_run_context_autobind.py` (5)

| # | Name | What it asserts |
|---|------|-----------------|
| A01 | `test_autobind_enters_when_no_context` | After `install_autobind`, calling `_agenerate` with NO active `run_context` works (no `RuntimeError`); fake sidecar sees a `request_decision` whose `run_id` matches the flow-id-derived pattern (`langflow-...:1`). |
| A02 | `test_caller_bound_context_wins` | If caller wraps the call in `async with run_context(RunContext(run_id="caller-rid"))`, the recorded `run_id` is `"caller-rid"`, NOT the auto-bound one. |
| A03 | `test_autobind_run_id_increments_per_call` | Two sequential `_agenerate` calls produce `run_id` ending in `:1` then `:2`. |
| A04 | `test_flow_id_fallback_when_graph_absent` | When `flow_id=None`, auto-bind uses a `uuid7()`-style base; recorded `run_id` matches `langflow-<uuid>` shape (UUID v4 regex). |
| A05 | `test_autobind_preserves_inner_a_generate_signature` | The patched `_agenerate` accepts `messages, stop, run_manager, **kwargs` identically to the SDK's original. `functools.wraps` preserves docstring + name. |

### 2.4 `test_install_script.py` (5)

| # | Name | What it asserts |
|---|------|-----------------|
| I01 | `test_install_copies_component_and_metadata_to_target` | `spendguard-langflow-install --target $TMPDIR` copies `spendguard_chat_model_wrapper.py` AND its metadata YAML to the target tree under the expected subpath. |
| I02 | `test_install_refuses_existing_without_force` | When the target file exists, exits non-zero with a clear "use --force to overwrite" message. |
| I03 | `test_install_force_overwrites` | `--force` overwrites and exits 0. |
| I04 | `test_install_refuses_system_path` | Target path starting with `/usr` / `/bin` / `/etc` / `/System` → refuses with explicit error (operator must use their own `LANGFLOW_COMPONENTS_PATH`). |
| I05 | `test_install_target_auto_creates_parents` | Target subdirectory doesn't exist → script creates parents, copies, exits 0. |

## 3. Demo regression — `verify_step_langflow.sql`

```sql
-- D36_LANGFLOW: at least 2 decisions carrying langflow integration tag.
DO $$ DECLARE c INT; BEGIN
  SELECT COUNT(*) INTO c FROM audit_outbox
   WHERE decision_context->>'integration' = 'langchain'
     AND decision_context->>'source' = 'langflow'
     AND created_at > now() - interval '5 minute';
  IF c < 2 THEN  -- 1 ALLOW + 1 DENY minimum
    RAISE EXCEPTION 'D36_LANGFLOW_GATE: expected >=2 langflow decisions, got %', c;
  END IF;
  RAISE NOTICE 'D36_LANGFLOW OK: langflow decisions=%', c;
END; $$;

-- D36_LANGFLOW: at least one DENY decision.
DO $$ DECLARE c INT; BEGIN
  SELECT COUNT(*) INTO c FROM audit_outbox
   WHERE decision_context->>'source' = 'langflow'
     AND cloudevent_payload::jsonb->'data'->>'decision' = 'DENY';
  IF c < 1 THEN
    RAISE EXCEPTION 'D36_LANGFLOW_GATE: expected >=1 DENY, got %', c;
  END IF;
END; $$;

-- D36_LANGFLOW: commit row present pairing with reservation row.
DO $$ DECLARE c INT; BEGIN
  SELECT COUNT(*) INTO c FROM commits
   WHERE latest_state IN ('estimated', 'provider_reported')
     AND created_at > now() - interval '5 minute';
  IF c < 1 THEN
    RAISE EXCEPTION 'D36_LANGFLOW_GATE: no commit rows';
  END IF;
END; $$;

-- D36_LANGFLOW: streaming step produced an end-of-stream commit row.
DO $$ DECLARE c INT; BEGIN
  SELECT COUNT(*) INTO c FROM audit_outbox
   WHERE decision_context->>'source' = 'langflow'
     AND decision_context->>'stream' = 'true';
  IF c < 1 THEN
    RAISE EXCEPTION 'D36_LANGFLOW_GATE: no streaming decision audited';
  END IF;
END; $$;

-- D36_LANGFLOW: canonical_events received the langflow events.
DO $$ DECLARE c INT; BEGIN
  SELECT COUNT(*) INTO c FROM canonical_events
   WHERE source_integration = 'langchain'
     AND payload::jsonb->>'source' = 'langflow';
  IF c < 1 THEN
    RAISE EXCEPTION 'D36_LANGFLOW_GATE: canonical_events empty for langflow';
  END IF;
END; $$;

-- D36_LANGFLOW: provider stub counter never increments on a DENY.
DO $$ DECLARE bad INT; BEGIN
  SELECT COUNT(*) INTO bad FROM audit_outbox
   WHERE decision_context->>'source' = 'langflow'
     AND cloudevent_payload::jsonb->'data'->>'decision' = 'DENY'
     AND (decision_context->>'stub_hits')::int > 0;
  IF bad > 0 THEN
    RAISE EXCEPTION 'D36_LANGFLOW_GATE: % DENY decisions saw upstream hits', bad;
  END IF;
END; $$;
```

## 4. Demo driver — `run_langflow_real_mode`

3 steps, each with 4 assertions. Layout mirrors `run_dify_plugin_real_mode` for consistency.

| Step | Body | Asserts |
|------|------|---------|
| 1 ALLOW | small prompt, fits budget; POST `/api/v1/run/{flow_id}` against Langflow's API | Langflow HTTP 200; upstream stub counter +1; fake sidecar `request_decision` recorded **before** stub hit; commit row visible with real `completion_tokens` |
| 2 DENY | prompt configured to exceed budget OR demo-only `force_hard_cap=1` budget setting | Langflow non-2xx (Langflow's translation of `DecisionDenied`); upstream stub counter unchanged; DENY decision audited; **no** commit row added |
| 3 STREAM | `stream=true` on the flow-run request | Langflow HTTP 200 SSE; ≥3 chunks received; stub counter +1; end-of-stream commit row with `decision_context.stream = true` |

The driver writes `[demo] langflow_real ALL 3 steps PASS (ALLOW + DENY + STREAM)` on success; gate failure exits 7.

## 5. Negative test surface (must-not-regress)

| What | Why | Where |
|------|-----|-------|
| Upstream hit on DENY | Worst correctness bug | B09 + demo step 2 + verify SQL `stub_hits` assertion |
| Reserve fires AFTER upstream `_agenerate` | Cancels D36 thesis | B08 strict-ordering event |
| Commit uses estimator instead of real usage on success | Charges wrong amount | covered by SDK tests; demo step 1 verifies real `completion_tokens` |
| Auto-bind clobbers caller-bound run_context | Breaks composability with code-driven Langflow calls | A02 |
| `build_model` deadlocks under running loop | Hangs Langflow on flow start | B07 |
| Install script writes to a path outside operator's tree | Supply-chain footgun | I04 |
| `inner=None` accepted at build | Crashes downstream Langflow node opaquely | C05 |

## 6. Performance budgets (informational)

| Op | Target | Source |
|----|--------|--------|
| `build_model` latency (canvas → wrapped model returned) | < 200ms p99 | dominated by sidecar `handshake()` UDS roundtrip |
| Per-invocation overhead (wrapper layer only, excluding sidecar gRPC) | < 1ms p99 | pure delegation cost — reuses existing langchain.py path |
| Streaming first-byte added latency vs raw upstream | < 50ms p95 | reserve roundtrip |

Verified manually post-merge; not a CI gate.

## 7. CI integration

`plugins/langflow/tests/` runs under a new pytest job in the existing GitHub Actions matrix (`pytest plugins/langflow`). No subprocess Langflow daemon required for unit tests — the `Component` base class instantiates cleanly with the inputs dict.

`make demo-up DEMO_MODE=langflow_real` runs in the existing `e2e-demo` matrix as a new cell. The Langflow image (`langflowai/langflow:1.8`) is ~1 GB; the cell uses GH Actions cache for the docker layer to avoid pulling on every run.

The `langflow-component-publish.yml` workflow runs only on tags `langflow-component-v*`; not in PR CI.

## 8. Fixtures and harness reuse

- **Fake sidecar:** symlink `plugins/langflow/tests/_fake_sidecar.py` → `sdk/python/tests/integrations/_fake_sidecar.py` (no copy; the symlink is the canonical reuse pattern from D10).
- **Fake LangChain model:** `langchain_core.language_models.fake_chat_models.FakeListChatModel` — already a transitive dep via `spendguard-sdk[langchain]`.
- **Strict-order `asyncio.Event`:** copied from the LangChain integration's test suite. Records `out-of-order` if the upstream HTTP / inner `_agenerate` fires before the sidecar `request_decision` flag.
