# D10 — Tests

**Reads:** [`design.md`](design.md), [`implementation.md`](implementation.md), [`acceptance.md`](acceptance.md).

## 1. Test inventory

| Tier | Suite | Path | Count |
|------|-------|------|-------|
| Unit | `test_provider.py` | `plugins/dify/tests/test_provider.py` | 6 |
| Unit | `test_reservation.py` | `plugins/dify/tests/test_reservation.py` | 12 |
| Unit | `test_openai_invoke.py` | `plugins/dify/tests/test_openai_invoke.py` | 8 |
| Unit | `test_anthropic_invoke.py` | `plugins/dify/tests/test_anthropic_invoke.py` | 7 |
| Unit | `test_streaming.py` | `plugins/dify/tests/test_streaming.py` | 6 |
| Integration | `test_plugin_daemon_e2e.py` | `plugins/dify/tests/test_plugin_daemon_e2e.py` | 4 |
| Demo regression | `verify_step_dify_plugin.sql` | `deploy/demo/verify_step_dify_plugin.sql` | 6 SQL assertions |
| Demo driver | `run_demo.py::run_dify_plugin_real_mode` | `deploy/demo/demo/run_demo.py` | 3 steps × 4 assertions |

Total new test surface: ~780 LOC tests + ~100 LOC SQL gates + ~150 LOC demo driver.

## 2. Unit tests

The unit suite uses the `_fake_sidecar.py` fixture already in `sdk/python/tests/integrations/` (re-exported into the plugin test tree). The Dify SDK base classes (`LargeLanguageModel`, `ModelProvider`) are imported from the real `dify-plugin` package (no monkey-patching); the OpenAI / Anthropic upstream clients are replaced with `respx` / `pytest-anthropic-mock` HTTP fakes so no outbound network calls.

### 2.1 `test_provider.py` (6)

| # | Name | What it asserts |
|---|------|-----------------|
| P01 | `test_import_floor_raises_on_old_sdk` | When `dify_plugin` version metadata indicates < 0.2.0, plugin entry module raises `ImportError` with install hint. |
| P02 | `test_validate_credentials_issues_reserve_release_roundtrip` | `SpendGuardProvider.validate_credentials` calls `_DifyReservation.reserve` + `release_failure` with `amount_atomic=1`; fake sidecar logs both. |
| P03 | `test_validate_rejects_empty_upstream_api_key` | Missing `upstream_api_key` → Dify-shaped validation error before any sidecar call. |
| P04 | `test_validate_rejects_unsupported_upstream` | `upstream_provider=cohere` → validation error listing the supported set. |
| P05 | `test_validate_rejects_empty_budget_id` | Missing `spendguard_budget_id` → validation error naming the field. |
| P06 | `test_validate_credentials_propagates_sidecar_deny_as_invokeauth` | Sidecar configured to DENY the probe → `InvokeAuthorizationError` raised (operator sees it on Save). |

### 2.2 `test_reservation.py` (12)

| # | Name | What it asserts |
|---|------|-----------------|
| R01 | `test_env_missing_uds_raises` | Constructing `_DifyReservation` with `SPENDGUARD_SIDECAR_UDS` unset → `SpendGuardConfigError` naming the var. |
| R02 | `test_env_missing_tenant_raises` | Same for `SPENDGUARD_TENANT_ID`. |
| R03 | `test_reserve_builds_binding_from_credentials` | `reserve(ctx)` constructs `BudgetBinding` whose `budget_id` / `window_instance_id` / `unit.unit_id` match the credentials. |
| R04 | `test_reserve_request_decision_payload_shape` | Verifies the 12-field `decision_context` includes `integration=dify_plugin`, `mode=plugin`, `upstream_provider`, `workspace_id`. |
| R05 | `test_reserve_propagates_decision_denied` | Sidecar returns DENY → `DecisionDenied` raised; commit_success never called. |
| R06 | `test_reserve_degrade_fail_closed` | Sidecar returns DEGRADE → `SidecarUnavailable` raised. |
| R07 | `test_reserve_degrade_fail_open_dev_allows` | With `SPENDGUARD_DIFY_FAIL_OPEN=1`, DEGRADE returns a sentinel `ReservationHandle` with `reservation_id=""`; commit_success no-ops + WARN logged. |
| R08 | `test_commit_success_emits_real_usage` | `commit_success(handle, {"completion_tokens": 42, ...})` calls `emit_llm_call_post` with `estimated_amount_atomic="42"` + `outcome="SUCCESS"`. |
| R09 | `test_release_failure_swallows_release_rpc_errors` | Sidecar release-RPC raises → `release_failure` swallows + WARN; does NOT re-raise (TTL sweep backstop). |
| R10 | `test_release_failure_classifies_cancelled` | `release_failure(handle, asyncio.CancelledError())` emits `outcome="CANCELLED"`. |
| R11 | `test_idempotency_key_derivation_stable` | Two `reserve` calls with identical `(tenant, session, run, step, llm_call)` produce identical idempotency keys (regression for double-charge). |
| R12 | `test_estimator_snapshot_frozen_before_sidecar_await` | Mutating the operator's claim object during the sidecar await does NOT change the snapshot used by streaming-fallback commit (mirrors `litellm.py:373-386`). |

