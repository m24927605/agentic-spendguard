# D11 — Tests

**Reads:** [`design.md`](design.md), [`implementation.md`](implementation.md), [`acceptance.md`](acceptance.md).

## 1. Test inventory

| Tier | Suite | Path | Count |
|------|-------|------|-------|
| Unit | `test_litellm_guardrail.py` | `sdk/python/tests/integrations/test_litellm_guardrail.py` | 14 |
| Integration | `test_litellm_guardrail_proxy_inproc.py` | `sdk/python/tests/integrations/test_litellm_guardrail_proxy_inproc.py` | 4 |
| Demo regression | `verify_step_litellm_guardrail.sql` | `deploy/demo/verify_step_litellm_guardrail.sql` | 5 SQL assertions |
| Demo driver | `run_demo.py::run_litellm_guardrail_mode` | `deploy/demo/demo/run_demo.py` | 3 steps × 4 assertions |

Total new test surface: ~700 LOC tests + ~80 LOC SQL gates + ~120 LOC demo driver.

## 2. Unit tests — `test_litellm_guardrail.py`

Mocks the SpendGuard sidecar via a thin in-process gRPC stub that mirrors `sdk/python/tests/integrations/_fake_sidecar.py` (already used by `test_litellm.py`). LiteLLM is **not** imported by the unit suite — `SpendGuardGuardrail` is exercised against a fake `CustomGuardrail` base class to keep tests deterministic.

### 2.1 Import + wiring

| # | Name | What it asserts |
|---|------|-----------------|
| U01 | `test_import_error_message_when_litellm_missing` | When `litellm.integrations.custom_guardrail` is patched to raise `ImportError`, the module-level import raises `ImportError` with `pip install 'spendguard-sdk[litellm-guardrail]'` substring. |
| U02 | `test_guardrail_name_default` | `SpendGuardGuardrail(...)` exposes `guardrail_name == "spendguard"`. |
| U03 | `test_guardrail_name_override` | `SpendGuardGuardrail(guardrail_name="myteam-spendguard")` propagates. |

### 2.2 Env-driven default factory (`_build_delegate`)

| # | Name | What it asserts |
|---|------|-----------------|
| U04 | `test_env_missing_uds_raises` | Constructing with no kwargs and `SPENDGUARD_SIDECAR_UDS` unset → `SpendGuardConfigError`, message names the var. |
| U05 | `test_env_missing_tenant_raises` | Same with `SPENDGUARD_TENANT_ID`. |
| U06 | `test_env_default_resolver_constructs_binding` | With all 8 env vars set, the loaded resolver returns a `BudgetBinding` whose fields equal the env values. |
| U07 | `test_env_default_resolver_loads_unit_ref_and_pricing` | `BudgetBinding.unit.unit_id` and `BudgetBinding.pricing.pricing_version` match env. |
| U08 | `test_resolver_module_env_imports_factory` | `SPENDGUARD_RESOLVER_MODULE=tests.fixtures.fake_resolver:make_triple` imports + dispatches to operator factory, ignores single-tenant env vars. |
| U09 | `test_resolver_module_bad_path_raises` | `SPENDGUARD_RESOLVER_MODULE=nonexistent.module:bad` → `SpendGuardConfigError` at boot. |
| U10 | `test_resolver_module_missing_attr_raises` | Module imports but `bad_fn` missing → `SpendGuardConfigError`. |
| U11 | `test_explicit_kwargs_override_env` | Passing `budget_resolver=` to `__init__` skips env loader entirely (verified by leaving env unset). |

### 2.3 Hook delegation

| # | Name | What it asserts |
|---|------|-----------------|
| U12 | `test_pre_call_hook_delegates_to_callback` | `async_pre_call_hook` calls `_delegate.async_pre_call_hook` with the same arg shape and returns its return value verbatim. |
| U13 | `test_pre_call_deny_propagates` | When delegate raises `DecisionDenied`, the guardrail re-raises (does NOT swallow). |
| U14 | `test_pre_call_degrade_propagates` | When delegate raises `SidecarUnavailable`, the guardrail re-raises. |

