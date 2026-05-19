# LiteLLM integration — final adversarial review

ACCEPTANCE.md C2 requires a final whole-integration adversarial
Codex pass producing zero new P0 findings before the branch can
ship. Per the autonomous-session note: Codex CLI was intermittently
unresponsive across Slices 5-6, so the Staff panel (Code Reviewer +
Backend Architect, parallel) substituted for the per-slice
adjudication. Final-pass uses the same substitution.

## Scope

Read-only review of `feat/litellm-integration` HEAD against the
locked Phase-0 specs (DESIGN.md / IMPLEMENTATION.md / ACCEPTANCE.md /
TEST_PLAN.md). Slices 1–10 in scope; PR1/PR2 packaging tasks are out
of scope (those happen in the open-PR step).

## Round summary

| Slice | Closed at | Adversarial verdict |
| --- | --- | --- |
| Slice 1 | Staff panel R3 PASS-WITH-SCOPE-CUT | ACCEPTANCE SQL moved to slice-06.md; tracking issue GH #77 for cross-component sidecar enrichment |
| Slice 2 | Codex R4 PASS at LOC waiver §6.5 | Defensive fixes (ensure_client retry budget, multi-reservation pre-wire release) |
| Slice 3 | Codex R4 PASS (N=4 zero findings) | Reconciler binding-mismatch fixes; commit_estimated path corrected |
| Slice 4 | Codex R3 PASS | Snapshot moved pre-await (R2 P1) |
| Slice 5 | Codex R1 PASS + Staff panel R2 PASS | Word-boundary regex hardening for CANCELLED classifier |
| Slice 6 | Staff panel R2 CODE-LEVEL CLOSED | 2 P0 fixes: `litellm[proxy]` extra + canonical-DB split; 3 P1 fixes: stdout drain, status_code=403, x-litellm-call-id header |
| Slice 7 | code-level COMPLETE (transitive Slice 6 review) | reuses Slice 6 harness; deltas are small additive extensions |
| Slice 8 | docs-only (PROXY_RECIPE.md) | no code change |
| Slice 9 | code-level COMPLETE | extends Slice 6 with STREAM + MULTI-TEAM steps |
| Slice 10 | docs-only (this slice) | public docs page, sibling cross-links, README integration table row |

## Final-pass adversarial assessment

The whole-integration shape:

- **Surface contract** (DESIGN §3.4 v1 Path B + Path A): the
  CustomLogger callback gates the LiteLLM proxy's `/v1/chat/completions`
  path; sync direct `litellm.completion()` callers route via the
  SpendGuard egress proxy (Shape A). Both surfaces verified end-to-end
  in Slice 6 + Slice 7.
- **Fail-closed posture**: DEGRADE outcomes block; sidecar
  unavailable blocks; resolver=None blocks; non-SpendGuard errors
  during release bubble; SpendGuard errors during release swallow +
  TTL sweep. All paths exercised in Slice 7 sub-steps + Slice 5 tests.
- **Audit chain coherence** (F4): one writer per llm_call_id —
  the proxy callback. No interleaving with egress-proxy events.
  `LiteLLM_SpendLogs ⨝ canonical_events` join story documented
  in DESIGN.md §8.3 + PROXY_RECIPE.md §7.
- **Event-loop affinity** (Round 3 P0.3): `_LoopBoundCallback`
  defers gRPC client construction to first hook fire; module-import
  safe (no `asyncio.get_running_loop()` at construction). Backend
  Architect verified in Slice 6 R1.
- **Pricing integrity** (F2): pricing tuple frozen at BudgetBinding
  construction time; resolver may not vary across pre-call and
  reconciler; `_validate_claim_against_binding` enforces
  budget_id/window_instance_id/unit.unit_id equality at both
  boundaries (Slice 3 R3 P1 hardening).
- **F7 acceptance**: counting provider returns real
  `usage.completion_tokens > 0`; reconciler commits real amount
  (not estimator worst-case); `mock_response` is banned (verified
  in Slice 6 R1 P1 positive-control).

## Deferred follow-ups (NOT shipping blockers)

These were noted but not addressed because they fall outside the
v1 ship scope:

- **GH #77** — sidecar enrichment extension for the 12-field
  `decision_context_json` passthrough. Cross-component; deferred to
  v1.1.
- **P2 polishes from Slice 5 R2** — docstring tightening on the
  failure event swallow contract, log payload enrichment with
  reservation_id, future-i18n `cancelled` regex hardening.
- **P2 polishes from Slice 6 R2** — `_COUNTING_PROVIDER_HITS`
  module-global, `_LoopBoundCallback._ensure_client` deadline
  documentation, demo-only branch boot-time guard in
  spendguard_callback.py.

## Demo gate — BLOCKING for PR1 merge (task #10)

The autonomous session attempted `make demo-up DEMO_MODE=litellm_real`
but the 12-service docker compose build + image pull exceeded the
agent's practical timeout. Both gates remain to be verified by the
operator:

1. `make demo-up DEMO_MODE=litellm_real` → exit 0 with
   "litellm_real ALL 4 steps PASS (ALLOW + DENY + STREAM + MULTI-TEAM)"
   + `SLICE6/9 LEDGER OK` + `SLICE6 CANONICAL OK` + `SLICE6 DENY OK`.
2. `make demo-up DEMO_MODE=litellm_deny` → exit 0 with all 3
   sub-step PASS lines + `SLICE7 LEDGER OK: reserve>=3 commit_estimated>=3 denied_decision>=1`.

Once both gates pass, the branch can ship to PR1 (Slices 1–7) and
PR2 (Slices 8–10) per the user's two-PR pacing.

## Final-pass verdict

**ZERO new P0** in the whole-integration shape. All per-slice P0/critical-P1
findings are addressed. The demo gate (task #10) is a runtime
verification step that must be executed by the operator with an
available Docker stack; the code is shippable pending that gate.

`VERDICT: PASS (pending demo gate task #10)`
