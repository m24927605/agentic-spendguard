# HARDEN 01 — SLICE_08-15 retrospective adversarial review

> **Branch**: `harden/HARDEN_01_slice_08_15_retrospective_review`
> **Status**: draft
> **Spec ancestor(s)**: `predictor-upgrade-hardening-spec-v1alpha1.md`, `predictor-architecture-spec-v1alpha1.md`
> **Depends on prior slices**: SLICE_08 through SLICE_15 merged to main (`6adb6f0` through `8908f9e`)
> **Blocks subsequent slices**: HARDEN_02 through HARDEN_08
> **Estimated change size**: variable; review artifacts plus targeted fixes only

---

## §0. TL;DR

Run the adversarial review that SLICE_08 through SLICE_15 skipped. For each shipped merge, inspect the actual diff against its pre-merge base, compare against the slice doc §8 acceptance criteria and §9 review checklist, then fix every Blocker/Major/Minor discovered in this hardening slice. SLICE_10 is the critical path because it deleted the legacy `chars/4 × 2` estimator and activated the new hot path.

---

## §1. Architectural context

The predictor spec set locked after SLICE_15, but SLICE_08 through SLICE_15 were batch-shipped with implementer self-validation only. This slice restores the review standard used for SLICE_01 through SLICE_07 without reopening locked architecture decisions. The audit focuses on production-readiness drift: hot-path correctness, audit-chain population, SDK compatibility, provider routing, benchmark reproducibility, and demo truthfulness.

---

## §2. Scope (must-do)

- Review SLICE_08 merge `6adb6f0` against `docs/slices/SLICE_08_cold_start_baseline_table.md`
- Review SLICE_09 merge `6407648` against `docs/slices/SLICE_09_run_cost_projector.md`
- Review SLICE_10 merge `c649196` against `docs/slices/SLICE_10_egress_proxy_decision_rewrite.md`
- Review SLICE_11 merge `ab8f4b1` against `docs/slices/SLICE_11_multi_provider_routing.md`
- Review SLICE_12 merge `019c62f` against `docs/slices/SLICE_12_sdk_default_estimators.md`
- Review SLICE_13 merge `83466fa` against `docs/slices/SLICE_13_calibration_report_cli.md`
- Review SLICE_14 merge `10af232` against `docs/slices/SLICE_14_customer_template_contrib.md`
- Review SLICE_15 merge `8908f9e` against `docs/slices/SLICE_15_end_to_end_benchmark.md`
- For each slice, run `git diff <merge_base>..<merge_commit>` and record review notes under `docs/reviews/hardening/HARDEN_01/`
- Fix every finding in this branch unless the finding is an explicit cross-slice prerequisite already owned by HARDEN_02-08

---

## §3. Out of scope

| Item | Pushed to |
|---|---|
| Real docker/kind execution | HARDEN_02 |
| GH issue priority and closure workflow | HARDEN_03 |
| Normative spec body edits | HARDEN_04 |
| New security controls not discovered in retrospective review | HARDEN_05 |
| Per-tenant SVID cert minting | HARDEN_08 |

---

## §4. File-level change list

### 4.1 New files

- `docs/reviews/hardening/HARDEN_01/SLICE_08.md`
- `docs/reviews/hardening/HARDEN_01/SLICE_09.md`
- `docs/reviews/hardening/HARDEN_01/SLICE_10.md`
- `docs/reviews/hardening/HARDEN_01/SLICE_11.md`
- `docs/reviews/hardening/HARDEN_01/SLICE_12.md`
- `docs/reviews/hardening/HARDEN_01/SLICE_13.md`
- `docs/reviews/hardening/HARDEN_01/SLICE_14.md`
- `docs/reviews/hardening/HARDEN_01/SLICE_15.md`

### 4.2 Modified files

- `services/output_predictor/**` if SLICE_08 L2 fallback review finds loader or confidence defects
- `services/run_cost_projector/**` if SLICE_09 review finds run-state, precedence, or audit defects
- `services/egress_proxy/**` and `services/sidecar/**` if SLICE_10 review finds hot-path or audit-column defects
- `services/egress_proxy/src/routing.rs`, provider modules, and `charts/spendguard/templates/networkpolicy.yaml` if SLICE_11 review finds routing or bypass-resistance defects
- `sdk/python/**` if SLICE_12 review finds proto, estimator, packaging, or decorator defects
- `services/calibration_report/**` if SLICE_13 review finds proof-mode, SQL, or tenant-auth defects
- `contrib/output_predictor_template/**` if SLICE_14 review finds conformance or template defects
- `tests/e2e/**`, `benchmarks/predictor-upgrade/**`, and `.github/workflows/predictor-benchmark.yml` if SLICE_15 review finds benchmark or verification defects

---

## §5. Schema / proto changes

No planned schema or proto changes. If retrospective review discovers a wire defect, the fix must be additive, backwards compatible, and documented in the affected proto comments. Python SDK generated protos are in-scope if SLICE_12 or issue #90 is implicated.

---

## §6. Audit-chain impact

The review must verify that all audit-chain promises made by SLICE_08 through SLICE_15 are real:

- SLICE_10 populates the 17 decision-side prediction columns on production hot-path rows
- commit-side actual token columns remain populated where the commit-estimated flow runs
- CloudEvent types emitted by SLICE_09, SLICE_13, and provider paths use the `spendguard.audit.*` prefix when the event is intended for ImmutableAuditLog
- `verify-chain --check-prediction-mirror` has a real code path, not only documentation

