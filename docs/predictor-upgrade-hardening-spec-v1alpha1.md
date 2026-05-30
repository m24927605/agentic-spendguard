# Predictor Upgrade Hardening Spec v1alpha1

> **Status**: draft (2026-05-31)
> **Phase**: post-SLICE_15 production hardening
> **Spec ancestors**: `predictor-architecture-spec-v1alpha1.md` (umbrella; LOCKED 2026-05-30), and the 9 sub-specs locked at the same time
> **Drives**: `docs/slices/HARDEN_01_*.md` through `docs/slices/HARDEN_08_*.md`
> **Owner**: Staff+ design panel (Software Architect lead; codex CLI as reviewer)

---

## §0. TL;DR

The predictor upgrade spec set LOCKED at SLICE_15 merge (2026-05-30) with all 15 slices on main. A 2026-05-31 maintainer audit identified that the codebase is ~75% production-ready, not 100%. The HARDEN phase ships 8 additive hardening slices (HARDEN_01–HARDEN_08) that close the remaining gap WITHOUT reopening any locked spec invariant. This document is the umbrella spec for that effort: it defines the gap, the cross-slice invariants, the slice dependency order, and the locked design decisions for the hardening cadence.

---

## §1. Why this spec exists (the 25% gap)

The 2026-05-31 honest gap list — produced by the maintainer after the spec-set lock — surfaced four classes of residual risk:

