codex
I’ll review this as a spec set, not as implementation guidance: first cross-document contradictions, then budget/test/review-loop risks, and I’ll keep the findings tied to specific sections.
codex
## Executive Summary

Spec-lock recommendation: **DENY**. The five docs do not describe one coherent v1: acceptance requires sync support, proxy support, streaming behavior, SpendLogs joins, and fail-closed demos that the implementation plan either defers, omits, or contradicts. The most dangerous gaps are audit correlation, proxy identity resolution, streaming acceptance, and demo invariants that can pass vacuously. Code should not start until the P0s below are reconciled.

## P0 Findings (must-fix-before-code)

1. **DESIGN.md §2.1 G3 / §9 ADR-005 / ACCEPTANCE.md §8 N5 — v1 surface contradicts itself.**  
   G3 says cover `litellm.completion()`; ADR-005 and acceptance explicitly exclude sync support. This will split implementers and reviewers on whether sync hooks are required. Suggested fix: make G3 match the async-only v1 scope, or move sync support back into implementation and tests.

2. **IMPLEMENTATION.md Slice 2 / DESIGN.md §6 / Slice 7 — proxy resolver cannot see proxy identity.**  
   `async_pre_call_hook` calls `budget_resolver(data)`, but proxy identity arrives as `user_api_key_dict`; Slice 7’s resolver reads `kwargs["user_api_key_dict"]`. Multi-tenant proxy budgeting will silently resolve `team_id=None`. Suggested fix: require the pre-call hook to build a resolver input containing `user_api_key_dict`, `team_id`, `key_alias`, metadata, and model.

3. **ACCEPTANCE.md §5.1 / IMPLEMENTATION.md Slices 5–7 / TEST_PLAN.md §3.1 / REVIEW_STANDARDS.md §7.3 — `litellm_real` demo has three incompatible definitions.**  
   Acceptance requires ALLOW, DENY, STREAM, and PROXY steps; implementation Slice 5 is allow-only, Slice 6 is separate deny, Slice 7 defers proxy demo, and Review Standards lists only three steps. The demo gate cannot be implemented unambiguously. Suggested fix: make one authoritative demo sequence and align all docs.

4. **ACCEPTANCE.md F7 + NF5 / IMPLEMENTATION.md Slice 5 out-of-scope / TEST_PLAN.md §2 — streaming is accepted but not sliced or tested.**  
   F7 requires stream commit with real usage; NF5 requires typed behavior when sidecar dies mid-stream. The plan defers streaming demo and has no tests for early termination, retry mid-stream, partial chunks, or SSE reset. Under Shape B, sidecar is not contacted during chunks, so NF5 is especially suspect. Suggested fix: either remove streaming from v1 acceptance or add an explicit implementation/test slice.

5. **DESIGN.md §8.3 + §11 / IMPLEMENTATION.md Slice 5 / ACCEPTANCE.md F4, S1, S2, Q2 — audit join story is not implementable as written.**  
   Direct `litellm.acompletion()` does not necessarily create `LiteLLM_SpendLogs`; yet Slice 5 asserts that row. Also Slice 2 does not specify passing `integration`, `litellm_call_id`, or frozen pricing into `decision_context_json`, making S1/S2 and Q2 impossible. Suggested fix: scope SpendLogs joins to proxy mode or require proxy in the demo, and specify the exact decision context fields sent to the sidecar.

6. **TEST_PLAN.md §8 / DESIGN.md §8.2 / ACCEPTANCE.md §5.1 Q1–Q2 — “exactly one row” audit invariant conflicts with the event model.**  
   Design creates multiple canonical events per call with the same `llm_call_id`; the killer invariant says the join is exactly one row. Depending on the query, the correct count is one commit row, four event rows, or three committed calls in the full demo. Suggested fix: qualify the invariant by event type.

7. **IMPLEMENTATION.md Slice 2–4 / TEST_PLAN.md §2.2 — reservation identity is lost.**  
   Slice 2 stash omits `reservation_id`; Slices 3 and 4 pass `stash.get("reservation_id", "")`; tests require reservation_id populated. Commit/release may no-op or fail against the sidecar. Suggested fix: require Slice 2 to stash the sidecar-returned reservation id, or state that decision_id alone is the sidecar contract and update tests.

8. **DESIGN.md §5 / IMPLEMENTATION.md Slices 1,2,6 / TEST_PLAN.md §3.2 — deny exception contract is split.**  
   Design names `SpendGuardDenied`; implementation and tests use `DecisionDenied`. Typed exception behavior is a core public API and demo assertion. Suggested fix: choose one exported exception name and use it everywhere.

9. **DESIGN.md ADR-002 / REVIEW_STANDARDS.md §9.6 / TEST_PLAN.md §2.4 — retry behavior contradicts the review gate.**  
   Design says reserve every retry attempt with distinct `litellm_call_id`; Review Standards calls reserving every retry without shared decision_id a P0. The planned implementation will be blocked by its own review protocol. Suggested fix: align Review Standards with ADR-002 or change ADR-002.