### 2.4 Post-call hooks — signature translation

| # | Name | What it asserts |
|---|------|-----------------|
| U15 | `test_post_call_success_translates_signature` | `async_post_call_success_hook(data, user_api_key_dict, response)` is translated into a kwargs dict containing `litellm_call_id`, `user_api_key_dict`, and forwarded to delegate's `async_log_success_event`. |
| U16 | `test_post_call_failure_translates_signature` | `async_post_call_failure_hook(request_data, original_exception, user_api_key_dict)` populates `kwargs["exception"] = original_exception` and forwards. |
| U17 | `test_post_call_success_no_usage_logs_warn_and_uses_estimator` | When `response.usage is None`, delegate's streaming-fallback path fires + WARN log + commit uses the snapshot estimator (regression for `_async_log_success_streaming` path being reached). |

### 2.5 Fail-open dev escape

| # | Name | What it asserts |
|---|------|-----------------|
| U18 | `test_fail_open_skips_commit_on_degrade` | With `SPENDGUARD_LITELLM_FAIL_OPEN=1`, sidecar DEGRADE on pre-call returns `data` unchanged + WARN log + no commit row reaches the fake sidecar. |

## 3. Integration tests — `test_litellm_guardrail_proxy_inproc.py`

Boots LiteLLM proxy **in-process** via `litellm.proxy.proxy_cli` (no Docker). Real `SpendGuardGuardrail` registered via real `proxy_config.yaml`. Uses the same `_fake_sidecar.py` fixture as the unit suite. The "upstream provider" is a `pytest-aiohttp` test server.

| # | Name | What it asserts |
|---|------|-----------------|
| I01 | `test_proxy_registers_guardrail_from_yaml` | Proxy boots cleanly with the new yaml; introspection endpoint lists `spendguard` in `default_on` guardrails. |
| I02 | `test_proxy_allow_path_e2e` | `POST /v1/chat/completions` → fake-sidecar sees `RequestDecision` BEFORE the upstream aiohttp server logs a hit. Commit row is observed after the response. |
| I03 | `test_proxy_deny_path_short_circuits` | Configure fake-sidecar to DENY. `POST` returns 400/403, upstream aiohttp server logs **zero** hits (critical invariant). |
| I04 | `test_proxy_degrade_path_503` | Configure fake-sidecar to DEGRADE. `POST` returns 503, upstream zero hits. |

I02's "before" assertion uses a single `asyncio.Event` set by the fake sidecar on `RequestDecision`. The upstream aiohttp handler awaits the event briefly and records a strict-ordering check: if the event was not set before the handler fires, the assertion records `out-of-order` and the test fails. This catches the exact regression the spec's section 2.2 invariant pins.

## 4. Demo regression — `verify_step_litellm_guardrail.sql`

SQL gates executed by `make demo-up DEMO_MODE=litellm_guardrail`. Layout mirrors `verify_step_litellm_real.sql`.

