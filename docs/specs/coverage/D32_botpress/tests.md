# D32 — Tests

**Reads:** [`design.md`](design.md), [`implementation.md`](implementation.md), [`acceptance.md`](acceptance.md).

## 1. Test inventory

| Tier | Suite | Path | Count |
|------|-------|------|-------|
| Unit | `reservation.test.ts` | `integrations/botpress/tests/reservation.test.ts` | 11 |
| Unit | `beforeAiGeneration.test.ts` | `integrations/botpress/tests/beforeAiGeneration.test.ts` | 7 |
| Unit | `afterAiGeneration.test.ts` | `integrations/botpress/tests/afterAiGeneration.test.ts` | 8 |
| Unit | `adapter.test.ts` | `integrations/botpress/tests/adapter.test.ts` | 6 |
| Unit | `lifecycle.test.ts` | `integrations/botpress/tests/lifecycle.test.ts` | 5 |
| Integration | `integration-v12.test.ts` | `integrations/botpress/tests/integration-v12.test.ts` | 4 |
| Demo regression | `verify_step_botpress.sql` | `deploy/demo/verify_step_botpress.sql` | 6 SQL assertions |
| Demo driver | `run_demo.py::run_botpress_real_mode` | `deploy/demo/demo/run_demo.py` | 3 steps × 4 assertions |

Total new test surface: ~840 LOC tests + ~100 LOC SQL gates + ~150 LOC demo driver.
Total unit + integration test count: **41 tests** (37 unit + 4 integration).

## 2. Unit tests

The unit suite uses an in-process `msw` HTTP server (`_mockSidecar.ts`) that simulates the D09 SLICE 1 HTTP companion endpoints (`/v1/decision`, `/v1/trace`). The `@botpress/sdk` types are imported from the real package, but hooks are invoked directly with synthesised hook-input objects (no Botpress runtime in the unit tier).

### 2.1 `reservation.test.ts` (11)

| # | Name | What it asserts |
|---|------|-----------------|
| R01 | `test_construct_requires_sidecar_url` | Constructing `SpendGuardReservation({ sidecarUrl: "" })` throws `SpendGuardConfigError` naming the field. |
| R02 | `test_construct_requires_budget_ids` | Same for missing `spendguardBudgetId` / `spendguardWindowInstanceId`. |
| R03 | `test_reserve_builds_binding_from_config_and_ctx` | `reserve(ctx)` constructs `BudgetBinding` whose `budget_id` / `window_instance_id` / `unit.unit_id` match the configuration; `session_id` matches `ctx.conversationId`. |
| R04 | `test_reserve_request_decision_payload_shape` | Verifies the 12-field `decision_context` includes `integration=botpress`, `mode=integration_sdk`, `upstream_provider`, `bot_id`, `conversation_id`. |
| R05 | `test_reserve_propagates_decision_denied` | Sidecar returns DENY → `DecisionDenied` raised; commitSuccess never called. |
| R06 | `test_reserve_degrade_fail_closed` | Sidecar returns DEGRADE → `SidecarUnavailable` raised. |
| R07 | `test_reserve_degrade_fail_open_dev_allows` | With `SPENDGUARD_BOTPRESS_FAIL_OPEN=1`, DEGRADE returns a sentinel `ReservationHandle` with `reservationId=""`; commitSuccess no-ops + WARN logged. |
| R08 | `test_commit_success_emits_real_usage` | `commitSuccess(handle, { inputTokens: 100, outputTokens: 42 })` posts `/v1/trace` with `estimated_amount_atomic="142"` + `outcome="SUCCESS"`. |
| R09 | `test_release_failure_swallows_release_rpc_errors` | Sidecar release-RPC raises → `releaseFailure` swallows + WARN; does NOT re-throw (TTL sweep backstop). |
| R10 | `test_release_failure_classifies_cancelled` | `releaseFailure(handle, new AbortError())` emits `outcome="CANCELLED"`. |
| R11 | `test_idempotency_key_derivation_stable` | Two `reserve` calls with identical `(tenant, conversation_id, run_id, step_id, llm_call_id)` produce identical idempotency keys (regression for double-charge). |

### 2.2 `beforeAiGeneration.test.ts` (7)

