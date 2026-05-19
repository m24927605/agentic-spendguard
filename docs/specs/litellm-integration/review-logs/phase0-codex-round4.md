codex
## Executive Summary

Spec-lock recommendation: **GRANT WITH P0 FIXES**. The spec has clearly absorbed prior review feedback and the 10-slice shape is much stronger than the stated 7-slice/915-line summary, but several hard acceptance invariants are not actually enforced by the slice skeletons or SQL gates. The biggest risks are audit-context loss, provider calls proceeding after malformed reservations, and demo queries that can pass while the LiteLLM ⇄ SpendGuard join is broken. Fix the P0s before any code is written.

## P0 Findings

**IMPLEMENTATION.md: Slice 2 / DESIGN.md §8.2a / ACCEPTANCE.md S1–S2**  
Claim: mandatory `decision_context_json` fields are built but never passed to `request_decision` in the Slice 2 skeleton.  
Why this breaks: F4/S1/S2/Q2 depend on `litellm_call_id`, pricing tuple, `mode`, and `team_id` landing in `canonical_events`; following the skeleton yields audit rows that cannot satisfy the acceptance SQL or SpendLogs join.  
Suggested fix: make the Slice 2 skeleton explicitly pass `decision_context_json=decision_context`, and keep the real-sidecar test that proves those fields land row-by-row.

**IMPLEMENTATION.md: Slice 2 + Slice 3**  
Claim: `reservation_ids` cardinality is validated only after the provider call, in success/failure hooks.  
Why this breaks: if the sidecar/API drift returns zero or multiple reservations on ALLOW, the pre-call hook still returns and LiteLLM contacts the provider; the later `SpendGuardConfigError` happens post-spend.  
Suggested fix: enforce `len(outcome.reservation_ids) == 1` inside `async_pre_call_hook` before returning to LiteLLM; fail closed before wire contact.

**ACCEPTANCE.md §5.1 Q2 / TEST_PLAN.md §2.9 + §8**  
Claim: the LiteLLM_SpendLogs join query is not load-bearing. It uses a `LEFT JOIN` and does not filter/assert `ls.request_id IS NOT NULL`.  
Why this breaks: a broken audit correlation can still produce one `canonical_events` row with `ls.request_id = NULL`; a naive “row count ≥1” verifier passes while the killer invariant fails.  
Suggested fix: use an `INNER JOIN` or add an explicit unmatched-count query expecting `0`, scoped to the proxy commit row.

**ACCEPTANCE.md F7 / IMPLEMENTATION.md Slice 4 / TEST_PLAN.md §2.4**  
Claim: F7 requires streaming commit with real reconciled usage, but Slice 4 explicitly allows missing `.usage` to fall back to the estimator.  
Why this breaks: the implementation can overcharge/under-reconcile streaming calls while still following the slice spec; “commit amount reflects actual usage” becomes provider-dependent and not provider-agnostic.  
Suggested fix: either make missing usage a hard failure for v1 streaming acceptance, or clearly mark estimator fallback as a non-F7 degraded path that cannot satisfy the demo gate.

**ACCEPTANCE.md §5.2 / TEST_PLAN.md §4.3 / IMPLEMENTATION.md Slice 7**  
Claim: deny-demo counter semantics conflict: acceptance says `requests_received == 0`, implementation asks for a positive-control allow, and the test plan later says all assertions are deltas.  
Why this breaks: the key fail-closed demo is either impossible with a positive control or can skip the positive control and go vacuous.  
Suggested fix: align all docs on “per-substep delta == 0 after a verified positive control,” with counter reset/snapshot semantics stated once.

**IMPLEMENTATION.md Slice 4 / ACCEPTANCE.md NF5 / TEST_PLAN.md §2.4 test 9**  
Claim: NF5 requires `SpendGuardSidecarUnavailable` at commit boundary, but the Slice 4 skeleton re-raises raw `SpendGuardError` on commit failure.  
Why this breaks: typed fail-closed semantics drift exactly where streaming is highest risk; tests and acceptance will disagree with code written from the skeleton.  
Suggested fix: wrap commit-boundary sidecar failures as `SpendGuardSidecarUnavailable` while preserving stash for retry/TTL behavior.

## P1 Findings

**IMPLEMENTATION.md Slice 2 / DESIGN.md §8.1**  
Claim: if `litellm_call_id` is missing, the hook mints one but does not propagate it to later success/failure hooks.  
Why this breaks: the provider call can proceed, then `_get_stash(kwargs)` cannot find the reservation, causing no commit/release until TTL.  
Suggested fix: fail closed when LiteLLM does not provide a usable call id, unless a proven callback-safe propagation path exists.

**IMPLEMENTATION.md Slice 4 / DESIGN.md §7.1**  
Claim: TTL plumbing is promised, but no `request_decision` API change for `ttl_seconds` is specified.  
Why this breaks: streaming TTL is a hard part of F7/NF5, but the implementation plan does not identify the wire field or SDK signature change.  
Suggested fix: specify the exact client/sidecar field used for reservation TTL and test it against the real sidecar.

