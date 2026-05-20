</stdin>
codex
## Executive Summary

Recommendation: **DENY** spec-lock until the P0s are fixed. The docs are much stronger than the initial 7-slice/915-line summary implies, but several acceptance gates are currently impossible or under-specified. The highest-risk failures are proxy-mode reality gaps, audit-query drift, retry acceptance drift, and a documented ledger-down allow path that contradicts hard-cap/audit guarantees. The demo strategy is directionally good, but one remaining `mock_response` allowance would undercut the main wrong-hook invariant.

## P0 Findings

1. **DESIGN.md §5 vs ACCEPTANCE.md F4/S1: ledger-down `DEGRADED → ALLOW` violates audit-chain coverage.**  
   Why this would break: F4/S1 require every LiteLLM call that reaches the wire to create canonical audit rows with LiteLLM context. If Postgres is down and the sidecar allows, the call can spend money without the signed audit chain and without hard-cap enforcement.  
   Suggested fix: reconcile this explicitly before code: either fail closed on ledger outage for LiteLLM, or narrow F4/S1/G2 and add a test/demo for the degraded exception.

2. **ACCEPTANCE.md §5.1 Q2 + DESIGN.md §8.2a + IMPLEMENTATION.md Slice 2/3: Q2 filters on phantom `decision_context_json->>'route'`.**  
   Why this would break: DESIGN mandates 10 decision-context fields and `route` is not one of them. Slice 2’s `_build_decision_context` also omits `route`, while Slice 3’s commit call does not pass context at all. The proxy audit join can return zero rows even if the integration works.  
   Suggested fix: make Q2 use fields the SDK actually emits, or make `route`/proxy identity a mandatory emitted field and verify it exists on `INVOICE_COMMITTED`.

3. **ACCEPTANCE.md F6 vs IMPLEMENTATION.md Slices 6/7/9 and TEST_PLAN.md §2.5: retry acceptance requires an in-demo step that does not exist.**  
   Why this would break: F6 says retry double-charge prevention is verified “by an in-demo step,” but neither demo stdout nor any demo slice includes retry/500 injection. TEST_PLAN only has Tier 2 integration coverage and says retry is “not direct” demo coverage.  
   Suggested fix: either add the retry scenario to the demo contract or change F6 verification to the named integration test. As written, acceptance cannot be satisfied.

4. **TEST_PLAN.md §1.3 vs §3.1/§4.3: Tier 3 still allows `mock_response` fallback in CI.**  
   Why this would break: §3.1 and §4.3 correctly ban `mock_response` because provider-counter == 0 becomes vacuous. But §1.3 says `mock_response` fallback in CI is allowed for the demo. That reintroduces the exact wrong-hook bug the “killer invariant” is meant to catch.  
   Suggested fix: remove the Tier 3 `mock_response` fallback everywhere; use only the counting HTTP endpoint or real localhost provider for demos.

5. **DESIGN.md §7.2 / IMPLEMENTATION.md Slice 8: proxy template handshakes with `asyncio.run()` then reuses the async client on LiteLLM’s event loop.**  
   Why this would break: async gRPC/UDS clients are commonly event-loop-affine. Creating the client/channel inside a temporary `asyncio.run()` loop, closing that loop, then using the same client in the proxy’s serving loop can fail at first request. This threatens the entire proxy path.  
   Suggested fix: bootstrap the SpendGuard client on the same loop that will execute callbacks, or prove with a real proxy subprocess test that the current client is loop-independent.

6. **ACCEPTANCE.md F3/§5.1 + IMPLEMENTATION.md Slices 8/9: proxy `team_id` source is under-specified.**  
   Why this would break: the proxy step says `POST /v1/chat/completions team=t1`, but the resolver reads `user_api_key_dict.team_id`. The docs do not specify LiteLLM team/key seeding or the exact auth path that makes `UserAPIKeyAuth.team_id` populated. A header-only “team_id” can be ignored or spoofed.  
   Suggested fix: specify the proxy auth/team setup that yields `user_api_key_dict.team_id`, and make the demo seed/use that path.