### 2.3 `test_openai_invoke.py` (8)

| # | Name | What it asserts |
|---|------|-----------------|
| O01 | `test_invoke_allow_path_calls_upstream_after_reserve` | Order: `_reservation.reserve` (recorded at T0) → upstream HTTP (recorded at T1 > T0) → `_reservation.commit_success` (T2 > T1). `asyncio.Event` strict-order check. |
| O02 | `test_invoke_deny_does_not_call_upstream` | Fake sidecar DENY → `respx` records **zero** outbound to `api.openai.com`. **Critical invariant.** |
| O03 | `test_invoke_translates_openai_response_to_dify_llmresult` | `LLMResult.message.content` == upstream `choices[0].message.content`; `LLMResult.usage.completion_tokens` matches. |
| O04 | `test_invoke_real_usage_drives_commit` | `commit_success` receives `completion_tokens` from `response.usage`, NOT estimator. |
| O05 | `test_invoke_upstream_apierror_releases_reservation` | Upstream `openai.APIError` → `release_failure` called with outcome=FAILURE; raised as Dify `InvokeError`. |
| O06 | `test_invoke_upstream_authenticationerror_translates` | Upstream `openai.AuthenticationError` → `InvokeAuthorizationError` (Dify class). |
| O07 | `test_invoke_uses_upstream_base_url` | When `credentials.upstream_base_url="https://my-proxy.example/v1"`, OpenAI client targets that base URL. |
| O08 | `test_invoke_gemini_stub_raises` | `upstream_provider=gemini` in v1 → `InvokeError("upstream provider gemini not supported in this plugin version")`. |

### 2.4 `test_anthropic_invoke.py` (7)

| # | Name | What it asserts |
|---|------|-----------------|
| A01 | `test_invoke_allow_path_calls_anthropic` | Same ordering invariant as O01 but against `api.anthropic.com`. |
| A02 | `test_invoke_deny_does_not_call_anthropic` | DENY → zero `api.anthropic.com` hits. |
| A03 | `test_message_format_translates_system_role` | Dify `prompt_messages` with `role=system` translates to Anthropic's top-level `system` field, not into `messages`. |
| A04 | `test_invoke_uses_input_output_tokens` | Reconciler reads `response.usage.input_tokens` + `output_tokens`; commit `amount_atomic` matches the sum (per binding unit). |
| A05 | `test_get_num_tokens_dispatches_to_sidecar_count_tokens` | `get_num_tokens(model, prompt_messages)` calls sidecar `count_tokens` UDS RPC with `provider=anthropic` and returns its result. |
| A06 | `test_invoke_upstream_overload_translates` | Anthropic `529 Overloaded` → Dify `InvokeServerUnavailableError`. |
| A07 | `test_invoke_unsupported_model_raises_dify_error` | Model not in `models/llm/spendguard.yaml` → Dify `InvokeBadRequestError`. |

### 2.5 `test_streaming.py` (6)

