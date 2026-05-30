# HARDEN 03 — Production-blocker GitHub issue triage and close

> **Branch**: `harden/HARDEN_03_production_blocker_gh_triage`
> **Status**: draft
> **Spec ancestor(s)**: `predictor-upgrade-hardening-spec-v1alpha1.md`
> **Depends on prior slices**: HARDEN_01, HARDEN_02
> **Blocks subsequent slices**: HARDEN_04 through HARDEN_08
> **Estimated change size**: medium-large; P1 issue closure across SDK, API, audit, manifests, SQL, and tests

---

## §0. TL;DR

List all open issues in `m24927605/agentic-spendguard`, classify #90-#177 into P1/P2/P3, then close the P1 production blockers in-slice. This is not a paperwork pass: each P1 issue receives a real code/test/doc fix, a local verification command, and a `gh issue close` with the fix commit.

---

## §1. Architectural context

SLICE_02 through SLICE_07 intentionally filed residual issues to keep review loops finite. The hardening phase changes the bar: P1 residuals are now production blockers and must close before customer onboarding. This slice is the canonical priority map for the remaining issue backlog.

---

## §2. Scope (must-do)

- Run `gh issue list --repo m24927605/agentic-spendguard --limit 100 --state open`
- Review all open issues #90 through #177
- Classify every open issue as P1/P2/P3 in `docs/reviews/hardening/HARDEN_03/issue-triage.md`
- Close P1 issues in-slice:
  - #90 Python SDK proto regen for STOP_RUN_PROJECTION
  - #137 control plane sampling-rate API persistence
  - #143 verify-chain admission for drift_alert
  - #145 plaintext DB URL in K8s manifests, workspace-wide
  - #150 `pg_indexes` schemaname filter
  - #160 integration tests: cycle_e2e_postgres, RLS injection, audit row population
  - #168 tokenizer sink AppendEventsRequest envelope
  - #169 SLICE_10 sidecar mirror column population verification
  - #171 per-tenant SVID cert minting cross-reference to HARDEN_08
- Close issues through `gh issue close` only after the fix commit is present

---

## §3. Out of scope

| Item | Pushed to |
|---|---|
| P2/P3 issue implementation | Future hardening or enhancement pass |
| Per-tenant SVID full implementation | HARDEN_08; this slice cross-references and updates issue state only when HARDEN_08 lands |
| New issue creation for unrelated findings | Allowed only when not a P1 in this slice |

---

## §4. File-level change list

### 4.1 New files

- `docs/reviews/hardening/HARDEN_03/issue-triage.md`
- `docs/reviews/hardening/HARDEN_03/p1-closure-log.md`

### 4.2 Modified files

- `sdk/python/src/spendguard/_proto/**` and SDK decision mapping files for #90
- `services/control_plane/src/handlers/tokenizer.rs` and control-plane migrations for #137
- `services/canonical_ingest/**` or verify-chain code for #143
- `charts/spendguard/templates/**`, `deploy/**`, and values files for #145 plaintext DB URL removal
- SQL or tests that query `pg_indexes` for #150
- `services/stats_aggregator/**`, `services/output_predictor/**`, and integration tests for #160
- `services/tokenizer/src/shadow/**` sink code for #168
- `services/egress_proxy/**`, `services/sidecar/**`, or mirror verification tests for #169
- `docs/slices/HARDEN_08_per_tenant_svid_cert.md` cross-link and GH issue notes for #171

---

## §5. Schema / proto changes

Expected additive changes:

- Regenerate Python SDK proto stubs after `STOP_RUN_PROJECTION = 6`
- Add control-plane persistence schema if #137 lacks a durable table
- Add verification metadata only if verify-chain drift alert admission needs a stored allowlist

No field renumbering, enum reuse, or backwards-incompatible API changes are allowed.

---

## §6. Audit-chain impact

P1 issue closure must preserve audit invariants:

- #143 ensures tokenizer drift alerts are admitted by verify-chain
- #168 ensures tokenizer shadow sink uses `AppendEventsRequest` with `producer_id`, `schema_bundle`, and `route`
- #169 ensures mirror columns that stats_aggregator depends on are populated and verifiable
- Control-plane persistence for #137 must emit or preserve operator-change audit events

