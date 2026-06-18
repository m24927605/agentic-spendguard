# HARDEN_01 Retrospective — SLICE_10 egress_proxy decision rewrite

- Slice doc: `docs/internal/slices/SLICE_10_egress_proxy_decision_rewrite.md`
- Merge commit: `c649196`
- Merge base / first parent: `6407648`
- Topic branch tip / second parent: `32bf36e`
- Diff command: `git diff c649196^1..c649196`
- Diff size: 14 files, +1640/-98

## Review Focus

- Deletion of the production `chars/4 x 2` estimator
- Correct tokenizer/output_predictor/run_cost_projector hot-path integration
- 17 decision-side audit column population
- Failure behavior when predictor/projector services are unavailable

## Findings

### Major — Egress projector call also used budget remaining zero

`services/egress_proxy/src/decision.rs::estimate_call_cost` invokes run_cost_projector in parallel for audit/projection metadata. The request used `budget_remaining_atomic: 0` because egress_proxy has no authoritative budget remaining snapshot. That made the ClaimEstimate's run projection metadata report `RUN_BUDGET_PROJECTION_EXCEEDED` for ordinary first calls if the projector client was configured.

Fix: HARDEN_01 changes the egress-side projector request to use `i64::MAX` when budget remaining is unknown. This prevents false RUN_BUDGET metadata while preserving the real budget enforcement in sidecar/ledger.

## Invariants Verified

- `rg "estimate_tokens"` shows only comments referencing the deleted heuristic and no callable production estimator.
- Tokenizer fallback remains in `spendguard-tokenizer` Tier 3, not a resurrected egress_proxy `chars/4 x 2` path.

## Residual Checks Routed Later

- HARDEN_02 must prove the proxy path boots and writes populated prediction metadata in demo.
- HARDEN_03/#169 must verify sidecar mirror columns are populated from this path.