10. **DESIGN.md §7.1 / IMPLEMENTATION.md Slice 2 / TEST_PLAN.md §2.2 / ACCEPTANCE.md §5.2 — default budget fallback is contradictory.**  
   Design defines `SPENDGUARD_LITELLM_DEFAULT_BUDGET_ID`; tests and acceptance expect fallback; implementation raises `SpendGuardConfigError` immediately when resolver returns `None`. Suggested fix: either implement the fallback path or delete it from design/tests/acceptance.

11. **TEST_PLAN.md §4.3 / §3.2 — deny “provider counter == 0” can pass vacuously.**  
   CI may use `mock_response`, where no provider HTTP endpoint exists, so the counter is implicitly zero even if the hook is wrong. This misses the exact “tests pass but wire is broken” class cited in the prompt. Suggested fix: deny demo must always use a counting HTTP endpoint.

## P1 Findings (should-fix-before-merge)

1. **IMPLEMENTATION.md §5 — 915-line estimate excludes required work.**  
   Tests, fixtures, SQL verify files, Makefile branches, docs site pages, and streaming/proxy acceptance work are not in the rollup. The budget is not credible.

2. **ACCEPTANCE.md NF1–NF5, S5–S6 / TEST_PLAN.md §2 — acceptance gates lack test mapping.**  
   Latency, 50-way concurrency, memory, SBOM, and fail-open warnings are required but not assigned to slices.

3. **DESIGN.md §7.1 / IMPLEMENTATION.md Slice 2 / ACCEPTANCE.md S6 — fail-open env semantics conflict.**  
   Design says env read at construction; implementation checks env per error path and also requires `fail_closed=False`; acceptance requires warnings on startup and per use.

4. **DESIGN.md §5 — ledger-down `DEGRADED` allows calls despite hard-cap positioning.**  
   If Postgres is down, allowing LLM calls undermines the fail-closed/hard-budget claim unless explicitly scoped as a known degraded mode.

5. **IMPLEMENTATION.md Slice 2 Codex focus — `data["spendguard"]` mutation is unresolved.**  
   The spec knows this may be serialized to provider kwargs but leaves it as “verify.” That is too late for a core hook contract.

6. **IMPLEMENTATION.md Slice 2 — fallback `run_id = new_uuid7()` weakens idempotency.**  
   Users without `run_context()` get random run ids, so repeated handling of the same `litellm_call_id` may not dedupe.

7. **IMPLEMENTATION.md Slice 7 / ACCEPTANCE.md F3 — proxy callback import-time `run_until_complete` is risky.**  
   LiteLLM proxy may import inside an active event loop; the spec calls this acceptable for an example, but acceptance requires the proxy path to work.

8. **DESIGN.md §6 / IMPLEMENTATION.md Slice 3 / TEST_PLAN.md §2.2 — claim cardinality conflicts.**  
   Estimator tests allow two claims; commit rejects anything except exactly one. This is incompatible with “token + dollar” claims unless v1 is explicitly single-claim.

9. **ACCEPTANCE.md D1–D3 / IMPLEMENTATION.md Slices — public documentation acceptance has no slice.**  
   Required docs pages and related-link updates are absent from the implementation plan.

## P2 Findings (nice-to-fix)

- **DESIGN.md §7.2 / IMPLEMENTATION.md Slice 5 — sidecar env var names differ:** `SPENDGUARD_SIDECAR_SOCKET` vs `SPENDGUARD_SIDECAR_UDS`.
- **IMPLEMENTATION.md Slice 7 / TEST_PLAN.md §2.7 — team budget env names differ:** `SPENDGUARD_BUDGET_FOR_TEAM_{team_id}` vs `SPENDGUARD_BUDGET_team-a`.
- **IMPLEMENTATION.md Slice 7 / TEST_PLAN.md §2.5 — proxy file paths differ:** `deploy/demo/litellm_proxy` vs `deploy/demo/litellm`.
- **DESIGN.md §8.2 / TEST_PLAN.md §3.1 — event names are abbreviated inconsistently:** `ALLOWED/COMMITTED` vs `DECISION_ALLOWED/INVOICE_COMMITTED`.
- **IMPLEMENTATION.md Slice 3 / TEST_PLAN.md §2.3 — client method names drift:** `emit_llm_call_post` vs `commit_invoice`.
- **DESIGN.md §7.2 / ACCEPTANCE.md F1 — callback config alternates between string and list forms.**
- **REVIEW_STANDARDS.md §7.1 / TEST_PLAN.md §2.1 — Slice 1 demo expectations conflict: none vs `DEMO_MODE=decision` regression.**

## P3 Findings (deferable)

- Provider-agnostic “100+ providers from day 1” is overclaimed without at least one non-OpenAI provider fixture.
- Latency estimates are asserted before the gRPC path is measured.
- “Largest existing integration” line-count comparison is not very meaningful because this spec moves much of the complexity into demos, fixtures, and operator templates.

## What you tried to break but couldn't

The Shape A/B/C trade-off is explicit and mostly honest about double-counting and visibility trade-offs. The ban on mocking `SpendGuardClient` in Tier 2/3 is strong and directly addresses prior wire-time failures. The max-five-round review loop does have an escape hatch, so it is not an infinite loop if followed. The deny-path provider counter is the right invariant in principle; it only fails because the test plan lets CI replace the real counter with `mock_response`.
tokens used
