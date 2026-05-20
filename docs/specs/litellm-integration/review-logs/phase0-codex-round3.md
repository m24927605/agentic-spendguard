codex
I’m reviewing the supplied spec text directly as the source of truth. I’m focusing on hard cross-doc contradictions first, then runtime/demo holes where the docs could let a broken implementation pass.
codex
## Executive Summary

Spec-lock recommendation: **DENY**. The docs are much stronger than the initial 7-slice/915-line summary, but the current 10-slice set still has fatal cross-doc contradictions around public API shape, proxy bootstrap, mandatory audit fields, sync fail-closed behavior, and commit retry/audit guarantees. The streaming section has improved edge-case coverage, but F7/NF5 still contains impossible or under-specified behavior for v1 end-of-stream commit. Fix the P0s before implementation begins.

## P0 Findings

**DESIGN.md §8.2a / ACCEPTANCE.md F4,S1 / TEST_PLAN.md §2.2 — Mandatory decision-context field count is contradictory.**  
DESIGN defines 12 fields, including `mode` and `team_id`, but ACCEPTANCE and TEST_PLAN repeatedly say “10 fields,” and TEST_PLAN’s unit assertion omits `mode`/`team_id`. This breaks Q2 because ACCEPTANCE §5.1 filters proxy commits on `decision_context_json->>'mode' = 'proxy'`. Suggested fix: align every doc/test reference to the same mandatory field set.

**IMPLEMENTATION.md Slice 2 — `decision_context_json` is built but never actually passed to `request_decision`.**  
The skeleton only comments that fields “flow to sidecar via runtime_metadata,” but names no real argument or helper. If copied literally, F4/S1 audit-chain acceptance fails while unit tests may still pass. Suggested fix: specify the exact public SDK parameter/wire path and make Slice 2 tests assert the persisted row.

**DESIGN.md §6 / IMPLEMENTATION.md Slice 1, Slice 8 / TEST_PLAN.md §2.8 — `_LoopBoundCallback` is a phantom/inconsistent public symbol.**  
`__all__` exports `_LoopBoundCallback`, Slice 8 tests require `handler_instance` to be an instance of it, but DESIGN §7.2 defines `_LoopBoundCallback` locally in the operator template and Slice 1’s skeleton never defines it. This can break import/export and the proxy template. Suggested fix: decide whether `_LoopBoundCallback` lives in SDK or only in the operator template, then update `__all__`, Slice 8, and tests consistently.

**IMPLEMENTATION.md Slice 8 — Proxy bootstrap guidance simultaneously requires lazy loop binding and mentions `asyncio.run`.**  
The goal says `_LoopBoundCallback` lazy init is mandatory, while Outputs/Out-of-scope/Codex focus still reference an `asyncio.run` example. That reintroduces the event-loop-affinity bug the spec claims to have fixed. Suggested fix: remove the stale `asyncio.run` instructions and make Slice 8 exclusively describe the lazy serving-loop pattern.

**DESIGN.md ADR-005 / IMPLEMENTATION.md Slice 1 / TEST_PLAN.md §3.2 — Sync fail-closed semantics are internally contradictory and untested.**  
ADR-005 says `log_pre_api_call` raises before the provider HTTP request, but TEST_PLAN warns `log_pre_api_call` fires after the wire. If the latter is true, sync `litellm.completion()` still spends money before erroring. Suggested fix: verify LiteLLM hook ordering and add a counting-provider test for sync misuse, or remove the pre-wire claim.

**DESIGN.md §5 / IMPLEMENTATION.md Slice 3 / TEST_PLAN.md §2.3 — Commit retry/idempotency is claimed but not implementable from the slice skeleton.**  
Failure mode says partial commit timeout can be retried idempotently, but `_extract_stash()` pops before `emit_llm_call_post`; a second success hook becomes a no-op. This can leave provider spend without `INVOICE_COMMITTED`, violating G4/F4. Suggested fix: assign retry ownership explicitly and do not make the only retry state disappear before commit is durably acknowledged.

**IMPLEMENTATION.md Slice 4 / TEST_PLAN.md §2.4 — Streaming missing-usage fallback has no data path.**  
Slice 2 stashes `estimator_claims`, and tests require fallback when `response_obj.usage` is missing, but `ClaimReconciler(ctx, response_obj)` cannot see the stashed estimate. This makes a required streaming edge case impossible to implement as specified. Suggested fix: make the callback own the fallback or expose the estimate through a defined input.