| # | Name | What it asserts |
|---|------|-----------------|
| B01 | `test_allow_returns_data_with_handle_stash` | Sidecar ALLOW → hook return value is `{ data }` and `data._spendguardHandle.reservationId` is non-empty. |
| B02 | `test_deny_throws_runtime_error_with_budget_denied_code` | Sidecar DENY → hook throws Botpress `RuntimeError` whose `code` is `BUDGET_DENIED`. |
| B03 | `test_deny_no_upstream` | After DENY throw, no entry exists in the mock sidecar's `/v1/trace` history (proxy for upstream HTTP). **Critical invariant.** |
| B04 | `test_degrade_throws_runtime_error_budget_degraded` | Sidecar DEGRADE → `RuntimeError` `code: BUDGET_DEGRADED`. |
| B05 | `test_reentrant_safety` | Two concurrent `beforeAiGeneration` calls for the same conversation produce distinct handles. |
| B06 | `test_config_error_throws_budget_config` | Missing config → `RuntimeError` `code: BUDGET_CONFIG`. |
| B07 | `test_strict_ordering_reserve_before_data_return` | `_mockSidecar` records the `/v1/decision` POST timestamp; hook returns AFTER that timestamp. |

### 2.3 `afterAiGeneration.test.ts` (8)

| # | Name | What it asserts |
|---|------|-----------------|
| A01 | `test_commit_uses_real_usage` | When `data.payload.usage = { inputTokens: 100, outputTokens: 42 }`, commit posts `estimated_amount_atomic="142"`. NOT estimator. INV-5 primary. |
| A02 | `test_no_usage_estimator_fallback_logs_warn` | Missing `data.payload.usage` → estimator-snapshot commit + WARN log substring `falling back to estimator`. INV-5 secondary. |
| A03 | `test_after_without_before_is_noop` | When `data._spendguardHandle` missing (before-hook didn't run), after-hook returns `{ data }` without RPC. |
| A04 | `test_commit_failure_releases_then_throws` | `_mockSidecar` configured so `/v1/trace` returns 500 → `releaseFailure` called, then `RuntimeError` thrown. |
| A05 | `test_cancel_releases` | `data._cancelled = true` flag → `releaseFailure` with `outcome="CANCELLED"`. |
| A06 | `test_anthropic_usage_shape_normalised` | Botpress's normalised `{ inputTokens, outputTokens }` covers Anthropic's `input_tokens` + `output_tokens` (no provider-shape leak). |
| A07 | `test_bedrock_usage_shape_normalised` | Same normalisation for Bedrock. |
| A08 | `test_handle_cleared_from_data_after_commit` | Successful commit removes `data._spendguardHandle` (no leak across hooks). |

### 2.4 `adapter.test.ts` (6)

| # | Name | What it asserts |
|---|------|-----------------|
| AD01 | `test_binding_carries_bot_id_as_tenant_default` | When `configuration.tenantId` empty, `binding.tenant_id == ctx.botId`. |
| AD02 | `test_binding_carries_explicit_tenant_id` | When `configuration.tenantId` set, that wins. |
| AD03 | `test_prompt_hash_computed_via_d05_helper` | `binding.metadata.prompt_hash` equals `computePromptHash(messages)` from D05. |
| AD04 | `test_error_translation_denied_to_budget_denied` | `DecisionDenied` → `RuntimeError("BUDGET_DENIED")`. |
| AD05 | `test_error_translation_unavailable_to_budget_degraded` | `SidecarUnavailable` → `RuntimeError("BUDGET_DEGRADED")`. |
| AD06 | `test_error_translation_config_to_budget_config` | `SpendGuardConfigError` → `RuntimeError("BUDGET_CONFIG")`. |

### 2.5 `lifecycle.test.ts` (5)

| # | Name | What it asserts |
|---|------|-----------------|
| L01 | `test_validateConfiguration_issues_reserve_release_roundtrip` | Calls `SpendGuardReservation.reserve` + `releaseFailure` with `amountAtomic=1`; mock sidecar logs both. |
| L02 | `test_validate_bad_sidecar_propagates` | Mock sidecar configured to refuse connection → `validateConfiguration` re-throws with substring `sidecar unreachable`. |
| L03 | `test_validate_zod_rejects_empty_budget_id` | Zod parse fails → Botpress register sees a structured error. |
| L04 | `test_validate_rejects_unsupported_upstream` | `upstreamProvider="cohere"` → Zod enum rejection. |
| L05 | `test_validate_sidecar_deny_propagates_as_budget_denied` | Mock sidecar DENY on the probe → `RuntimeError("BUDGET_DENIED")` (operator sees it on register save). |

## 3. Integration tests — `integration-v12.test.ts` (4)

Boots a self-hosted Botpress v12 container (`botpress/server:v12.30.x` pinned by digest) via testcontainers-node, mounts the built `@spendguard/botpress-integration` `dist/`, seeds a sample bot, then triggers conversation events.

| # | Name | What it asserts |
|---|------|-----------------|
| I01 | `test_hook_fires_reserve_before_model_call` | POST a conversation message that triggers bot AI generation → mock sidecar records `/v1/decision` BEFORE Botpress's outbound to the mock upstream. Strict-ordering check via timestamped event log. INV-2. |
| I02 | `test_deny_short_circuits_the_generation` | Configure mock sidecar DENY → conversation reply contains the error code (`BUDGET_DENIED`); mock upstream records ZERO hits. INV-1. |
| I03 | `test_success_commits_real_usage` | ALLOW + successful upstream → mock sidecar `/v1/trace` records `estimated_amount_atomic` matching the upstream usage frame's `inputTokens + outputTokens`. INV-5. |
| I04 | `test_validateConfiguration_emits_sidecar_probe_at_install` | POSTing integration config to Botpress admin API triggers `register` → mock sidecar logs the 1-token probe. INV-4. |

I01 + I02 + I03 use strict-ordering event logs in `_mockSidecar` (`/v1/decision` and the mock upstream HTTP handler both push timestamped events to a shared in-memory queue). The integration test then asserts on the queue order, not on side effects alone.

The Botpress image is heavy (~800 MB). CI uses GH Actions cache for the docker layer. The integration test runs in a dedicated job (`botpress-integration-ci.yml`) gated by a path filter (`integrations/botpress/**`), so unrelated PRs do not pay the cost.

## 4. Demo regression — `verify_step_botpress.sql`

```sql
-- D32_BOTPRESS: at least 2 decisions carrying botpress integration tag.
DO $$ DECLARE c INT; BEGIN
  SELECT COUNT(*) INTO c FROM audit_outbox
   WHERE decision_context->>'integration' = 'botpress'
     AND decision_context->>'mode' = 'integration_sdk'
     AND created_at > now() - interval '5 minute';
  IF c < 2 THEN
    RAISE EXCEPTION 'D32_BOTPRESS_GATE: expected >=2 decisions, got %', c;
  END IF;
  RAISE NOTICE 'D32_BOTPRESS OK: decisions=%', c;
END; $$;

-- D32_BOTPRESS: at least one DENY decision.
DO $$ DECLARE c INT; BEGIN
  SELECT COUNT(*) INTO c FROM audit_outbox
   WHERE decision_context->>'integration' = 'botpress'
     AND cloudevent_payload::jsonb->'data'->>'decision' = 'DENY';
  IF c < 1 THEN
    RAISE EXCEPTION 'D32_BOTPRESS_GATE: expected >=1 DENY, got %', c;
  END IF;
END; $$;

-- D32_BOTPRESS: commit row present pairing with reservation row.
DO $$ DECLARE c INT; BEGIN
  SELECT COUNT(*) INTO c FROM commits
   WHERE latest_state IN ('estimated', 'provider_reported')
     AND created_at > now() - interval '5 minute';
  IF c < 1 THEN
    RAISE EXCEPTION 'D32_BOTPRESS_GATE: no commit rows';
  END IF;
END; $$;

-- D32_BOTPRESS: canonical_events received the botpress events (outbox forwarder ran).
DO $$ DECLARE c INT; BEGIN
  SELECT COUNT(*) INTO c FROM canonical_events
   WHERE source_integration = 'botpress';
  IF c < 1 THEN
    RAISE EXCEPTION 'D32_BOTPRESS_GATE: canonical_events empty for botpress';
  END IF;
END; $$;

-- D32_BOTPRESS: provider stub counter matches expected ALLOW count.
DO $$ DECLARE bad INT; BEGIN
  SELECT COUNT(*) INTO bad FROM audit_outbox
   WHERE decision_context->>'integration' = 'botpress'
     AND cloudevent_payload::jsonb->'data'->>'decision' = 'DENY'
     AND (decision_context->>'stub_hits')::int > 0;
  IF bad > 0 THEN
    RAISE EXCEPTION 'D32_BOTPRESS_GATE: % DENY decisions saw upstream hits', bad;
  END IF;
END; $$;

-- D32_BOTPRESS: streaming step produced an end-of-hook commit row.
DO $$ DECLARE c INT; BEGIN
  SELECT COUNT(*) INTO c FROM audit_outbox
   WHERE decision_context->>'integration' = 'botpress'
     AND decision_context->>'stream' = 'true';
  IF c < 1 THEN
    RAISE EXCEPTION 'D32_BOTPRESS_GATE: no streaming decision audited';
  END IF;
END; $$;
```

## 5. Demo driver — `run_botpress_real_mode`

3 steps, each with 4 assertions. Layout mirrors `run_dify_plugin_real_mode` for consistency.

| Step | Body | Asserts |
|------|------|---------|
| 1 ALLOW | Small conversation message; POST `/api/v2/admin/workspaces/.../conversations/.../messages` | Botpress HTTP 200; upstream stub counter +1; sidecar `/v1/decision` recorded **before** stub hit; commit row visible with real `inputTokens + outputTokens` |
| 2 DENY | Conversation message with demo-only `force_hard_cap=1` env on the Botpress runtime | Botpress reply contains `BUDGET_DENIED` code; upstream stub counter unchanged; DENY decision audited; **no** commit row added |
| 3 STREAM | Conversation message; bot configured for streaming via `model.stream = true` | Botpress HTTP 200; upstream stub counter +1; end-of-hook commit row with `decision_context.stream = "true"` |

The driver writes `[demo] botpress_real ALL 3 steps PASS (ALLOW + DENY + STREAM)` on success; gate failure exits 7.

## 6. Negative test surface (must-not-regress)

| What | Why | Where |
|------|-----|-------|
| Upstream hit on DENY | Worst correctness bug | B03 + I02 + demo driver step 2 + verify SQL stub_hits assertion |
| Reserve fires AFTER upstream | Cancels D32 thesis | B07 + I01 strict-ordering event |
| Commit uses estimator instead of real usage on success | Charges wrong amount | A01 + I03 |
| afterAi without beforeAi causes a phantom commit/release | Reservation chain corruption | A03 |
| `validateConfiguration` only probes upstream and not sidecar | Operators won't catch SpendGuard wiring errors at install | L01 |
| Hook leaks `sidecarUrl` mTLS key path to logs | Secret leak | Manual + lint rule on the integration tree (no `log.info(configuration)` substring) |
| Sidecar `_spendguardHandle` survives across distinct conversations | Cross-tenant contamination | A08 + B05 |

## 7. Performance budgets (informational)

| Op | Target | Source |
|----|--------|--------|
| `beforeAiGeneration` hook overhead (excluding sidecar HTTP round-trip) | < 2 ms p99 | pure delegation cost |
| Cold-start integration boot (Botpress register) | < 1500 ms | tsup bundle import + lazy SpendGuardClient init |
| First-token added latency vs raw upstream | < 50 ms p95 | reserve round-trip over mTLS loopback |

Verified manually post-merge; not a CI gate (no perf CI infra in tree today).

## 8. CI integration

`integrations/botpress/tests/*.test.ts` (unit) runs under a new pnpm vitest job in the existing GitHub Actions matrix.

`integrations/botpress/tests/integration-v12.test.ts` runs in a dedicated path-filter-gated job (`botpress-integration-ci.yml`); the gate is `paths: integrations/botpress/**`. Cell uses docker layer cache for the Botpress image.

`make demo-up DEMO_MODE=botpress_real` runs in the existing `e2e-demo` matrix as a new cell. Docker layer cache covers the Botpress image.

The `botpress-integration-publish.yml` workflow runs only on tags `botpress-integration-v*`; not in PR CI.