---

## §7. Failure mode coverage

| Scenario | Expected behavior |
|---|---|
| A skipped slice has an unreviewed Blocker | Fix in HARDEN_01 before merge |
| A finding requires real cluster execution | Record it as a HARDEN_02 acceptance item and keep the code fix in the owning slice |
| A finding is caused by locked spec drift | Record it for HARDEN_04 unless it is blocking production behavior now |
| SLICE_10 replacement under-reserves | Fix immediately; Strategy A reservation invariant takes priority over compatibility |
| Benchmark artifact cannot be reproduced | Treat as Major; either fix the harness or mark the result as non-authoritative |

---

## §8. Acceptance criteria

### 8.1 Retrospective review artifacts

- Eight review files exist under `docs/reviews/hardening/HARDEN_01/`
- Each file cites the merge base, merge commit, slice doc, and exact diff command used
- Each finding is classified Blocker/Major/Minor and mapped to a fix commit or explicit HARDEN_NN owner

### 8.2 Code fixes

- Every in-scope finding is fixed in this branch
- Hot-path invariants are grep-verified: no resurrected `chars/4 × 2` production estimator; no placeholder drop-handle worker where production config requires a real worker

### 8.3 Regression gates

- `cargo build` and affected `cargo test` suites pass
- `helm template charts/spendguard --set chart.profile=demo` passes
- `helm template charts/spendguard --set chart.profile=production` passes with required production values

### 8.4 Demo-mode regression

- `make demo-up DEMO_MODE=default` runs unless HARDEN_02 owns an environment-level failure; any failure is captured as a concrete finding with logs

---

## §9. Slice-specific adversarial review checklist

1. Does SLICE_10 fully delete or bypass the old `services/egress_proxy/src/decision.rs:277-295` heuristic?
2. Does every production audit row after SLICE_10 carry tokenizer, predictor, strategy, confidence, and run projection metadata?
3. Does SLICE_09 emit RUN_* codes with correct precedence and fail-safe reservation behavior?
4. Does SLICE_11 route OpenAI, Anthropic, Bedrock, Vertex, and Azure without tokenizer/provider mismatch?
5. Does SLICE_12 regenerate Python protos and preserve caller-supplied estimator compatibility?
6. Does SLICE_13's canonical proof mode verify real canonical events instead of cache-only data?
7. Does SLICE_14's customer template fail safely and clearly communicate that the model stub is not production-ready?
8. Does SLICE_15's benchmark measure p99 correctly and avoid averaging tail latency?
9. Are all "per SLICE_NN" citations backed by real file paths and commit hashes?
10. Are all container and Helm additions still on the SLICE_03 security baseline?

---

## §10. Out-of-scope deferrals

| Item | Why deferred |
|---|---|
| Docker/kind install and execution flake handling | HARDEN_02 owns environment bring-up |
| Closing GH issues unrelated to SLICE_08-15 review | HARDEN_03 owns issue closure |
| New SVID identity architecture | HARDEN_08 owns cert minting |

---

## §11. Risk / rollback plan

- Risk: retrospective review opens a large cross-cutting fix that destabilizes the hot path. Mitigation: keep fixes atomic and prefer surgical invariants over refactors.
- Risk: SLICE_10 findings require rollback of the decision rewrite. Mitigation: patch the rewrite unless it demonstrably cannot preserve reservation safety; only then consider reverting the specific merge.
- Rollback: revert the HARDEN_01 merge; individual review artifacts are documentation-only and safe to retain if needed.

---

## §12. AIT execution notes

Reviewer: codex CLI via `ait run --adapter codex --review-mode adversarial`. Max 5 rounds. Staff+ panel arbitration if 5 rounds fail.

Use one review dispatch for the HARDEN_01 branch after implementation. The review prompt must include the eight SLICE_08-15 review artifacts, the diff from main to `harden/HARDEN_01_slice_08_15_retrospective_review`, and the high-risk SLICE_10 hot-path invariant.

---

## §13. Adoption history

| Round | Reviewer / panelist | Decision | Outcome |
|---|---|---|---|
| Design | Software Architect | Retrospective must compare actual merge diffs, not just final files | §2 requires `git diff <merge_base>..<merge_commit>` per slice |
| Design | Backend Architect | SLICE_10 is the highest-risk review target | §8 and §9 make the hot-path heuristic deletion a gate |
| Design | Security Engineer | Container baseline and audit routing must be rechecked for skipped slices | §9 includes security baseline and `spendguard.audit.*` checks |
| Design | Database Optimizer | Audit mirror population must be verified from write path to canonical events | §6 and §8 require verify-chain and mirror checks |
| Design | Predictor domain expert | Benchmark claims are only valid if reproducible | §8 and §9 require benchmark artifact review |

---

## §14. Merge checklist

- [ ] Eight retrospective review files committed
- [ ] Every finding fixed or explicitly mapped to a later HARDEN_NN prerequisite
- [ ] `cargo build` and affected `cargo test` suites pass
- [ ] Demo and production Helm templates render clean
- [ ] `make demo-up DEMO_MODE=default` attempted and result recorded
- [ ] AIT adversarial review passes or Staff+ arbitration is recorded

---

*Slice version: HARDEN_01_slice_08_15_retrospective_review v1alpha1 | Spec ancestor: predictor-upgrade-hardening-spec-v1alpha1 | Branch: `harden/HARDEN_01_slice_08_15_retrospective_review`*