| # | Name | What it asserts |
|---|------|-----------------|
| S01 | `test_stream_yields_chunks_before_commit` | Iterating the generator yields ≥ 1 `LLMResultChunk` BEFORE `commit_success` is invoked. |
| S02 | `test_stream_commit_uses_final_usage_chunk` | When upstream emits a final usage frame, commit uses it (NOT estimator). |
| S03 | `test_stream_no_usage_estimator_fallback_logs_warn` | Upstream omits usage frame → commit uses estimator snapshot + WARN log substring `falling back to estimator` present. |
| S04 | `test_stream_caller_cancellation_releases_reservation` | Closing the generator mid-stream triggers `release_failure(outcome=CANCELLED)`. |
| S05 | `test_stream_upstream_error_mid_stream_releases` | Upstream raises mid-stream → `release_failure(outcome=FAILURE)` + error surfaces as Dify `InvokeError`. |
| S06 | `test_stream_anthropic_message_delta_usage_parsed` | Anthropic's `message_delta` event with `usage.output_tokens` is captured by `_streaming_accumulator`. |

## 3. Integration tests — `test_plugin_daemon_e2e.py` (4)

Boots the plugin daemon in a subprocess (`python -m dify_plugin.runtime`) listening on a unix socket, then issues plugin RPC calls modelled on what Dify core sends. Uses the `_fake_sidecar.py` fixture for the SpendGuard side and `respx` for the upstream HTTP. No Docker, no Dify core.

| # | Name | What it asserts |
|---|------|-----------------|
| E01 | `test_plugin_loads_and_validates_credentials` | Daemon boots, `validate_credentials` RPC succeeds, fake sidecar logs the 1-token probe. |
| E02 | `test_plugin_invoke_allow_end_to_end` | RPC `invoke` (non-streaming) → fake sidecar reserve → respx upstream hit → fake sidecar commit. Strict order via `asyncio.Event`. |
| E03 | `test_plugin_invoke_deny_blocks_upstream` | Configure fake sidecar DENY → RPC returns Dify `InvokeAuthorizationError`; respx logs **zero** hits. |
| E04 | `test_plugin_invoke_stream_e2e` | RPC `invoke` (streaming) → chunks flow → end-of-stream commit row in fake sidecar. |

E02 / E03 use a strict-ordering `asyncio.Event` set by the fake sidecar on `RequestDecision`. The respx handler awaits the event briefly and records `out-of-order` if it was not set before the handler fires.

## 4. Demo regression — `verify_step_dify_plugin.sql`

```sql
-- D10_DIFY: at least 2 decisions carrying dify_plugin integration tag.
DO $$ DECLARE c INT; BEGIN
  SELECT COUNT(*) INTO c FROM audit_outbox
   WHERE decision_context->>'integration' = 'dify_plugin'
     AND decision_context->>'mode' = 'plugin'
     AND created_at > now() - interval '5 minute';
  IF c < 2 THEN  -- 1 ALLOW + 1 DENY minimum
    RAISE EXCEPTION 'D10_DIFY_GATE: expected >=2 dify decisions, got %', c;
  END IF;
  RAISE NOTICE 'D10_DIFY OK: dify decisions=%', c;
END; $$;

-- D10_DIFY: at least one DENY decision.
DO $$ DECLARE c INT; BEGIN
  SELECT COUNT(*) INTO c FROM audit_outbox
   WHERE decision_context->>'integration' = 'dify_plugin'
     AND cloudevent_payload::jsonb->'data'->>'decision' = 'DENY';
  IF c < 1 THEN
    RAISE EXCEPTION 'D10_DIFY_GATE: expected >=1 DENY, got %', c;
  END IF;
END; $$;

-- D10_DIFY: commit row present pairing with reservation row.
DO $$ DECLARE c INT; BEGIN
  SELECT COUNT(*) INTO c FROM commits
   WHERE latest_state IN ('estimated', 'provider_reported')
     AND created_at > now() - interval '5 minute';
  IF c < 1 THEN
    RAISE EXCEPTION 'D10_DIFY_GATE: no commit rows';
  END IF;
END; $$;

-- D10_DIFY: streaming step produced an end-of-stream commit row.
DO $$ DECLARE c INT; BEGIN
  SELECT COUNT(*) INTO c FROM audit_outbox
   WHERE decision_context->>'integration' = 'dify_plugin'
     AND decision_context->>'stream' = 'true';
  IF c < 1 THEN
    RAISE EXCEPTION 'D10_DIFY_GATE: no streaming decision audited';
  END IF;
END; $$;

-- D10_DIFY: canonical_events received the dify events (outbox forwarder ran).
DO $$ DECLARE c INT; BEGIN
  SELECT COUNT(*) INTO c FROM canonical_events
   WHERE source_integration = 'dify_plugin';
  IF c < 1 THEN
    RAISE EXCEPTION 'D10_DIFY_GATE: canonical_events empty for dify_plugin';
  END IF;
END; $$;

-- D10_DIFY: provider stub counter matches expected ALLOW count.
-- (The demo driver writes a stub_hits field into decision_context for the
--  DENY step; assert the counter never incremented on a DENY.)
DO $$ DECLARE bad INT; BEGIN
  SELECT COUNT(*) INTO bad FROM audit_outbox
   WHERE decision_context->>'integration' = 'dify_plugin'
     AND cloudevent_payload::jsonb->'data'->>'decision' = 'DENY'
     AND (decision_context->>'stub_hits')::int > 0;
  IF bad > 0 THEN
    RAISE EXCEPTION 'D10_DIFY_GATE: % DENY decisions saw upstream hits', bad;
  END IF;
END; $$;
```

