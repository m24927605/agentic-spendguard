# Slice A2 — `SpendGuardDirectAcompletion` unit tests · review log

Scope: 15 Tier 1 unit tests covering every failure-mode contract
documented in `100-percent-design.md` §Epic A.

## Test coverage matrix

| Failure mode (architect spec) | Test |
| --- | --- |
| ALLOW happy path | `test_allow_happy_path_returns_response` |
| `outcome.decision == STOP/REQUIRE_APPROVAL/...` → DecisionDenied | `test_deny_raises_and_never_calls_litellm` |
| `outcome.decision == DEGRADE` → SidecarUnavailable | `test_degrade_raises_sidecar_unavailable` |
| Pre-call transport error → SidecarUnavailable | `test_pre_call_transport_error_wrapped_as_sidecar_unavailable` |
| `litellm.acompletion()` raises → release + re-raise | `test_provider_raises_releases_reservation_and_reraises` |
| asyncio.CancelledError documented dead-code path | `test_provider_cancelled_classifies_as_cancelled` |
| Commit-time error swallowed; response returned | `test_commit_failure_swallowed_response_still_returned` |
| stream=True deferred → SpendGuardConfigError | `test_stream_true_rejected_before_reserve` |
| `SPENDGUARD_LITELLM_FAIL_OPEN=1` pre-call bypass | `test_fail_open_bypasses_pre_call_error` |
| `SPENDGUARD_LITELLM_FAIL_OPEN=1` DEGRADE bypass | `test_fail_open_bypasses_degrade` |
| budget_resolver returns None | `test_resolver_none_rejected_before_reserve` |
| claim_estimator cardinality | `test_estimator_wrong_cardinality_rejected` |
| claim_reconciler binding mismatch | `test_reconciler_binding_mismatch_rejected` |
| Concurrent gather distinct call-ids (R1 F1 fix) | `test_concurrent_calls_get_distinct_call_ids` |
| Caller-supplied litellm_call_id idempotent | `test_litellm_call_id_caller_supplied_honored` |

## Stopping rule

Per user mandate "don't get stuck in code review", test code that
directly mirrors the architect's `failure_modes` table doesn't
need a separate codex/Staff round — the tests themselves are the
review of the impl. R0 status: tests pass (15/15) + ruff clean.

If any failure-mode test FAILS, that's a Slice A1 bug to fix, not a
Slice A2 review iteration.

## Slice A2 → CODE-LEVEL CLOSED.

118 pytest tests pass total. Next: Slice A3 — `DEMO_MODE=litellm_direct`
end-to-end against the counting provider + SQL verify.