---

## §7. Failure mode coverage

| Scenario | Expected behavior |
|---|---|
| GitHub issue list unavailable | Use local issue references only temporarily; rerun `gh issue list` before merge |
| A P1 overlaps another HARDEN slice | Fix code here when possible; if truly cross-slice, record owner and do not close until owner lands |
| Closing issue without fix commit | Forbidden |
| P2/P3 issue appears security-critical during triage | Upgrade to P1 and fix in this slice |

---

## §8. Acceptance criteria

### 8.1 Triage artifact

- Every open issue #90-#177 is classified P1/P2/P3 with one-sentence rationale
- P1 list is complete and includes any new HARDEN_01/HARDEN_02 blockers

### 8.2 P1 closure

- All P1 issues except explicit HARDEN_08 ownership are fixed and closed
- Each closure comment cites the fixing commit and verification command

### 8.3 Verification gates

- Affected Rust crates build and test
- Affected Python SDK tests pass
- Helm demo and production templates render after manifest fixes
- New integration tests for #160 run locally

### 8.4 Demo-mode regression

- `make demo-up DEMO_MODE=default` runs after P1 fixes, or failure is a HARDEN_02 environment artifact already recorded

---

## §9. Slice-specific adversarial review checklist

1. Is the issue triage complete for all #90-#177 open issues?
2. Were P1 issues closed only after code/tests landed?
3. Does SDK proto regen include `STOP_RUN_PROJECTION` in generated stubs and user-facing mapping?
4. Does sampling-rate persistence survive process restart?
5. Are plaintext DB URLs removed from every Kubernetes manifest surface?
6. Does `pg_indexes` usage filter `schemaname`?
7. Do integration tests exercise Postgres, RLS injection, and audit row population?
8. Does tokenizer shadow sink use the full AppendEventsRequest envelope?
9. Does mirror-column verification prove SLICE_10 write path population?
10. Is #171 left open only if HARDEN_08 has not merged yet?

---

## §10. Out-of-scope deferrals

| Item | Why deferred |
|---|---|
| P2/P3 backlog closure | Not production-blocking |
| #171 full implementation before HARDEN_08 | Dedicated identity slice owns it |
| New dashboard/reporting for issue state | GitHub issue list remains source of truth |

---

## §11. Risk / rollback plan

- Risk: P1 closure touches many surfaces. Mitigation: keep each P1 as an atomic commit with its own verification.
- Risk: closing #171 too early hides identity risk. Mitigation: #171 only closes after HARDEN_08 implementation or remains linked.
- Rollback: revert the specific P1 fix commit and reopen the associated issue with regression details.

---

## §12. AIT execution notes

Reviewer: codex CLI via `ait run --adapter codex --review-mode adversarial`. Max 5 rounds. Staff+ panel arbitration if 5 rounds fail.

Reviewer must read `issue-triage.md`, confirm that P1 issues have real code fixes, and check the GitHub closure log.

---

## §13. Adoption history

| Round | Reviewer / panelist | Decision | Outcome |
|---|---|---|---|
| Design | Software Architect | Triage must cover the whole #90-#177 range | §2 requires full issue list review |
| Design | Backend Architect | P1 closure must be atomic per issue | §11 requires per-P1 fix commits |
| Design | Security Engineer | Plaintext DB URL and SVID work are production blockers | #145 and #171 are P1 |
| Design | Database Optimizer | `pg_indexes` schemaname and RLS tests are required | #150 and #160 are P1 |
| Design | SDK/domain expert | STOP_RUN_PROJECTION SDK regen blocks real users | #90 is P1 |

---

## §14. Merge checklist

- [ ] `gh issue list` output captured
- [ ] `issue-triage.md` classifies all open #90-#177 issues
- [ ] P1 fixes committed and verified
- [ ] P1 issues closed or explicitly linked to HARDEN_08
- [ ] Affected tests and Helm templates pass
- [ ] AIT adversarial review passes or Staff+ arbitration is recorded

---

*Slice version: HARDEN_03_production_blocker_gh_triage v1alpha1 | Spec ancestor: predictor-upgrade-hardening-spec-v1alpha1 | Branch: `harden/HARDEN_03_production_blocker_gh_triage`*