7. **DESIGN.md ADR-005 / NG6: sync `litellm.completion()` is documented out of scope but not blocked.**  
   Why this would break: if a process installs the callback globally and then uses sync `completion()`, the call may bypass enforcement silently. “Document async only” is not fail-closed behavior for an enforcement integration.  
   Suggested fix: make sync hooks fail before the wire, or explicitly prove LiteLLM cannot invoke the registered callback for sync calls in a way users would confuse as protected.

## P1 Findings

1. **IMPLEMENTATION.md Slice 4 + TEST_PLAN.md §2.4: streaming fallback to estimator is not actually available.**  
   The reconciler signature is `(ResolverContext, response_obj)` and the stash does not retain estimator output. Tests require fallback when `response_obj.usage` is missing, but the data path is absent.

2. **TEST_PLAN.md §2.4/§5.7 vs ACCEPTANCE.md F7/NF5: streaming edge coverage is incomplete.**  
   Tests cover success, caller cancellation, long stream, and sidecar disconnect, but not provider network reset mid-SSE, partial final chunk, provider error after chunks, or LiteLLM retry during streaming.

3. **IMPLEMENTATION.md Slice 2 vs TEST_PLAN.md §2.2: stash location contradicts itself.**  
   TEST_PLAN says the allow path stashes into `data["spendguard"]`; implementation and unit tests require no `spendguard` key on returned data. This can send implementers toward the provider-wire leak the spec is trying to prevent.

4. **ACCEPTANCE.md §5.2 failure guide contradicts ADR-001/default-budget removal.**  
   It says fallback chain includes `SPENDGUARD_LITELLM_DEFAULT_BUDGET_ID`, but DESIGN §7.1 and Slice 2 explicitly removed that env var.

5. **IMPLEMENTATION.md Slice 3/5: `_stash.pop()` on success/failure has leak and race risk.**  
   If LiteLLM never fires either terminal callback, or stream consumption is abandoned, SDK memory grows even though ledger TTL eventually releases. NF3 checks proxy restart leakage, not per-call stash leakage.

6. **ACCEPTANCE.md §5.1 Q3 hash-chain query may assert the wrong genesis count.**  
   Expected `1` assumes the first event in a session has no previous event. If canonical_events is a global or tenant chain, the first session event can validly point to a prior event outside the session.

7. **TEST_PLAN.md §8 says join produces “exactly one” row; ACCEPTANCE.md §5.1 says `≥1`.**  
   This is small but acceptance-relevant drift for the second “killer invariant.”

## P2 Findings

- **IMPLEMENTATION.md Slice 3 code skeleton:** `_extract_stash` is marked `@staticmethod` but references `self`; likely copy/paste bug.
- **TEST_PLAN.md §2.8:** `test_template_module_loads` is called unit coverage but the template’s top-level `_client = asyncio.run(_bootstrap_client())` needs a live sidecar.
- **DESIGN.md §7.1:** “Slice 4 + Slice 9” TTL wording is okay but stale-ish now that the slice plan is 10 slices.
- **IMPLEMENTATION.md §5:** total line budget is now 1400, while the project-context summary still says 915. The actual docs are internally updated, but rollout notes should stop repeating 915.

## P3 Findings

- Demo stdout uses exact Unicode punctuation; if shell/log normalization has bitten this project before, consider ASCII-only matching.
- `LiteLLM_SpendLogs` table/column casing should be verified against the actual LiteLLM version pin before implementation.
- `litellm_call_id` fallback to `new_uuid7()` should probably be loud, not silent, because it weakens reconciliation.

## What you tried to break but couldn't

The 7-slice/915-line concern is mostly already fixed in the actual docs: IMPLEMENTATION is now 10 slices with a 1400-line ceiling and a pre-authorized Slice 9 split. The direct-mode `LiteLLM_SpendLogs` issue is also handled well: DESIGN, TEST_PLAN, and ACCEPTANCE now limit SpendLogs joins to proxy mode. The test plan correctly bans `SpendGuardClient` mocks in Tier 2/3 and requires a counting provider for deny-path demos in the detailed sections; the remaining problem is the conflicting Tier 3 `mock_response` fallback sentence.