1. **Adversarial-review skipped on 8 slices**: SLICE_08–SLICE_15 batch-shipped on implementer self-validation only (per the user's notify-when-done directive). The 4-reviewer R1 panel was not run; Blockers and Majors that the panel would have caught are now latent in main.
2. **E2E never actually ran end-to-end**: `make demo-up` modes were exercised piecewise during slice review, but no single audit ran the 5+ demo modes back-to-back on real docker, and the kind cluster + Helm + chaos NetworkPolicy path was never executed against the SLICE_11 chart.
3. **81 deferred GH issues**: across SLICE_02–SLICE_07, 81 residual issues were filed with target-slice annotations. ~10 of them are production blockers (verify-chain admission, plaintext DB URL, SVID per-tenant cert, etc.) that must close before any external customer is onboarded.
4. **Spec/impl drift accumulated**: stats-aggregator-spec §4.1 still references the round-1 column names `recorded_at` / `cloudevent_payload`; the actual canonical_events schema uses `ingest_at` / `payload_json`. contract-dsl-spec-v1alpha2 §6.1 still uses round-1 wording "graceful STOP fallback" that the implementation no longer matches.

The HARDEN phase is strictly additive: no locked spec invariant is reopened; every change either (a) closes a gap that the spec already implied, (b) reconciles spec text with the shipping implementation, or (c) hardens a deferred residual that the slice itself acknowledged.

---

## §2. Cross-slice invariants

### 2.1 Codex CLI as canonical reviewer

Per `feedback_hardening_workflow.md` (2026-05-31 directive), the canonical adversarial reviewer for every HARDEN slice is **codex CLI dispatched by AIT**. The `claude-code` adapter is not used for this phase. Rationale:

- The honest-gap directive was issued by a maintainer who wants a single adversarial gate, not a 4-reviewer panel.
- Codex CLI's adversarial mode is the independent second opinion already proven during SLICE_02–SLICE_07 cross-checks.
- One reviewer reduces per-slice cycle cost from 50–300k tokens to 10–80k tokens, which matters when shipping 8 slices.

The concrete invocation is:

```bash
ait run \
  --adapter codex \
  --review-mode adversarial \
  --base main \
  --branch harden/HARDEN_NN_<name> \
  --slice-doc docs/slices/HARDEN_NN_*.md \
  --review-budget deep
```

Each HARDEN slice's §12 (AIT execution notes) MUST name codex CLI via this AIT invocation and MUST NOT name the claude-code adapter.

### 2.2 Max 5 codex review rounds per slice

Identical to SLICE_01–SLICE_07 round budget. Each round:

1. Codex review produces a finding list (Blocker / Major / Minor).
2. Implementer fixes ALL findings — not just Blockers. Per `predictor-review-checklist.md` §4 round-pass rule, severity is for triage only; every finding gates the round.
3. Re-review.
4. If the new finding list is empty → round passes.

If round 5 still has findings after the implementer's fix attempt, the slice escalates to Staff+ panel arbitration per §2.3.

### 2.3 Staff+ panel arbitration on round-5 fail

Identical process to `staff-panel-arbitration-process.md`, but dispatched through separate `ait run --adapter codex --review-mode adversarial` invocations. The panel composition for HARDEN slices is:

- Software Architect (always)
- Backend Architect (always)
- Security Engineer (always)
- Database Optimizer (always)
- Domain expert (varies by slice; see each slice's §12)

The arbitration ruling is committed to `docs/arbitrations/HARDEN_NN-ruling-YYYY-MM-DD.md` and linked from the merge commit. Per `feedback_hardening_workflow.md` step 5, **arbitration must not block** — the panel ships a decision (merge / block / rework), it does not punt.

### 2.4 No GitHub PR for HARDEN slices

Per `feedback_no_github_pr_for_ai_workflows.md` (strengthened 2026-05-31): HARDEN slices merge via direct branch push + memory record. No PR, no review request. Justification:

- HARDEN is a pure AI-agent workflow with codex CLI as the gate.
- GitHub PR ceremony added 1–2 days per SLICE during the predictor upgrade with no quality lift.
- Memory + commits + arbitration rulings (when convened) are the durable record.

The single exception: if a HARDEN slice modifies a file under upstream review (e.g., the Microsoft AGT integration), a PR may be opened against that upstream repo as already separate from the HARDEN merge.

### 2.5 Residual tracking (GH issues vs in-slice fix)

Per `feedback_codex_iteration_pattern.md` stopping rule: codex finding fixed in-slice, NOT tracked as gap-bullet. The ONLY items tracked as GH issues during HARDEN are:

- Findings that the slice itself declares out-of-scope (cross-slice prereqs).
- Findings genuinely outside the predictor-upgrade scope (e.g., a security finding about an unrelated service the slice incidentally touches).

A HARDEN slice that ships with > 3 GH-tracked residuals is a yellow flag: the slice was probably too big and should have been split. > 5 is a red flag: the slice MUST be re-scoped before merge.

### 2.6 No locked spec edits

The 10 predictor-upgrade specs are LOCKED per `predictor-architecture-spec-v1alpha1.md` §0.2 (2026-05-30). HARDEN slices MAY:

- Append to a spec's `§9. Adoption history` table.
- Append to a spec's `§N. Round-NN notes` subsection (the round-of-record append-only convention).
- Add a HARDEN-NN cross-link in a spec's "Related" footer.

HARDEN slices MAY NOT:

- Edit a spec's normative §N body text.
- Change a spec's locked invariant.
- Reopen a spec's `§0.2 lock criteria`.

Drift reconciliation (HARDEN_04) is the explicit exception and runs under a separate ruleset (§3.4).

---

## §3. Slice dependency order

### 3.1 The 8 HARDEN slices

| Slice | Scope | Reviewer | Domain expert |
|---|---|---|---|
| HARDEN_01 | SLICE_08–15 retrospective adversarial review | codex CLI | Backend Architect |
| HARDEN_02 | E2E real-cluster validation (docker + kind + Helm) | codex CLI | Performance Benchmarker |
| HARDEN_03 | 81 GH issues triage + P1 close | codex CLI | Backend Architect |
| HARDEN_04 | Spec/impl drift reconciliation | codex CLI | Technical Writer |
| HARDEN_05 | Security hardening backlog (replay protection, PII opt-in, cool-down cap, rustls provider, drop tonic gzip) | codex CLI | Security Engineer |
| HARDEN_06 | SLICE_05/06 leftover (tokenizer sink envelope, Ed25519 wire) | codex CLI | Backend Architect |
| HARDEN_07 | Cargo/helm/migration verification | codex CLI | Database Optimizer |
| HARDEN_08 | Per-tenant SVID cert minting | codex CLI | Security Engineer |

### 3.2 Ordering rationale

The maintainer directive for this phase is strict: slices run sequentially in numeric order, without stopping between slices unless a slice reaches the round-5 Staff+ arbitration path.

```
HARDEN_01 → HARDEN_02 → HARDEN_03 → HARDEN_04 → HARDEN_05 → HARDEN_06 → HARDEN_07 → HARDEN_08
```

Reasoning:

1. **HARDEN_01 first** because SLICE_08–SLICE_15 missed adversarial review and may contain hot-path regressions, especially in SLICE_10.
2. **HARDEN_02 second** because real demo and cluster failures must be observed early; later slices then close concrete failures rather than theoretical ones.
3. **HARDEN_03 third** because GH P1 triage can incorporate HARDEN_01/HARDEN_02 findings and close production blockers in one pass.
4. **HARDEN_04 fourth** because the specs should be reconciled after the first three discovery-heavy slices reveal the complete drift set.
5. **HARDEN_05 fifth** because the security backlog can rely on reconciled spec text and closed P1 issue taxonomy.
6. **HARDEN_06 sixth** because SLICE_05/06 leftover CloudEvent and signing work should land after the replay/PII/rate-limit model is hardened.
7. **HARDEN_07 seventh** because cargo, Helm, migration, and NetworkPolicy verification should run after the security and signing changes have landed.
8. **HARDEN_08 eighth** because per-tenant SVID cert minting is the final identity hardening layer and must pass the verification gates introduced by HARDEN_07.

### 3.3 Parallelism

No implementation parallelism is used in this maintainer-directed hardening run. Parallelism is allowed only inside a slice for read-only inspection, test execution, or independent Staff+ arbitration reviews. Merge order remains HARDEN_01 through HARDEN_08.

### 3.4 HARDEN_04 spec-edit exception

The "no locked spec edits" invariant (§2.6) has one carved-out exception: HARDEN_04 itself. HARDEN_04 MAY edit:

- Stale column-name references in spec §N body text (e.g., `recorded_at` → `ingest_at` in stats-aggregator-spec §4.1).
- Stale CloudEvent type names (e.g., `prediction.drift_alert` → `spendguard.audit.prediction_drift_alert.v1alpha1` in stats-aggregator-spec §7.2).
- Round-1 wording that contradicts shipped behavior (e.g., contract-dsl-spec-v1alpha2 §6.1 "graceful STOP fallback").

The edit MUST be in-place + flagged with a Round-N comment annotation per the existing convention. Every edit MUST cite the SLICE_NN commit hash where the implementation diverged. HARDEN_04 may NOT change a spec's locked invariant under cover of "drift".

---

## §4. Hardening phase locked design decisions

These decisions are LOCKED for the duration of the HARDEN phase and require a v1alpha2 of this spec to revise.

### 4.1 LD-H-01: codex CLI is the canonical reviewer

Stated in §2.1. The `claude-code` adapter is never used during HARDEN. Staff+ arbitration, when required, is run as separate AIT codex invocations for each panel role.

### 4.2 LD-H-02: 5-round budget + arbitration escalation

Stated in §2.2 + §2.3. No HARDEN slice exceeds 5 codex rounds; round 5 → arbitration.

### 4.3 LD-H-03: in-slice fix, not gap-bullet

Stated in §2.5. Codex findings are genuinely fixed in-slice. The only GH-tracked residuals are explicit cross-slice prereqs.

### 4.4 LD-H-04: no PR for HARDEN merges

Stated in §2.4. Direct branch merge to main + memory record.

### 4.5 LD-H-05: locked specs are append-only (HARDEN_04 carved out)

Stated in §2.6 + §3.4. Adoption-history + round-N notes append-only; no normative body edits except HARDEN_04 drift fixes.

### 4.6 LD-H-06: E2E demo is the final gate

Per `feedback_demo_quality_gate.md` (strengthened): SLICE_08–SLICE_15 implementer self-validation is not sufficient. HARDEN_02 actually boots real services in production main.rs and runs 5+ demo modes back-to-back. HARDEN_02 is the merge gate for the whole HARDEN phase — if HARDEN_02 fails, the failure routes back to the relevant HARDEN_NN as a finding, not to a post-HARDEN slice.

### 4.7 LD-H-07: P1 vs P2 vs P3 priority schema

Per HARDEN_03 design:
- **P1**: production blocker. Closes in-slice during HARDEN_03 (or its assigned HARDEN_NN slice).
- **P2**: quality / hardening gap. Documented in a HARDEN_NN slice's §10 deferral table; closure NOT required during HARDEN.
- **P3**: enhancement / nice-to-have. Marked as `enhancement` label on the GH issue; no slice owns it.

The honest-gap list's 10 production blockers (#90 / #137 / #143 / #145 / #150 / #160 / #168 / #169 / #171 + 1 floating SLICE_05 cleanup) are all P1.

### 4.8 LD-H-08: no new specs during HARDEN

The HARDEN phase ships slice docs (`HARDEN_NN_*.md`) and ONE umbrella spec (this file). No new normative specs are introduced. Rationale: HARDEN is a debt-paydown phase, not a design phase. New design work waits for v1alpha2 of the predictor architecture spec set.

---

## §5. Acceptance criteria for the HARDEN phase

The HARDEN phase is complete when ALL of the following hold:

1. All 8 HARDEN slices merged to main with codex-approved or panel-arbitrated record.
2. The 10 P1 GH issues are closed (#90, #137, #143, #145, #150, #160, #168, #169, #171, + the SLICE_05 floating cleanup).
3. HARDEN_02 E2E real-cluster validation: 5+ demo modes pass end-to-end on real docker; helm install green on kind.
4. Spec/impl drift audit (HARDEN_04 final pass): zero stale column references; zero stale CloudEvent type references; zero contradictory wording.
5. All 14 services + crates have consistent Cargo.lock under `cargo update --workspace --dry-run` (no drift).
6. NetworkPolicy enforce test green on kind 1.24+.
7. Per-tenant SVID cert can be minted + rotated end-to-end (HARDEN_08).
8. README + ARCHITECTURE.md + CHANGELOG updated to reflect HARDEN-phase outcomes.

When (1)–(8) are green, this spec is LOCKED and the next phase is v1alpha2 design (out of scope here).

---

## §6. Out-of-scope deferrals

| Item | Why not in HARDEN | Pushed to |
|---|---|---|
| v1alpha2 predictor spec | Design phase, not hardening phase | Future |
| New service introduction | Strictly additive hardening only | Future |
| Customer onboarding tooling | UX/marketing, not hardening | Phase 6 |
| Multi-region failover | Architecture upgrade, not hardening | Phase 2+ |
| Continuous-learning pillar | Explicitly excluded per project_three_pillars decision | Never (per decision) |

---

## §7. Risk / rollback plan

- **Risk: HARDEN_01 retrospective surfaces a Blocker that requires a SLICE_08–SLICE_15 revert.** Mitigation: HARDEN_01 fixes the Blocker in-slice on `harden/HARDEN_01_*` branch; main is not reverted unless the Blocker is so deep that an HARDEN_NN fix cannot land safely. If revert IS required, the revert lands on its own slice (HARDEN_01_REVERT_SLICE_NN) and the corresponding SLICE_NN re-runs through HARDEN_01.
- **Risk: HARDEN_02 E2E reveals a wire mismatch that pre-dates HARDEN.** Mitigation: route the wire mismatch back as a finding to the originating HARDEN_NN (typically HARDEN_06). If the mismatch pre-dates ALL of HARDEN, file as `enhancement` and ship.
- **Risk: codex CLI persistently disagrees with implementer across multiple HARDEN slices.** Mitigation: the staff-panel arbitration is FINAL per `staff-panel-arbitration-process.md`. If 3+ HARDEN slices hit Staff+ arbitration, pause cadence per `staff-panel-arbitration-process.md` §7 (24h cooling-off + retrospective).
- **Rollback per slice**: each HARDEN_NN has its own §11 rollback plan. The hardening phase as a whole is additive; the only "rollback to pre-HARDEN" path is reverting individual HARDEN_NN merges in reverse order (HARDEN_08 → HARDEN_07 → … → HARDEN_01).

---

## §8. Adoption history (filled during hardening)

| HARDEN | Reviewer | Codex rounds | Arbitration? | Outcome |
|---|---|---|---|---|
| HARDEN_01 | (placeholder) | (placeholder) | (placeholder) | (placeholder) |
| HARDEN_02 | (placeholder) | (placeholder) | (placeholder) | (placeholder) |
| HARDEN_03 | (placeholder) | (placeholder) | (placeholder) | (placeholder) |
| HARDEN_04 | (placeholder) | (placeholder) | (placeholder) | (placeholder) |
| HARDEN_05 | (placeholder) | (placeholder) | (placeholder) | (placeholder) |
| HARDEN_06 | (placeholder) | (placeholder) | (placeholder) | (placeholder) |
| HARDEN_07 | (placeholder) | (placeholder) | (placeholder) | (placeholder) |
| HARDEN_08 | (placeholder) | (placeholder) | (placeholder) | (placeholder) |

---

*Spec version: predictor-upgrade-hardening-spec v1alpha1 (draft 2026-05-31) | Drives: HARDEN_01–HARDEN_08 slice docs | Locked-in design decisions: §4 | Reviewer: codex CLI via AIT | Panel-arbitration backstop: Staff+ codex panel via AIT | Branch: `design/predictor-upgrade-hardening`*