**ACCEPTANCE.md NF5 / DESIGN.md ADR-003 / IMPLEMENTATION.md Slice 4 — “Sidecar killed mid-stream surfaces typed exception” is impossible under v1 as written.**  
Shape B only talks to the sidecar pre-call and at end-of-stream; it has no chunk-level sidecar interaction. Killing the sidecar between chunks cannot surface until commit, after provider tokens have already streamed. Suggested fix: restate NF5 to match end-of-stream commit semantics or add an explicit v1 mechanism; as written it is not a valid acceptance gate.

## P1 Findings

**ACCEPTANCE.md §5.1 Q2 / TEST_PLAN.md Summary — Audit join invariant weakened and inconsistent.**  
The prompt-level invariant says exactly one `litellm_call_id ⇄ canonical_events.llm_call_id` row, but ACCEPTANCE Q2 checks `decision_context_json->>'litellm_call_id'` to SpendLogs and only requires `≥1`. This can miss duplicate or wrong `canonical_events.llm_call_id` derivation in the demo. Suggested fix: align the invariant and query.

**IMPLEMENTATION.md Slice 2 / DESIGN.md §6 — Estimator “exactly one claim” contract is not enforced pre-wire.**  
Reconciler multi-claim is rejected, but estimator multi-claim can reach the sidecar and provider, then fail later at commit due multi-reservation. Suggested fix: Slice 2 should reject 0 or >1 claims before `request_decision`.

**IMPLEMENTATION.md §4 Cross-slice invariants — Stash TTL sweep is required but not owned by a slice.**  
The invariant says Slice 1 adds a background sweep, but Slice 1’s goal/skeleton omit it and tests only appear under Slice 8. This can leave proxy memory growth unimplemented. Suggested fix: assign the sweep to a concrete slice with constructor/loop-affinity behavior.

**REVIEW_STANDARDS.md §3.4 / ACCEPTANCE.md C1 — Stopping rule can pass with old unresolved critical-path P1s.**  
Condition B only checks zero new P1 in the final round, not that earlier critical-path P1s were fixed/deferred/disputed and accepted. Suggested fix: require all critical-path P1s from rounds 1..N to be addressed before STOPPING-RULE-MET.

**ACCEPTANCE.md §9 / REVIEW_STANDARDS.md §7.1 — Per-slice demo gate is unclear for Slices 1–5.**  
Acceptance says each slice’s demo mode must PASS, while TEST_PLAN gives Slices 1–5 no Tier 3 demo. REVIEW_STANDARDS says use `decision` regression until LiteLLM mode exists, but that is not reflected in ACCEPTANCE. Suggested fix: make the early-slice demo requirement explicit.

**TEST_PLAN.md §2.4 — Streaming consumer-abandons-generator case is not covered.**  
The plan covers cancellation, provider reset, retries, and partial chunks, but not a caller simply breaking out of `async for` without a clean final usage frame. That is a realistic leak-to-TTL path. Suggested fix: add or explicitly defer that case.

## P2 Findings

**IMPLEMENTATION.md Slice 8 / ACCEPTANCE.md F1 — Proxy callback YAML form drifts between scalar string and list form.**  
DESIGN shows `callbacks: spendguard_litellm_proxy_callback.handler_instance`; Slice 8 says list-form wiring; ACCEPTANCE says string form is proxy-only. Pick one parseable LiteLLM form.

**DESIGN.md §6 — `BudgetResolver` type excludes `None` while docs define `None` error behavior.**  
Either type it as optional or define `None` as runtime misuse outside the public type.

**TEST_PLAN.md §2.4 — Additional streaming tests are numbered 5–8 after 1–3.**  
Minor, but it makes checklist tracking sloppy.

**IMPLEMENTATION.md Slice 1 — Out-of-scope text says sync hooks are out of scope while skeleton implements `log_pre_api_call`.**  
Not fatal if ADR-005 is corrected, but the slice text should stop contradicting itself.

## P3 Findings

- `REVIEW_STANDARDS.md` header still says `IMPLEMENTATION.md (TBD)` despite the full implementation plan existing.
- The initial prompt summary’s 7-slice/915-line description is stale versus the actual docs’ 10-slice/1400-line plan.
- Several “Round 2 fix” annotations inside final docs are useful history but noisy for implementers.

## What you tried to break but couldn't

The 7-slice/915-line undercount is fixed in the actual IMPLEMENTATION body: it now uses 10 slices, includes docs/demo/SQL in the rollup, and admits the 1400-line ceiling. The deny-path counter invariant is load-bearing and well protected: Tier 3 bans `mock_response`, requires a real counting endpoint, and includes positive-control language. The direct-vs-proxy `LiteLLM_SpendLogs` distinction is also correctly addressed; Q2 is scoped to proxy mode only, avoiding a false acceptance failure for direct `acompletion()`. Streaming coverage is much better than the summary implied, with tests for provider error after chunks, partial final chunk, retry mid-stream, and network reset.
