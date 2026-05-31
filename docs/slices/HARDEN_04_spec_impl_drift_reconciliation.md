# HARDEN 04 — Spec/implementation drift reconciliation

> **Branch**: `harden/HARDEN_04_spec_impl_drift_reconciliation`
> **Status**: draft
> **Spec ancestor(s)**: `predictor-upgrade-hardening-spec-v1alpha1.md`
> **Depends on prior slices**: HARDEN_01 through HARDEN_03
> **Blocks subsequent slices**: HARDEN_05 through HARDEN_08
> **Estimated change size**: small-medium; documentation and grep-backed code/spec audit

---

## §0. TL;DR

Reconcile locked predictor specs with the implementation shipped by SLICE_01-15. This slice is the only hardening slice allowed to edit normative locked spec text, and only to correct proven implementation drift with commit-hash citations.

---

## §1. Architectural context

The spec set locked on 2026-05-30, but some text still describes round-1 drafts instead of shipped code. The goal is not redesign. The goal is to make the locked documents truthful: column names, CloudEvent types, and fail-closed behavior must match production code.

---

## §2. Scope (must-do)

- In `docs/stats-aggregator-spec-v1alpha1.md` §4.1, change `recorded_at` to `ingest_at`
- In `docs/stats-aggregator-spec-v1alpha1.md` §4.1, change `cloudevent_payload` to `payload_json`
- In `docs/stats-aggregator-spec-v1alpha1.md` §7.2, set event type to `spendguard.audit.prediction_drift_alert.v1alpha1`
- In `docs/contract-dsl-spec-v1alpha2.md` §6.1, replace "graceful STOP fallback" wording with the fail-closed behavior implemented by SLICE_02/09/10
- Cross-link every changed spec section to the SLICE_NN commit hash that made the implementation authoritative
- Run a grep audit: every CloudEvent type in specs must match emission code or be explicitly documented as planned/future
- Record the grep commands and results under `docs/reviews/hardening/HARDEN_04/`

---

## §3. Out of scope

| Item | Pushed to |
|---|---|
| New v1alpha2 architecture changes | Future design phase |
| Style-only rewrites of locked specs | Not allowed |
| Changing implementation to match stale text | Only if text was actually correct and code is unsafe |

---

## §4. File-level change list

### 4.1 New files

- `docs/reviews/hardening/HARDEN_04/spec-drift-audit.md`
- `docs/reviews/hardening/HARDEN_04/cloudevent-grep-results.md`

### 4.2 Modified files

- `docs/stats-aggregator-spec-v1alpha1.md`
- `docs/contract-dsl-spec-v1alpha2.md`
- `docs/calibration-report-spec-v1alpha1.md`
- `docs/slices/SLICE_13_calibration_report_cli.md`
- `proto/spendguard/sidecar_adapter/v1/adapter.proto` comment text only
- `services/calibration_report/README.md`
- `services/calibration_report/src/{formatters,report,recommendations,sql_queries}.rs`
- `services/calibration_report/tests/scenarios.rs`
- Any other predictor spec where grep proves a stale CloudEvent type, stale column name, or contradictory failure behavior

---

## §5. Schema / proto changes

No schema or generated proto changes. This slice reconciles documentation to already-shipped schema and proto behavior; proto edits, if any, are comment-only corrections.

---

## §6. Audit-chain impact

The audit-chain impact is documentary but production-critical:

- Spec text must name the actual canonical_events columns `ingest_at` and `payload_json`
- Drift alert event type must preserve the `spendguard.audit.*` prefix so ImmutableAuditLog routing is unambiguous
- Contract DSL failure wording must not imply a permissive fallback where the code fails closed

---

## §7. Failure mode coverage

| Scenario | Expected behavior |
|---|---|
| Spec and implementation disagree | Cite implementation commit and correct spec if implementation matches locked invariant |
| Code appears wrong, not spec | File/fix as blocker in this slice if small, otherwise route to owning HARDEN_NN |
| CloudEvent type appears only in docs | Mark as planned/future or remove if stale |
| Normative edit would alter behavior | Stop; requires new spec version |

---

## §8. Acceptance criteria

### 8.1 Required drift fixes

