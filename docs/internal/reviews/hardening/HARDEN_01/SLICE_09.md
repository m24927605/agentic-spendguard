# HARDEN_01 Retrospective — SLICE_09 run_cost_projector

- Slice doc: `docs/internal/slices/SLICE_09_run_cost_projector.md`
- Merge commit: `6407648`
- Merge base / first parent: `6adb6f0`
- Topic branch tip / second parent: `355810e`
- Diff command: `git diff 6407648^1..6407648`
- Diff size: 29 files, +7320/-17

## Review Focus

- Signal 1/2/3 layering and code precedence
- Per-run state cache concurrency
- Sidecar projector integration and audit columns
- RUN_* emission shape

## Findings

### Major — Unknown budget remaining was treated as zero

`services/sidecar/src/decision/transaction.rs` parsed `DecisionRequest.inputs.projected_p90_atomic` as the run projector's budget remaining approximation and defaulted missing/invalid values to `0`. Because `run_cost_projector::compute_layering` emits `RUN_BUDGET_PROJECTION_EXCEEDED` when `projection_atomic > budget_remaining_atomic`, any non-trivial projection on a request without a budget snapshot could produce a false budget-projection stop.

Fix: HARDEN_01 changes the missing/invalid/negative budget snapshot path to `i64::MAX`, a non-triggering sentinel, and adds unit tests for unknown, invalid, negative, and valid values. The ledger reserve path remains the authoritative hard budget gate.

## Residual Checks Routed Later

- HARDEN_02 must still verify a true `RUN_BUDGET_PROJECTION_EXCEEDED` path using a real or explicitly injected budget remaining value.
- HARDEN_03/#160 should add a Postgres-backed integration path around run projection and audit row population.