```sql
-- D11_GUARDRAIL: at least one reserve row carrying the litellm guardrail mode.
DO $$ DECLARE c INT; BEGIN
  SELECT COUNT(*) INTO c FROM audit_outbox
   WHERE decision_context->>'integration' = 'litellm'
     AND decision_context->>'mode' = 'proxy'
     AND created_at > now() - interval '5 minute';
  IF c < 2 THEN  -- ≥2: 1 ALLOW + 1 DENY decisions audited
    RAISE EXCEPTION 'D11_GUARDRAIL_GATE: expected ≥2 litellm decisions, got %', c;
  END IF;
  RAISE NOTICE 'D11_GUARDRAIL OK: litellm decisions=%', c;
END; $$;

-- D11_GUARDRAIL: at least one DENY decision.
DO $$ DECLARE c INT; BEGIN
  SELECT COUNT(*) INTO c FROM audit_outbox
   WHERE decision_context->>'integration' = 'litellm'
     AND cloudevent_payload::jsonb->'data'->>'decision' = 'DENY';
  IF c < 1 THEN
    RAISE EXCEPTION 'D11_GUARDRAIL_GATE: expected ≥1 DENY, got %', c;
  END IF;
END; $$;

-- D11_GUARDRAIL: at least one commit row paired with a reserve row.
DO $$ DECLARE c INT; BEGIN
  SELECT COUNT(*) INTO c FROM commits
   WHERE latest_state IN ('estimated', 'provider_reported', 'invoice_reconciled')
     AND created_at > now() - interval '5 minute';
  IF c < 1 THEN
    RAISE EXCEPTION 'D11_GUARDRAIL_GATE: no commit rows';
  END IF;
END; $$;

-- D11_GUARDRAIL: canonical chain received the events (outbox forwarder ran).
DO $$ DECLARE c INT; BEGIN
  SELECT COUNT(*) INTO c FROM canonical_events
   WHERE source_integration = 'litellm';
  IF c < 1 THEN
    RAISE EXCEPTION 'D11_GUARDRAIL_GATE: canonical_events empty for litellm';
  END IF;
END; $$;
```

Additional SQL assertion: provider stub counter (recorded via a sidecar-tagged `decision_context.stub_hits` field in the demo driver) matches expected ALLOW count. Implements the "DENY never hits provider" invariant from acceptance §2.

## 5. Demo driver — `run_litellm_guardrail_mode`

3 steps, each producing 4 assertions. Layout follows existing `run_litellm_real_mode` for consistency.

| Step | Body | Asserts |
|------|------|---------|
| 1 ALLOW | small messages, fits budget | HTTP 200, stub counter +1, `_fake_sidecar` RequestDecision recorded **before** stub hit, commit row visible |
| 2 DENY | exhaust budget OR demo-only override that triggers hard-cap | HTTP 400/403, stub counter unchanged, DENY decision audited, **no** commit row added |
| 3 STREAM | `stream=True` | HTTP 200, stub counter +1, end-of-stream commit row, `decision_context.stream = true` |

The driver writes a single-line summary `[demo] litellm_guardrail ALL 3 steps PASS (ALLOW + DENY + STREAM)` on success; gate-failure is exit code 7 (matches `app.py` convention).

## 6. Negative test surface

| What | Why | Where |
|------|-----|-------|
| Provider hit on DENY | Most severe correctness bug; must never regress | I03 + demo driver step 2 |
| Reserve fires AFTER provider (wrong hook order) | Cancels the entire D11 thesis | I02 strict-ordering event |
| LiteLLM `during_call` mode chosen by mistake | Would silently lose pre-call gating | proxy_config schema test U01b — yaml schema linter ensures `mode: pre_call` literal |
| Two callbacks registered (legacy + guardrail) → double reserve | Would charge double | I05 — register both and assert exactly 1 `RequestDecision` per `litellm_call_id` |

I05 is the "co-registration safety net" test — pins that operators migrating from callback path to guardrail path can run both during cutover without budget double-charge. Implementation: dedup on `idempotency_key` already happens sidecar-side; the test verifies it covers this exact case.

## 7. Performance budgets (informational, not gates)

| Op | Target | Source |
|----|--------|--------|
| `async_pre_call_hook` overhead (guardrail layer only, excluding sidecar gRPC) | < 1ms p99 | wraps existing delegate; pure delegation cost |
| Cold-start guardrail registration | < 50ms | env parsing + module import |

Verified manually post-merge; not a CI gate (no perf CI infra in tree today).

## 8. CI integration

`sdk/python/tests/integrations/test_litellm_guardrail.py` + `test_litellm_guardrail_proxy_inproc.py` run under existing `pytest sdk/python` GitHub Actions matrix. Add `litellm[proxy]>=1.55` to the `[litellm-guardrail]` extra used by the test matrix.

`make demo-up DEMO_MODE=litellm_guardrail` runs in the existing `e2e-demo` matrix as a new matrix cell.