- stats-aggregator §4.1 uses `ingest_at` and `payload_json`
- stats-aggregator §7.2 uses `spendguard.audit.prediction_drift_alert.v1alpha1`
- contract-dsl §6.1 describes fail-closed reality

### 8.2 Citation hygiene

- Every changed paragraph cites the SLICE_NN merge commit or implementation commit
- No fabricated "per SLICE_NN" citations

### 8.3 Grep audit

- `rg 'recorded_at|cloudevent_payload|prediction_drift|drift_alert|STOP fallback|graceful STOP' docs services crates proto` results are reviewed
- Every spec CloudEvent type has a matching emission site or explicit future marker

### 8.4 Demo-mode regression

- No demo required unless this slice discovers and fixes code. If code is touched, run `make demo-up DEMO_MODE=default`.

---

## §9. Slice-specific adversarial review checklist

1. Are all normative edits tied to concrete implementation commits?
2. Did the slice avoid redesigning locked behavior?
3. Are `ingest_at` and `payload_json` consistent with canonical_ingest migrations?
4. Does `spendguard.audit.prediction_drift_alert.v1alpha1` match the stats_aggregator emission code?
5. Does contract DSL wording describe fail-closed behavior precisely?
6. Did grep cover docs, services, crates, and proto?
7. Are any stale CloudEvent names left in examples or tests?
8. Are adoption-history notes updated without overstating lock criteria?
9. Did the patch avoid unrelated prose churn?
10. Are all citations real file paths or commit hashes?

---

## §10. Out-of-scope deferrals

| Item | Why deferred |
|---|---|
| Full docs site rewrite | This is a correctness patch |
| v1beta1 spec preparation | Future phase |
| Operator-facing migration guide | Only needed if behavior changes; this slice should not change behavior |

---

## §11. Risk / rollback plan

- Risk: a doc edit accidentally changes a normative promise. Mitigation: require commit-backed citations and adversarial review.
- Risk: grep misses generated docs. Mitigation: use `rg --files` and include site docs if they mirror specs.
- Rollback: revert the documentation patch; no runtime state changes.

---

## §12. AIT execution notes

Reviewer: codex CLI via `ait run --adapter codex --review-mode adversarial`. Max 5 rounds. Staff+ panel arbitration if 5 rounds fail.

Reviewer should prioritize citation truthfulness and grep completeness over prose style.

---

## §13. Adoption history

| Round | Reviewer / panelist | Decision | Outcome |
|---|---|---|---|
| Design | Software Architect | Locked specs may be edited only for proven drift | §2 and §7 constrain edits |
| Design | Backend Architect | Fail-closed wording must match sidecar behavior | contract-dsl §6.1 is in scope |
| Design | Security Engineer | Audit-routed CloudEvents must retain `spendguard.audit.*` | stats-aggregator §7.2 is in scope |
| Design | Database Optimizer | Column names must be verified against migrations | stats-aggregator §4.1 is in scope |
| Design | Technical Writer | Avoid broad prose churn | §9 includes minimal-diff review |
| Implementation | codex CLI | Broad grep found calibration-report spec drift too | Added calibration-report spec corrections and grep artifacts |
| AIT R1 | codex CLI adversarial reviewer | Calibration ratio direction and stats bucket key still drifted | Fixed actual/predicted wording, Strategy C threshold/tests, and `prompt_class` bucket-key prose |
| AIT R2 | codex CLI adversarial reviewer | Strategy A critical actual/predicted ratios were hidden by formatter special-case | Threshold checks now precede the Strategy A label; recommendation Rule 1 covers any strategy |
| AIT R3 | codex CLI adversarial reviewer | Strategy C critical under-prediction could render as generic warning | Strategy C under-prediction label now precedes generic warning; text/markdown regressions added |

---

## §14. Merge checklist

- [ ] Required spec drift edits complete
- [ ] Commit-hash citations added
- [ ] CloudEvent and stale-column grep audit recorded
- [ ] No unrelated spec redesign
- [ ] AIT adversarial review passes or Staff+ arbitration is recorded

---

*Slice version: HARDEN_04_spec_impl_drift_reconciliation v1alpha1 | Spec ancestor: predictor-upgrade-hardening-spec-v1alpha1 | Branch: `harden/HARDEN_04_spec_impl_drift_reconciliation`*
