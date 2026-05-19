# LiteLLM ⇄ Agentic SpendGuard Integration — TEST_PLAN.md

> Status: Proposed (doc-first; no test code lands until IMPLEMENTATION.md is accepted)
> Owner: Platform / Integrations / Testing
> Related: [`DESIGN.md`](./DESIGN.md), [`IMPLEMENTATION.md`](./IMPLEMENTATION.md),
> `feedback_demo_quality_gate.md`, `project_known_demo_flakes.md`,
> `sdk/python/src/spendguard/integrations/agt.py` (the integration we mirror).

---

## 1. Test Tiers & Philosophy

This integration is graded on three concentric tiers. Each tier exposes a class
of bugs the previous tier cannot. **No tier substitutes for another.**

### 1.1 Tier 1 — Unit (pure-Python, no sidecar, no LiteLLM proxy)

- `litellm` imports MUST be guarded so the test module loads when the optional
  dep is absent (mirrors `agt.py`'s `try/except ImportError`). Tests skip with
  a clear message in that case.
- Targets pure-Python logic: `derive_uuid_from_signature` mapping,
  `_signature(payload)` stability, `ClaimEstimator` / `ClaimReconciler` outputs
  on canned LiteLLM `kwargs` / `response_obj`, dataclass frozen/slots
  invariants, env-var parsing.
- Whole tier runs in < 10 s on CI.
- **Tier 1 cannot find**: wire-protocol mismatches, callback registration drift,
  retry-loop double-reserve, end-of-stream commit timing.

### 1.2 Tier 2 — Integration (real sidecar, real Postgres, real `litellm` SDK)

- Real `litellm` installed via `pip install 'spendguard-sdk[litellm,test]'`,
  same version pin as `pyproject.toml`'s `litellm` extra.
- Real SpendGuard sidecar over UDS, real Postgres ledger, real
  `canonical_events` chain (`sidecar_session` fixture — see §4).
- **Provider HTTP** is the only mocked layer: `litellm`'s built-in
  `mock_response="..."` kwarg or `respx`/`aioresponses` interception, or real
  localhost ollama on the developer machine.
- Tests drive `litellm.acompletion(..., metadata={...})` and assert wire
  results: `RequestDecision` reached the sidecar (verified via canonical
  events row, not via mock), reservation in `reservations`, commit in
  `invoices` with reconciled tokens, `RESERVATION_RELEASED` on failure.
- **Tier 2 cannot find**: real LiteLLM proxy multi-worker races, real
  `SpendLogs` ↔ SpendGuard join correctness, fixture-vs-real-provider drift.

### 1.3 Tier 3 — Demo (end-to-end; the quality gate)

Per `feedback_demo_quality_gate.md`: **Codex review ✅ is necessary, NOT
sufficient.** Every wire-touching slice must demonstrably run end-to-end
through `make demo-up DEMO_MODE=…` before acceptance. The 14 wire-time bugs
caught only by demo bring-up in Phase 2B is the precedent.

- Real Postgres, sidecar, ledger.
- Real `litellm` proxy (`litellm --config proxy_config.yaml`) for proxy mode;
  Python entrypoint calling `litellm.acompletion()` for the callback-only
  variant.
- Real `spendguard_litellm_proxy_callback.py` template loaded by the proxy.
- Provider: **counting HTTP endpoint** (in-process `aiohttp` mock server)
  for both demos. `litellm.acompletion(mock_response="...")` is BANNED
  in Tier 3 because the deny-path provider-counter assertion goes
  vacuous (Round 2 Phase 0 review P0.4 fix — supersedes any earlier
  "mock_response fallback in CI" wording). Real ollama on localhost is
  also acceptable for `litellm_real` (counts via access log).
- Invariants from DESIGN.md §11:
  1. `litellm.acompletion()` succeeds end-to-end.
  2. One reserve + one commit canonical event per call.
  3. `LiteLLM_SpendLogs` row also written (LiteLLM-owned, not ours).
  4. `derive_uuid_from_signature("litellm:" + litellm_call_id,
     scope="llm_call_id")` joins `canonical_events.llm_call_id` to
     `LiteLLM_SpendLogs.litellm_call_id`.
  5. Deny mode: upstream provider HTTP layer is **never** contacted
     (verified by a request-counter side-channel — see §3.2).

### 1.4 The mocking line — what is banned

**For Tier 2 and Tier 3, mocking `SpendGuardClient` itself is BANNED.** Lesson
of `feedback_demo_quality_gate.md`: 14 wire-time bugs were caught only by
running real sidecar + real client across the real UDS / gRPC boundary. A
test that monkey-patches `SpendGuardClient.request_decision` to return a fake
outcome proves nothing about what we actually ship.

Allowed at Tier 2: mocking provider HTTP (OpenAI / Anthropic / ollama),
mocking monotonic clock for TTL-derived logic.

Banned everywhere except Tier 1: mocking `SpendGuardClient` methods,
`canonical_events` / ledger tables, or `litellm.acompletion()` itself
(bypassing LiteLLM is meaningless — we are testing the callback LiteLLM
invokes).

---

## 2. Per-slice test mapping

Slices follow DESIGN.md §12 / IMPLEMENTATION.md (10-slice plan after Phase
0 review). Each subsection heading is the anchor IMPLEMENTATION.md links
to (`#tests-for-slice-N`). The ≤250-line slice cap also applies to test
code: if a slice's tests exceed 250 lines, the test file is split by
concern (one file per CustomLogger hook, for example), or the overflow
counts against the next slice's line budget.

**Slice → tests mapping (10-slice plan):**

| Slice | Title | Tier 1 file | Tier 2 file | Tier 3 demo |
|-------|-------|-------------|-------------|-------------|
| 1 | SDK skeleton | `test_litellm_skeleton.py` | — | — |
| 2 | Pre-call hook | `test_litellm_precall_unit.py` | `test_litellm_precall_integration.py` | — |
| 3 | Success commit (non-streaming) | `test_litellm_commit_unit.py` | `test_litellm_commit_integration.py` | — |
| 4 | Streaming reconciler | `test_litellm_streaming_unit.py` | `test_litellm_streaming_integration.py` | — |
| 5 | Failure release | `test_litellm_failure_unit.py` | `test_litellm_failure_integration.py` | — |
| 6 | Demo `litellm_real` step 1+2 | — | — | `DEMO_MODE=litellm_real` (partial) |
| 7 | Demo `litellm_deny` (3 sub-steps) | — | — | `DEMO_MODE=litellm_deny` |
| 8 | Proxy template + recipe | `test_litellm_proxy_template.py` | `test_litellm_proxy_subprocess.py` | — |
| 9 | Demo `litellm_real` step 3+4 | — | — | `DEMO_MODE=litellm_real` (complete) |
| 10 | Docs site + final pass | — | docs link check | — (Codex pass is its own gate) |

### 2.1 tests-for-slice-1 — SDK skeleton

Scope: shell, dataclasses, `try/except ImportError`, `__all__`, env-var parsing.

**Unit** (`tests/test_litellm_skeleton.py`, < 120 lines):

1. `test_module_imports_without_litellm_installed` — patch `sys.modules` to
   simulate `litellm` absence; assert `ImportError` with expected install hint.
2. `test_module_imports_with_litellm_installed` — happy path; `__all__`
   contains every DESIGN.md §6 public symbol.
3. `test_litellm_run_context_is_frozen_slots` — assignment raises
   `FrozenInstanceError`.
4. `test_budget_binding_dataclass_shape` — fields + types match DESIGN.md §6.
5. `test_env_var_fail_open_default_is_false` — unset → `fail_closed=True`.
6. `test_env_var_fail_open_one_flips_to_true` — `=1` only, not `=true`/`=yes`
   (match SDK convention; verify against `client.py` before pinning).
7. `test_env_var_ttl_seconds_default_300` and rejects negative.
8. `test_run_context_async_cm_set_get_reset` — contextvar set / get / reset.
9. `test_resolver_context_dataclass_shape` — `ResolverContext` is frozen +
   slotted; has `data`, `user_api_key_dict`, `call_type` fields.
10. `test_no_default_budget_env_var` — `SPENDGUARD_LITELLM_DEFAULT_BUDGET_ID`
    is NOT read anywhere (greps the module source); confirms the env-var
    deletion per P0.10 fix from Phase 0 review.

**Integration**: NONE. The shell is non-functional; running it against a real
sidecar would assert nothing.

**Demo invariants**: NONE for this slice (the only slice with zero demo
coverage; later slices compensate).

### 2.2 tests-for-slice-2 — Pre-call hook + reservation path

Scope: `async_pre_call_hook` calls `request_decision`, on ALLOW
stashes `decision_id` + `reservation_ids` tuple + decision context
into `self._stash[litellm_call_id]` (NEVER into `data` — Round 2
P1.3 alignment with IMPLEMENTATION P1.5), on DENY raises
`DecisionDenied`.

**Unit** (`tests/test_litellm_precall_unit.py`, < 200 lines):

1. `test_pre_call_hook_builds_resolver_context_with_user_api_key_dict` —
   resolver receives a `ResolverContext` with `user_api_key_dict` populated
   from the hook's explicit arg (P0.2 fix from Phase 0 review).
2. `test_pre_call_hook_uses_claim_estimator_output_single_claim` —
   estimator returns exactly **one** claim (v1 contract per DESIGN.md §6);
   it appears in `request_decision(projected_claims=[...])`.
3. `test_pre_call_hook_stashes_reservation_ids_tuple` — on ALLOW,
   `callback._stash[litellm_call_id]["reservation_ids"]` is a tuple
   (P0.7 fix; **plural** key name).
4. `test_pre_call_hook_does_not_mutate_data_with_spendguard_key` — after
   hook returns, `"spendguard" not in returned_data` (P1.5 fix —
   stash lives on `self._stash`, NOT on `data`).
5. `test_pre_call_hook_raises_on_resolver_returns_none` —
   resolver returns `None` → `SpendGuardConfigError`. No env-var
   fallback (P0.10 fix; default budget env removed).
6. `test_idempotency_key_derivation` — same `litellm_call_id` → same
   derived `llm_call_id` and `decision_id`; **different**
   `litellm_call_id` (per LiteLLM retry) → different `decision_id`
   (P0.9 confirmation).
7. `test_decision_context_json_fields` — pre-call sends the 12 fields
   from DESIGN.md §8.2a (`integration`, `litellm_call_id`, `model`,
   `pricing_version`, `price_snapshot_hash_hex`, `fx_rate_version`,
   `unit_conversion_version`, `prompt_hash`, `call_type`, `stream`,
   `mode`, `team_id`). Asserts `mode='direct'` when
   `user_api_key_dict is None`, `mode='proxy'` otherwise.

**Integration** (`tests/integration/test_litellm_precall_integration.py`,
< 200 lines, **sidecar required**):

1. `test_pre_call_hook_real_allow_lands_in_canonical_events` — real
   sidecar, real ledger. Invoke callback with a synthesized `data`.
   Query `canonical_events` for `DECISION_REQUESTED` +
   `DECISION_ALLOWED`; assert `llm_call_id` matches derivation,
   `decision_context_json->>'integration' = 'litellm'`, and at least
   one `reservation_id` populated in the `RESERVATION_CREATED` event.
2. `test_pre_call_hook_real_deny_raises_typed_exception` — pre-exhaust
   budget; assert `DecisionDenied` with exact
   `reason_codes=["BUDGET_EXCEEDED"]` and no `RESERVATION_CREATED` row.
3. `test_pre_call_hook_sidecar_unreachable_fails_closed` — bogus socket
   path; `SidecarUnavailable` raised; no canonical event.
4. `test_pre_call_hook_fail_open_env_allows` —
   `SPENDGUARD_LITELLM_FAIL_OPEN=1` and unreachable sidecar; call
   proceeds with WARNING log present (per ACCEPTANCE.md S6); no
   canonical event because sidecar was never reached.
5. `test_pre_call_hook_proxy_identity_resolved` — fake
   `UserAPIKeyAuth(team_id="t1")`; resolver returns binding for `t1`
   only (P0.2 confirmation in integration mode).

**Demo invariants this slice contributes to**:
- `DEMO_MODE=litellm_real`: reservation observable in `reservations` after
  slice 2. Commit (slice 3) not yet there, so reservation sits in `RESERVED`
  until commit slice lands — acceptable interim state.
- `DEMO_MODE=litellm_deny`: fully testable at slice 2 in isolation (no
  commit needed). The deny demo locks in here. See §3.2.

### 2.3 tests-for-slice-3 — Success-event commit path + reconciler (non-streaming)

Scope: `async_log_success_event` non-streaming branch calls
`emit_llm_call_post(outcome="SUCCESS", ...)` with reconciled claims from
`response_obj.usage` and `reservation_ids[0]` from the stash. **Streaming
branch is Slice 4.**

**Unit** (`tests/test_litellm_commit_unit.py`, < 160 lines):

1. `test_commit_reads_usage_from_response_obj` — canned
   `usage.prompt_tokens=37, completion_tokens=92`; reconciler produces
   `amount_atomic` = (37 × input_price + 92 × output_price) in
   PricingFreeze units.
2. `test_commit_uses_stashed_decision_id_and_first_reservation_id` —
   `emit_llm_call_post` called with `decision_id` from stash and
   `reservation_id = stash["reservation_ids"][0]` (P0.7 confirmation).
3. `test_commit_missing_stash_is_noop` — pre-call hook never fired (or
   stash already popped); silent no-op, no RPC.
4. `test_commit_rejects_multi_claim_reconciler` — reconciler returns 2
   claims → `SpendGuardConfigError` (v1 single-claim contract, P1.8 fix).
5. `test_commit_rejects_multi_reservation_stash` — stash has 2
   `reservation_ids` → `SpendGuardConfigError` (v1 single-reservation
   constraint, P0.7 partner test).
6. `test_commit_non_streaming_skips_streaming_branch` — `stash["stream"]
   is False`; `_async_log_success_streaming` not invoked.
7. `test_reconciler_handles_missing_usage` — provider returned no usage
   field; reconciler falls back to estimator's last value (documented).

**Integration** (`tests/integration/test_litellm_commit_integration.py`,
< 180 lines, **sidecar required**):

1. `test_full_reserve_commit_lands_invoice_row` — pre-call + success-event
   sequence against real sidecar (provider HTTP mocked via counting
   endpoint, not `mock_response`). One `invoices` row, balance
   decremented by exactly the reconciled amount.
2. `test_idempotent_commit` — fire `async_log_success_event` twice with
   the same `decision_id` (simulates LiteLLM retry of the log call only,
   not the LLM call). One invoice, not two; same `invoice_id`.
3. `test_commit_with_zero_completion_tokens` — empty response; commit
   succeeds with prompt-only cost.

**Demo invariants**: `DEMO_MODE=litellm_real` step 1 ALLOW: full allow →
commit cycle observable. Balance decrements visibly. Slices 2+3 together
complete the non-streaming allow-mode happy path.

### 2.4 tests-for-slice-4 — Streaming reconciler

Scope: `_async_log_success_streaming` branch fires on `stash["stream"]
is True`; reads `response_obj.usage` after the full stream is consumed;
commits with real totals (not estimator worst-case).

**Unit** (`tests/test_litellm_streaming_unit.py`, < 140 lines):

1. `test_streaming_branch_dispatch` — `stash["stream"]=True` routes to
   `_async_log_success_streaming`; non-streaming path is NOT taken.
2. `test_streaming_commit_uses_real_usage_not_estimator` — estimator
   said 1000 worst-case tokens; reconciler reads actual 200 tokens
   from `response_obj.usage`; commit amount reflects 200.
3. `test_streaming_response_missing_usage_falls_back_to_estimator` —
   provider didn't emit `.usage` at end-of-stream; reconciler uses
   estimator's value; warning logged.
4. `test_streaming_ttl_env_passed_to_reservation` —
   `SPENDGUARD_LITELLM_TTL_SECONDS=600` propagates to pre-call
   reservation request (read once at __init__).

**Integration**
(`tests/integration/test_litellm_streaming_integration.py`, < 180
lines, **sidecar required**):

1. `test_streaming_acompletion_reserves_then_commits` —
   `litellm.acompletion(..., stream=True)` against mock provider with
   counted chunk stream; assert `RESERVATION_CREATED` observed before
   first chunk delivered, `INVOICE_COMMITTED` after final chunk; commit
   amount ≠ pre-call estimator amount.
2. `test_streaming_caller_cancellation_releases` — caller cancels
   mid-stream via `asyncio.wait_for(...)`; `async_log_failure_event`
   fires with `CancelledError`; `RESERVATION_RELEASED` observed.
3. `test_streaming_long_stream_within_ttl` — fixture-controlled stream
   of 8s wall-clock; TTL=10s; commit lands within TTL window;
   reservation never auto-released.

**Demo invariants this slice contributes to**: `DEMO_MODE=litellm_real`
step 3 STREAM (Slice 9).

**Additional streaming edge-case tests** (Round 2 P1.2 + Round 3
P1.5 fixes — coverage was incomplete; numbering continuous per P2.3):

4. `test_streaming_provider_error_after_chunks` — provider closes
   the connection after delivering N chunks (no end-of-stream usage
   frame); `async_log_failure_event` fires; reservation released.
5. `test_streaming_partial_final_chunk` — final chunk arrives truncated
   (provider crashed mid-frame); LiteLLM raises; failure-event path
   exercises like (4).
6. `test_streaming_with_num_retries` — `stream=True` + `num_retries=2`,
   first attempt fails mid-stream; second attempt succeeds. Expected:
   2 distinct reservations (one per attempt per ADR-002), first
   released, second committed.
7. `test_streaming_network_reset_mid_sse` — simulated network reset
   on the fixture socket during chunk stream; LiteLLM error path
   surfaces; failure-event fires; reservation released.
8. `test_streaming_consumer_abandons_generator` — caller breaks
   out of `async for` early (e.g. found the answer; doesn't drain);
   no clean end-of-stream frame; reservation TTL-sweeps. Documents
   the expected leak-to-TTL path (Round 3 P1.6 fix — coverage gap).
9. `test_streaming_sidecar_offline_at_commit_boundary` — sidecar
   killed AFTER the stream completes but BEFORE
   `async_log_success_event` lands; surfaces
   `SidecarUnavailable` (Round 3 P0.8 — NF5 revised
   semantics).

### 2.5 tests-for-slice-5 — Failure-event release + retry handling

Scope: `async_log_failure_event` calls
`emit_llm_call_post(outcome="FAILURE"|"CANCELLED")` with stashed
`reservation_ids[0]`. LiteLLM `num_retries` fires multiple pre-call
hooks per logical user call; each retry mints a fresh `litellm_call_id`
so derived `decision_id` is distinct (DESIGN.md §5 retry row, P0.9
clarification).

**Unit** (`tests/test_litellm_failure_unit.py`, < 140 lines):

1. `test_failure_event_calls_release_with_decision_id_and_reservation` —
   release RPC invoked with stashed `decision_id` and
   `reservation_ids[0]`.
2. `test_failure_event_on_cancelled_error_class` — `kwargs["exception"]`
   is an `asyncio.CancelledError` instance; classified as `CANCELLED`.
3. `test_failure_event_on_cancelled_string` — `kwargs["exception"]` is
   a string containing "cancelled" (some LiteLLM versions); still
   classified as `CANCELLED` (defensive check).
4. `test_failure_event_missing_stash_is_noop` — no stash → silent no-op.
5. `test_retry_loop_each_attempt_has_distinct_decision_id` — three
   simulated retry kwargs with distinct `litellm_call_id`s; derived
   `decision_id`s all differ; aligns with DESIGN.md §5 retry contract
   (P0.9).
6. `test_failure_event_swallows_release_rpc_errors` —
   `emit_llm_call_post` raises; failure-event returns silently
   (TTL-sweep is durable backstop).

**Integration**
(`tests/integration/test_litellm_failure_integration.py`, < 180 lines,
**sidecar required**):

1. `test_provider_500_releases_reservation` — counting endpoint returns
   500; `RESERVATION_RELEASED` in canonical_events; balance restored.
2. `test_num_retries_three_releases_two_keeps_third` — `num_retries=3`,
   first two fail with 500, third succeeds. 3 reservations, 2 released,
   1 committed; final balance = exactly one charge (P0.9 in vivo).
3. `test_client_cancel_releases_reservation` — wrap call in
   `asyncio.wait_for(..., timeout=0.01)`; reservation released after
   `CancelledError`.

**Demo invariants**: not direct. Both `litellm_real` and `litellm_deny`
demos implicitly exercise this slice (any in-flight reservation must
have a matching commit or release path).

### 2.6 tests-for-slice-6 — Demo `litellm_real` ALLOW + DENY (steps 1+2)

**This slice IS the Tier 3 demo for steps 1+2 of `litellm_real`.** No
slice-specific unit tests; the demo IS the test. See §3.1. Acceptance
is the green `make demo-up DEMO_MODE=litellm_real` run (steps 1+2
lines visible; steps 3+4 deferred to Slice 9).

**What lands**:
- `run_litellm_real_mode()` in `deploy/demo/demo/run_demo.py` (mirrors
  `run_agt_composite_mode`) — steps 1+2 only.
- `deploy/demo/verify_step_litellm_real.sql` with Q1/Q3 from
  ACCEPTANCE.md §5.1 (Q2 added by Slice 9).
- `deploy/demo/Makefile` branch for the new mode.
- Counting HTTP endpoint helper (`_start_counting_provider(...)` in
  run_demo.py).

**Unit / integration**: NONE. (Coverage earned in slices 1–5.)

**Demo invariants this slice owns**:
- `DEMO_MODE=litellm_real` runs steps 1+2 end-to-end (see §3.1).
- `verify_step_litellm_real.sql` (Q1+Q3 subset) exits 0.
- Counting endpoint sees ≥1 hit for step 1 ALLOW; 0 incremental hits
  for step 2 DENY.

### 2.7 tests-for-slice-7 — Demo `litellm_deny` (3 fail-closed sub-steps)

**This slice IS the Tier 3 demo for the deny path.** See §3.2 — three
sub-steps: budget exhausted, sidecar offline, resolver returns None.

**What lands**:
- `run_litellm_deny_mode()` in `run_demo.py` covering all 3 sub-steps.
- `deploy/demo/verify_step_litellm_deny.sql` with the zero-provider-contact
  asserts for each sub-step.
- Makefile branch for `DEMO_MODE=litellm_deny`.
- Counting HTTP endpoint shared with Slice 6's helper.

**Demo invariants this slice owns**:
- `DEMO_MODE=litellm_deny` runs all 3 sub-steps end-to-end (see §3.2).
- Provider request count == 0 after ALL THREE sub-steps complete (the
  key wire-time assertion — P0.11 fix: counting HTTP endpoint
  mandatory, `mock_response` BANNED for deny mode).
- `verify_step_litellm_deny.sql` exits 0.

### 2.8 tests-for-slice-8 — Proxy callback template + recipe

Scope: `deploy/demo/litellm_proxy/spendguard_litellm_proxy_callback.py`
operator example + `PROXY_RECIPE.md` + `proxy_config.yaml`. **Uses
SDK-provided `_LoopBoundCallback` for lazy bootstrap on the
LiteLLM serving event loop (Round 3 P0.3/P0.4 fix; supersedes the
earlier `asyncio.run` recommendation).

**Unit** (`tests/test_litellm_proxy_template.py`, < 100 lines):

1. `test_template_module_loads` — `importlib.import_module(...)`
   succeeds with required env vars set.
2. `test_resolver_maps_team_id_to_budget_via_env_var` — set
   `SPENDGUARD_BUDGET_FOR_TEAM_t1=...`; resolver returns matching
   `BudgetBinding` (canonical env-var form per P2 fix —
   `SPENDGUARD_BUDGET_FOR_TEAM_{team_id}` only, NOT
   `SPENDGUARD_BUDGET_team-a`).
3. `test_resolver_unknown_team_raises` — `team_id` missing from
   `UserAPIKeyAuth` → explicit `RuntimeError`, not silent default.
4. `test_template_does_not_use_asyncio_run_at_import_time` — grep
   the source for top-level `asyncio.run(`; assert ZERO matches.
   The proxy template MUST use `_LoopBoundCallback`'s lazy init
   (Round 2 P0.5 fix — bootstrap on LiteLLM's serving loop, not a
   temporary loop).
5. `test_template_uses_loop_bound_callback` — assert
   `handler_instance` is an instance of `_LoopBoundCallback`.
6. `test_stash_sweep_drops_stale_entries` — synthesize 3 stash
   entries with `inserted_at` older than TTL; trigger sweep; assert
   stash becomes empty (Round 2 P1.5 fix — bounded memory).

**Integration**
(`tests/integration/test_litellm_proxy_subprocess.py`, < 200 lines,
**sidecar + real LiteLLM proxy subprocess required**):

1. `test_proxy_subprocess_boots_with_template` — `litellm --config
   proxy_config.yaml --port 4000` subprocess starts; handler_instance
   registered; one HTTP `POST /v1/chat/completions` to the subprocess
   produces canonical_events row.
2. `test_proxy_two_team_isolation` — two teams bound to two SpendGuard
   budgets via env. One call per team; each team's invoice lands
   against its own budget; no cross-charge.

**Demo invariants**: Slice 9 step 4 uses this template's
`spendguard_litellm_proxy_callback.py` directly.

### 2.9 tests-for-slice-9 — Demo `litellm_real` STREAM + PROXY (steps 3+4)

**This slice extends Slice 6's `run_litellm_real_mode()` with steps 3+4.**
The demo IS the test. See §3.1 (now showing all 4 steps).

**What lands**:
- `run_litellm_real_mode()` extended with steps 3 (STREAM) and 4
  (PROXY).
- `deploy/demo/verify_step_litellm_real.sql` extended with Q2
  LiteLLM_SpendLogs cross-join.
- `deploy/demo/litellm_proxy/` compose service (LiteLLM proxy
  subprocess for step 4).

**Demo invariants this slice owns**:
- `DEMO_MODE=litellm_real` runs all 4 steps end-to-end; final stdout
  line is exactly `[demo] PASS — all 4 steps OK`.
- Step 3 STREAM commit amount ≠ estimator amount (proves reconciler
  ran).
- Step 4 PROXY produces ≥1 `LiteLLM_SpendLogs` row + ≥1 matched
  Q2 cross-join row. (Q2 is only meaningful in proxy mode per
  DESIGN.md §8.3 — P0.5 fix.)
- Counting endpoint hit count matches expected ALLOW/PROXY successful
  calls (≥2 hits across the 4 steps; 0 hits during DENY/STREAM-deny
  if any).

### 2.10 tests-for-slice-10 — Docs site + final Codex pass

Scope: `docs/site/docs/integrations/litellm.md` 3-path page + sibling
Related-footer updates + README + quickstart + the final
whole-integration adversarial Codex pass per ACCEPTANCE.md C2.

**Unit/Integration**: NONE in the traditional sense — this slice ships
docs + a one-shot Codex run.

**Doc tests** (via existing docs site CI):

1. `test_litellm_md_renders` — docs site build succeeds with new page.
2. `test_three_paths_present` — page contains "Path A", "Path B",
   "Path C" headings with code snippets.
3. `test_quickest_validation_block_matches_acceptance` — the
   "Quickest validation" code block in the docs page matches
   ACCEPTANCE.md §5.1 verbatim (no copy-paste drift).
4. `test_sibling_related_footers_link_back` — `grep -l "litellm.md"
   docs/site/docs/integrations/{agt,langchain,openai-agents,pydantic-ai}.md`
   returns all four files.

**Final Codex pass** (one-shot, results in
`docs/specs/litellm-integration/review-logs/final-pass.md`):

- Adversarial mode against `base-before-slice-01..HEAD-of-slice-10`.
- Criterion: **zero new P0**. P1 acceptable only if logged with
  status (deferred-with-issue / disputed-with-reason) per
  ACCEPTANCE.md C2.

**Demo invariants**: not direct. Slice 10 verifies the documented
quickstart still produces the expected output (regression against
ACCEPTANCE.md §5.1).

---

## 3. Demo Modes — the quality gate

### 3.1 DEMO_MODE=litellm_real (4-step allow + audit chain)

**Stack**: Postgres + sidecar + canonical-ingest + LiteLLM proxy
subprocess (for step 4) + Python entrypoint calling `litellm.acompletion()`
(for steps 1+2+3) + counting HTTP endpoint (in-process `aiohttp`
mock server with hit counter — `mock_response` BANNED for this demo
per P0.11 fix).

**Invocation**:

```bash
cd deploy/demo
make demo-down                # clean state
DEMO_MODE=litellm_real make demo-up
```

**Expected stdout** (authoritative shape — REVIEW_STANDARDS.md §7.3
and ACCEPTANCE.md §5.1 MUST match this verbatim; all proxy-driven):

```
[demo] DEMO_MODE=litellm_real → litellm proxy + sidecar + ledger ready
[demo] handshake ok session_id=...
[demo] step 1: ALLOW — POST /v1/chat/completions team=t1 → DECISION_ALLOWED → INVOICE_COMMITTED
[demo] step 2: DENY — POST over-budget → DecisionDenied raised (provider counter delta=0)
[demo] step 3: STREAM — POST stream=true → sse complete → INVOICE_COMMITTED with real usage
[demo] step 4: PROXY-MULTI-TEAM — POST team=t2 → DECISION_ALLOWED → INVOICE_COMMITTED (isolated from t1)
[demo] PASS — all 4 steps OK
```

Absent the literal `PASS — all 4 steps OK` line → demo gate **fails**
regardless of exit code.

Slices that build this demo incrementally:
- Slice 6 ships steps 1+2 + counting endpoint helper + Q1/Q3 SQL.
- Slice 9 appends steps 3+4 + Q2 cross-join SQL (proxy mode only).

**Expected ledger rows** (example UUIDs):

| table | key column | example value |
|---|---|---|
| `canonical_events` | `llm_call_id` | `019059c0-0000-7000-8000-...` (derived from `litellm_call_id`) |
| `reservations` | `state` | `COMMITTED` |
| `invoices` | `decision_id` | matches the ALLOWED event |
| `ledger_transactions` | `operation_kind` | `committed_invoice` |

**Failure-mode triage**:

1. **Sidecar unreachable** → check UDS path env; verify
   `[sidecar] listening on …` in `make demo-up` logs.
2. **No canonical_events row** → callback never fired. Check
   `litellm.callbacks` registration order (callbacks set BEFORE first
   `acompletion`). Verify `litellm_settings.callbacks` parsed the
   entry-point string.
3. **`llm_call_id` mismatch** → derivation drift. Replicate
   `test_prompt_hash.py`'s shared-vector pattern: pin a
   `(litellm_call_id, expected_llm_call_id)` regression vector.
4. **Reservation stuck in `RESERVED`** → success event never fired.
   Streaming misconfigured or `response_obj` shape changed in a LiteLLM
   upgrade. Check LiteLLM version pin in `pyproject.toml`.

### 3.2 DEMO_MODE=litellm_deny (3-step fail-closed deep dive)

**Stack**: Postgres + sidecar (toggled offline for step 2) + counting
HTTP endpoint. `mock_response` BANNED (P0.11 fix). Each sub-step
asserts the counting endpoint stays at the pre-sub-step snapshot.

**Invocation**:

```bash
cd deploy/demo
make demo-down                # clean state
DEMO_MODE=litellm_deny make demo-up
```

**Expected stdout** (authoritative shape — ACCEPTANCE.md §5.2 MUST
match this verbatim):

```
[demo] DEMO_MODE=litellm_deny → fail-closed scenarios
[demo] handshake ok session_id=...
[demo] step 1: budget exhausted — DecisionDenied raised (provider untouched)
[demo] step 2: sidecar offline — SidecarUnavailable raised (provider untouched)
[demo] step 3: resolver returns None + no default budget — SpendGuardConfigError raised
[demo] PASS — all 3 deny paths OK
```

Absent the literal `PASS — all 3 deny paths OK` line → demo gate
**fails** regardless of exit code.

**Most important assertion**: post-call provider request counter == 0. If
the deny fires AFTER the upstream provider was contacted, the integration
has failed its core promise (money already spent).

**Expected ledger rows**:

| table | column | value |
|---|---|---|
| `canonical_events` | event_type | `DECISION_DENIED` |
| `ledger_transactions` | `operation_kind` | `denied_decision` |
| `reservations` | (n/a) | zero rows for this `decision_id` |
| `invoices` | (n/a) | zero rows for this `decision_id` |

**Failure-mode triage**:

1. **Provider counter > 0** → callback firing AFTER the HTTP call.
   We accidentally registered the budget check on `async_log_success_event`
   (post-wire log hook) instead of `async_pre_call_hook` (pre-wire
   gate). Verify the override: `async_pre_call_hook` MUST raise
   `DecisionDenied`/`SidecarUnavailable` to block; raising
   in `async_log_success_event` is too late (provider already paid).
   Note that `log_pre_api_call` is **sync** and also pre-wire, but it
   fires only for sync `litellm.completion()` calls — for the async
   path used by the demo, only `async_pre_call_hook` controls the
   gate (Round 3 P0.5 fix — previous wording incorrectly said
   `log_pre_api_call` fires after the wire).
2. **No `DECISION_DENIED` event** → `request_decision` RPC returned an error
   instead of a typed deny. Check sidecar logs for stack traces.
3. **`pytest.raises(Exception)`-style false positive** — demo must assert
   the exact `DecisionDenied` subclass with the exact reason_code. See §5.

### 3.3 What we DO NOT depend on

- The demo MUST NOT depend on `DEMO_MODE=ttl_sweep`. Per
  `project_known_demo_flakes.md`, ttl_sweep currently fails its verify
  assertion (downstream reserve flow bug). Slices 5/6 run green without
  requiring a ttl_sweep run.
- The demo MUST NOT depend on a live OpenAI / Anthropic API key in
  CI. The **counting HTTP endpoint** (in-process aiohttp mock with
  hit counter) is the CI default. Real ollama on localhost is
  acceptable for the allow demo when available. Round 4 P2.1 fix:
  `mock_response` is BANNED for Tier 3 demos; no fallback to it.
  Slices 1–5 SDK regression uses `DEMO_MODE=decision` which has its
  own provider stub independent of LiteLLM.

---

## 4. Fixtures & test infrastructure

### 4.1 Sidecar fixture

Session-scoped `sidecar_session` pytest fixture spins up the same compose
stack `make demo-up DEMO_MODE=decision` uses, parameterized via
`SPENDGUARD_TEST_COMPOSE_PROJECT` prefix so parallel runs don't collide. The
fixture yields a `SpendGuardClient` already handshook.

Pattern lifted from `run_demo.py` ~line 644:
`SpendGuardClient(socket_path=…, tenant_id=…); await c.connect();
await c.handshake()`. The fixture encapsulates that plus teardown that
flushes canonical_events for the test's tenant_id — isolation is per-tenant,
not per-database.

`sdk/python/tests/conftest.py` does not exist at HEAD (`tests/` contains only
`test_prompt_hash.py`). The LiteLLM slice introduces it. If it grows past
~80 lines, factor into `tests/_fixtures/sidecar.py` and import.

### 4.2 Postgres fixture

Same compose stack provides Postgres. Fixture exposes:
- `pg_session` — `psycopg`/`asyncpg` connection scoped to one test; rolls
  back on teardown OR runs `DELETE FROM canonical_events WHERE tenant_id=...`
  for tests needing committed state.
- `pg_verify(sql_path)` helper wrapping the psql command pattern from the
  existing `verify_step_*.sql` family.

### 4.3 LiteLLM mock provider — counting HTTP endpoint MANDATORY for deny

Three shapes — but **NOT interchangeable for the deny demo** (P0.11 fix
from Phase 0 review):

1. **Counting HTTP endpoint** — in-process `aiohttp` mock server with a
   per-request hit counter. **MANDATORY for both `DEMO_MODE=litellm_real`
   and `DEMO_MODE=litellm_deny`.** Why: `mock_response` (option 2) does
   NOT contact a real HTTP endpoint, so the "provider counter == 0"
   assertion in §3.2 / ACCEPTANCE.md §5.2 becomes vacuously true even
   if the callback is wired wrong (e.g. registered as `log_pre_api_call`
   instead of `async_pre_call_hook`, which fires AFTER the wire).
   Counter endpoint exposes `GET /__counters` returning
   `{"requests_received": N}`.
2. **`litellm.acompletion(..., mock_response="canned text")`** — built-in,
   skips HTTP entirely, returns canned response with plausible `usage`.
   **BANNED for Tier 3 demos** (per above). Acceptable for fast Tier 1
   unit tests that don't assert on wire-time counters.
3. **Real ollama on `localhost:11434`** — acceptable for the allow demo
   when the dev machine has it (counts as a counting endpoint via
   access log line count). The demo Makefile detects ollama presence;
   if absent, falls back to the counting HTTP endpoint (option 1) —
   **never** falls back to `mock_response`.
4. **`aioresponses` / `respx`** — for Tier 2 unit/integration tests
   simulating provider 5xx, rate-limit, partial streams. Acceptable
   because these tests don't assert on the deny demo's wire counter.

**Counting endpoint helper**: `_start_counting_provider(host="127.0.0.1",
port=...)` is implemented in `deploy/demo/demo/run_demo.py` by Slice 6
and shared by Slice 7 + Slice 9. It returns a context manager yielding
a `(url, counter_fn)` tuple; `counter_fn()` returns the current hit
count. Asserts are always against deltas (pre-call count vs post-call
count), never absolute, to allow positive controls before assertion
windows.

### 4.4 Budget seeding

Each integration test seeds its own tenant/budget/contract bundle in
`pg_session`'s setup phase. Pattern:

```sql
-- tests/integration/_fixtures/seed_litellm_budget.sql
INSERT INTO tenants ...
INSERT INTO budgets (..., amount_atomic) VALUES (..., :budget_amount);
INSERT INTO contracts ...
```

`:budget_amount=0` for deny-mode; large value for allow-mode. No test depends
on another test's seeded state. Per-tenant UUIDs derived from the test name
prevent parallel-worker collisions:

```python
tenant_id = str(derive_uuid_from_signature(
    test_name, scope="test_tenant_id"
))
```

---

## 5. Anti-patterns — DO NOT write these tests

### 5.1 Mock-the-thing-you-are-testing (BANNED at Tier 2/3)

```python
async def test_pre_call_hook_calls_request_decision(monkeypatch):
    client = Mock()
    client.request_decision = AsyncMock(return_value=fake_outcome)
    callback = SpendGuardLiteLLMCallback(client=client, ...)
    await callback.async_pre_call_hook(...)
    client.request_decision.assert_called_once()  # ← proves nothing
```

Passes even if SDK and sidecar speak different wire formats. The 14 bugs
from `feedback_demo_quality_gate.md` would all have slipped past this. The
integration tier MUST use a real `SpendGuardClient` against a real sidecar.

### 5.2 Dataclass-equality fluff

```python
def test_budget_binding_equality():
    b1 = BudgetBinding(budget_id="x", ...)
    b2 = BudgetBinding(budget_id="x", ...)
    assert b1 == b2  # ← @dataclass(frozen=True) gives this for free
```

Unless the test drives the binding through the resolver and into a real
`request_decision` call, it asserts only stdlib behaviour. Upgrade or delete.

### 5.3 `pytest.raises(Exception)` — too broad

```python
with pytest.raises(Exception):  # ← catches every bug
    await callback.async_pre_call_hook(...)
```

Passes if the SDK raises `KeyError` because a refactor broke unrelated code.
Always assert the typed exception AND inspect attributes:

```python
with pytest.raises(DecisionDenied) as exc_info:
    await callback.async_pre_call_hook(...)
assert exc_info.value.reason_codes == ["BUDGET_EXCEEDED"]
assert exc_info.value.decision_id is not None
```

### 5.4 Post-condition-blind tests

```python
async def test_reservation_created():
    await callback.async_pre_call_hook(...)
    # ... no assertion the reservation actually exists in Postgres
```

If a test invokes the wire and does not verify the post-condition landed,
it merely verified no exception was raised. Always `SELECT` the expected
row.

### 5.5 AGT-tests-pattern (forbidden in LiteLLM)

AGT integration lives at `sdk/python/src/spendguard/integrations/agt.py` but
at HEAD has **zero test files** (`grep -r "integrations.agt"
sdk/python/tests/` returns nothing; only test is `test_prompt_hash.py`). AGT
is verified only via `DEMO_MODE=agent_real_agt` (`run_agt_composite_mode` in
`run_demo.py`).

The LiteLLM slice MUST NOT replicate this gap. Tier 1 + Tier 2 coverage is
mandatory for every code path. The AGT precedent of "demo is enough" is
REJECTED here because:

1. LiteLLM has more code paths than AGT (pre-call + commit + failure +
   retry vs AGT's single evaluate call).
2. LiteLLM users customize `BudgetResolver` / `ClaimEstimator` /
   `ClaimReconciler` more aggressively than AGT users customize their
   PolicyDocument — unit tests pin these contracts.
3. CI cannot run the demo on every PR (compose too heavy); unit +
   integration tests are the per-PR signal.

If AGT's lack-of-tests is a gap, that's a separate follow-up; for LiteLLM
we ship with full coverage from slice 1.

### 5.6 Sleep-and-pray timing

```python
await callback.async_pre_call_hook(...)
await asyncio.sleep(2)  # ← hoping the event lands
rows = await pg.fetch("SELECT * FROM canonical_events ...")
```

Use a `wait_for_canonical_event` helper (poll until landed or timeout with
clear message). No arbitrary sleeps in tests.

---

## 5.7 Non-functional acceptance test mapping (P1.2 fix)

Each ACCEPTANCE.md NF and S clause is assigned to a specific test or
demo step. Coverage was missing in the pre-Phase-0 draft.

| Acceptance ID | Test / demo location | What it asserts |
|---|---|---|
| NF1 latency p50≤10ms / p99≤25ms | `tests/integration/test_litellm_precall_integration.py::test_pre_call_hook_latency_histogram` | 100 sequential calls; histogram artefact uploaded |
| NF2 50-way concurrency = 25 ALLOW + 25 DENY | `tests/integration/test_litellm_precall_integration.py::test_concurrent_50_way_hard_cap` | `asyncio.gather` 50 hooks against budget=25; exactly 25 `DecisionDenied` raised |
| NF3 memory leak | `tests/integration/test_litellm_proxy_subprocess.py::test_proxy_restart_no_leak` | tracemalloc snapshots ×5 spin-up/down cycles |
| NF4 zero module-level mutable state | `tests/test_litellm_skeleton.py::test_module_level_mutable_state_scan` | `ast.parse` source; only `_RUN_CONTEXT` ContextVar permitted |
| NF5 sidecar mid-stream disconnect | `tests/integration/test_litellm_streaming_integration.py::test_streaming_sidecar_disconnect` | kill sidecar between chunks; surface typed `SidecarUnavailable` |
| S1 every row has `litellm_call_id` | `verify_step_litellm_real.sql` Q1 + Q3 derived check | `decision_context_json->>'litellm_call_id' IS NULL` returns 0 |
| S2 frozen pricing tuple present | Same SQL — predicates over `pricing_version` / `price_snapshot_hash_hex` / `fx_rate_version` / `unit_conversion_version` | 4× `IS NULL` predicates return 0 |
| S3 no provider keys in SDK | `tests/test_litellm_skeleton.py::test_no_provider_api_key_handling` | grep over module source for `OPENAI_API_KEY` etc. → 0 hits |
| S4 ≤4 KiB decision_context_json | `verify_step_litellm_real.sql` `octet_length()` predicate | every row ≤ 4096 bytes |
| S5 SBOM licence allowlist | existing project SBOM CI job on the `[litellm]` extra | exit 0 with new extra |
| S6 fail-open WARNING per use + startup | `tests/integration/test_litellm_precall_integration.py::test_fail_open_warning_loud` | caplog captures WARNING at construction AND at each fail-open path taken |

---

## 6. Coverage targets (pragmatic, not aspirational)

| Tier | Target | Measurement |
|---|---|---|
| Tier 1 unit | 100% line coverage of `integrations/litellm.py` minus the `try/except ImportError` block | `pytest --cov=spendguard.integrations.litellm` |
| Tier 1 + Tier 2 combined | Every branch of `async_pre_call_hook`, `async_log_success_event`, `async_log_failure_event` exercised with both ALLOW and DENY outcomes | branch coverage report |
| Tier 3 demo | Both `litellm_real` and `litellm_deny` green on maintainer's dev machine AND in CI | `make demo-up DEMO_MODE=…` exit 0 |

We do not chase 100% of `__init__` boilerplate, dataclass declarations, or
`__all__`. Coverage is a guide; the gate is the demo.

Performance budget: Tier 1 < 10 s, Tier 2 < 90 s, each demo mode
< 60 s wall-clock.

---

## 7. Known flakes & flake budget

Per `project_known_demo_flakes.md`: `DEMO_MODE=ttl_sweep` is flaky (downstream
reserve flow bug unrelated to ttl_sweeper itself).

- LiteLLM integration MUST NOT depend on `ttl_sweep` running. Slices 5/6 do
  not call `DEMO_MODE=ttl_sweep` as a pre-step.
- MUST NOT inherit ttl_sweep-mode SQL into `verify_step_litellm_real.sql` or
  `verify_step_litellm_deny.sql`.
- Long-running-stream TTL behaviour is tested by Tier 2 with a tunable
  in-test TTL (e.g. 2 seconds), NOT by relying on the production
  ttl_sweep mode.

**Flake budget at acceptance: zero.** Any quarantined-as-flaky test in
Tier 1/2, or any intermittent demo failure on the same HEAD, blocks
acceptance. Required green:
- `pytest sdk/python/tests/ -v`
- `pytest sdk/python/tests/integration/ -v`
- `make demo-up DEMO_MODE=litellm_real`
- `make demo-up DEMO_MODE=litellm_deny`

…each run 3× from a clean state, all 12 runs green.

---

## 8. Summary

- **Three tiers**: unit / integration / demo. Each exposes bugs the previous
  cannot.
- **Mocking `SpendGuardClient` is BANNED** in Tier 2/3 — the point is to
  verify the real wire.
- **Demo is the gate**, not Codex. Slices 5/6 are the Tier 3 demos
  themselves; no separate "demo slice" beyond the demo runs.
- **Anti-pattern blocklist** enforced in code review.
- **Zero known-flaky tests at acceptance.** No dependency on `ttl_sweep`.

The two demo invariants most likely to expose wire-time bugs that no static
review can catch:

1. **`DEMO_MODE=litellm_deny`: provider HTTP request counter == 0 after a
   denied call.** Single observable assertion that proves the callback fires
   BEFORE the wire, not after. A wrong hook (`log_pre_api_call` instead of
   `async_pre_call_hook`) would make the deny demo PASS every other
   assertion (deny event written, no invoice) while still leaking money to
   the provider. Only the counter catches it.
2. **`DEMO_MODE=litellm_real`: `litellm_call_id` ↔
   `canonical_events.llm_call_id` join produces ≥1 matched row** for
   the proxy step (step 4) only. The operator's only way to reconcile
   LiteLLM's SpendLogs with SpendGuard's audit chain. If the
   derivation (`derive_uuid_from_signature("litellm:" + ..., scope=...)`)
   is wrong, every other assertion still passes locally but the join
   fails at scale and the audit story is silently broken. The join
   row count is the cheapest, most direct signal. ACCEPTANCE.md §5.1
   Q2 uses `≥1` (not "exactly one") to permit multiple commits per
   demo run without re-tuning the invariant — Round 2 P1.7 alignment
   fix.