## 5. Demo driver — `run_dify_plugin_real_mode`

3 steps, each with 4 assertions. Layout mirrors `run_litellm_real_mode` for consistency.

| Step | Body | Asserts |
|------|------|---------|
| 1 ALLOW | small prompt, fits budget; POST `/v1/chat-messages` blocking | Dify HTTP 200; upstream stub counter +1; fake sidecar RequestDecision recorded **before** stub hit; commit row visible with real `completion_tokens` |
| 2 DENY | prompt configured to exceed budget OR demo-only `force_hard_cap=1` flag | Dify HTTP 403; upstream stub counter unchanged; DENY decision audited; **no** commit row added |
| 3 STREAM | `response_mode=streaming` | Dify HTTP 200 SSE; ≥3 chunks received; stub counter +1; end-of-stream commit row with `decision_context.stream = true` |

The driver writes `[demo] dify_plugin_real ALL 3 steps PASS (ALLOW + DENY + STREAM)` on success; gate failure exits 7.

## 6. Negative test surface (must-not-regress)

| What | Why | Where |
|------|-----|-------|
| Upstream hit on DENY | Worst correctness bug | O02 + E03 + demo driver step 2 + verify SQL stub_hits assertion |
| Reserve fires AFTER upstream | Cancels D10 thesis | O01 + E02 strict-ordering event |
| Commit uses estimator instead of real usage on success | Charges wrong amount | O04 + S02 |
| Streaming cancel does not release reservation | Reservation leaks | S04 |
| `validate_credentials` only probes upstream and not sidecar | Operators won't catch SpendGuard wiring errors at install | P02 |
| Plugin daemon process leaks the operator's `upstream_api_key` to logs | Secret leak | Manual + lint rule on the plugin tree (no `log.info(credentials)` substring) |

## 7. Performance budgets (informational)

| Op | Target | Source |
|----|--------|--------|
| `_invoke` overhead (plugin layer only, excluding sidecar gRPC) | < 2ms p99 | pure delegation cost |
| Cold-start plugin daemon boot | < 1500ms | dify-plugin runtime + spendguard SDK import + lazy SidecarClient init |
| Streaming first-byte added latency vs raw upstream | < 50ms p95 | reserve roundtrip |

Verified manually post-merge; not a CI gate (no perf CI infra in tree today).

## 8. CI integration

`plugins/dify/tests/` runs under a new pytest job in the existing GitHub Actions matrix (`pytest plugins/dify`). The integration test E0* boots the plugin runtime subprocess in CI (no Dify core image required, so the matrix stays fast).

`make demo-up DEMO_MODE=dify_plugin_real` runs in the existing `e2e-demo` matrix as a new cell. The Dify images (`langgenius/dify-api:1.0`, `langgenius/dify-worker:1.0`) are large (~2GB combined); the cell uses GH Actions cache for the docker layer to avoid pulling on every run.

The `dify-plugin-publish.yml` workflow runs only on tags `dify-plugin-v*`; not in PR CI.