**IMPLEMENTATION.md §4 cross-slice invariants / Slice 1 / TEST_PLAN.md §2.8**  
Claim: `_StashSweeper` ownership is contradictory: cross-slice invariant says Slice 1 adds it, skeleton omits it, tests appear under Slice 8.  
Why this breaks: stale stash entries remain until process death or are tested too late, undermining NF3 and retry/stream behavior.  
Suggested fix: assign sweeper implementation and tests to the same slice.

**DESIGN.md §6 / IMPLEMENTATION.md §5**  
Claim: API surface says one `litellm.py` file “~250 lines target,” but SDK slices 1–5 alone budget 550 cumulative lines.  
Why this breaks: reviewers may enforce the wrong size target or miss that this is closer to the pydantic-ai integration than `agt.py`.  
Suggested fix: update DESIGN to say the final module is expected to be ~500–600 lines, with ≤250 additions per slice.

**IMPLEMENTATION.md Slice 1 / IMPLEMENTATION.md §5**  
Claim: Slice 1’s local line budget is 208 lines, but the rollup table lists 160.  
Why this breaks: H1/H7 review logs cannot objectively check the slice budget.  
Suggested fix: make the rollup match the per-slice section.

**DESIGN.md §7.2 / IMPLEMENTATION.md Slice 8 / ACCEPTANCE.md F1**  
Claim: proxy callback registration alternates between scalar dotted path and “list form wiring.”  
Why this breaks: LiteLLM proxy callback loading is an integration-critical wire point; ambiguity here can produce a demo that never loads the callback.  
Suggested fix: pick one exact YAML shape and mirror it verbatim across all docs.

**DESIGN.md §7.2 / IMPLEMENTATION.md Slice 8**  
Claim: the proxy template is called copy-pasteable but leaves `claim_estimator=...` and `claim_reconciler=...`.  
Why this breaks: Slice 9 says the demo uses this output; placeholders make the proxy step non-runnable unless hidden code is added later.  
Suggested fix: distinguish illustrative docs from runnable demo template, and make the demo template concrete.

**ACCEPTANCE.md §9.2 / REVIEW_STANDARDS.md §7.1**  
Claim: SDK slices 2–5 run only `DEMO_MODE=decision`, not a LiteLLM callback demo.  
Why this breaks: the Pydantic-AI “tests pass but wire broken” class can survive until Slice 6/9.  
Suggested fix: once Slice 2 exists, require a minimal `litellm.acompletion` demo/regression for callback-touching slices.

## P2 Findings

- **TEST_PLAN.md §3.3**: stale “`mock_response` / ollama path is default” conflicts with the Tier 3 ban on `mock_response`; remove the `mock_response` mention.
- **TEST_PLAN.md §1.3**: “LiteLLM_SpendLogs row also written” is not proxy-qualified in the invariant list; later sections qualify it correctly.
- **IMPLEMENTATION.md Slice 5**: failure hook releases only the first reservation when multiple are present; that leaks the rest if the malformed sidecar response path ever happens.
- **ACCEPTANCE.md C2 vs IMPLEMENTATION.md §9**: final pass allows deferred P1s, but Definition of Done says no unresolved P0/P1; align the ship bar.
- **ACCEPTANCE.md F3 / Slice 9**: “POST ... with `team_id`” can read like a spoofable header path, despite the recipe requiring authenticated `UserAPIKeyAuth`.
- **TEST_PLAN.md §5.7 / IMPLEMENTATION.md §5**: test LOC policy differs between ≤250 per slice, ≤200 per file, tests excluded from implementation rollup, and total ≤1500.
- **IMPLEMENTATION.md Slice 6/7**: files touched omit Makefile/demo dispatch details that TEST_PLAN says must land.
- **DESIGN.md §6 vs IMPLEMENTATION.md Slice 1**: `BudgetBinding.unit` is public as `common_pb2.UnitRef` in design but `Any` in skeleton.

## P3 Findings

- “LiteLLM is the most-deployed open-source LLM gateway” is unsupported and not needed for the spec.
- Absolute local path in ACCEPTANCE.md §5 makes the command less portable.
- “No global state” should explicitly exempt intentional mutation of `litellm.callbacks`.
- Single-PR/no-squash strategy is process-heavy but not a correctness issue.
- Repeated “Codex is necessary, not sufficient” language could be shortened after acceptance.

## What you tried to break but couldn't

The 7-slice/915-line concern is mostly already fixed in the actual docs: the supplied spec now consistently uses a 10-slice plan and a 1400-line ceiling, aside from smaller budget drift. The sync `litellm.completion()` silent-bypass risk is addressed with a pre-wire sync hook that raises. The test plan does cover the major streaming edge cases called out in the prompt: early generator abandon, retry mid-stream, partial final chunk, network reset, and sidecar failure at commit boundary. The review protocol also does not livelock indefinitely: round 5 forces split, defer, or owner escalation.
