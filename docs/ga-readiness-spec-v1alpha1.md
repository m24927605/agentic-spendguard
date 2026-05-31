# GA Readiness Spec v1alpha1

> **Status**: draft
> **Phase**: post-HARDEN operationalization
> **Base**: `main` at `38fdab1` after HARDEN_08
> **Drives**: `docs/slices/GA_01_*.md` through `docs/slices/GA_10_*.md`
> **Owner**: Staff+ readiness panel; codex CLI implementer and adversarial reviewer

---

## §0. Executive Summary

HARDEN_01 through HARDEN_08 brought the predictor upgrade to the internal production-ready code threshold. This GA Readiness phase converts that hardened codebase into an externally operable product: release packaging, production deployment guides, observability, runbooks, soak/load evidence, plugin onboarding, security signoff, and backlog triage.

This phase is docs-first. The design documents and slice implementation documents land before production code changes. Implementation then proceeds slice-by-slice. Every slice must pass local acceptance gates, codex CLI adversarial review through AIT, and merge/push/memory recording before the next slice starts.

---

## §1. Preconditions

- Predictor upgrade SLICE_01 through SLICE_15 are merged.
- HARDEN_01 through HARDEN_08 are merged.
- `origin/main` is at or after `38fdab1`.
- P1 production blockers #90, #137, #143, #145, #150, #160, #168, #169, and #171 are closed.
- No GitHub PRs are opened for all-AI workflows; direct branch merge plus memory is the durable record.

---

## §2. Goals

1. Produce release artifacts that map deterministically to a main commit.
2. Provide operator-grade production Helm values, migration playbooks, rollback playbooks, and release notes.
3. Ship dashboards, alert rules, runbooks, and incident drills backed by real emitted metrics.
4. Run a long-duration soak harness with evidence, not just unit tests.
5. Prove scale and performance with production-like tenant/run/provider cardinality.
6. Package customer plugin onboarding and certification, including SVID and conformance checks.
7. Complete independent security signoff, supply-chain evidence, and non-P1 backlog triage.

---

## §3. Non-Goals

- Reopening locked predictor architecture invariants.
- Introducing a new major service unless a slice's Staff+ arbitration explicitly requires it.
- Building a marketing website or sales collateral.
- Claiming managed SaaS multi-region readiness; this phase targets production-grade deployability of the current product.
- Deferring codex review findings as GitHub issues unless they are explicit cross-slice prerequisites.

---

## §4. Slice Inventory

| Slice | Focus | Primary readiness gap |
|---|---|---|
| GA_01 | Release packaging | release/deployment packaging |
| GA_02 | Versioning, changelog, release notes | release/deployment packaging |
| GA_03 | Production Helm values | release/deployment packaging |
| GA_04 | Migration and rollback playbooks | release/deployment packaging |
| GA_05 | Observability dashboards and metric inventory | operational readiness |
| GA_06 | Alerts, runbooks, and incident drills | operational readiness |
| GA_07 | Long-running soak harness | long-running soak |
| GA_08 | Scale/performance SLO proof | scale/performance proof |
| GA_09 | Security signoff and supply chain | security signoff |
| GA_10 | Customer plugin onboarding and backlog triage | plugin onboarding + non-P1 backlog |

The seven readiness gaps are intentionally split into ten implementation slices so each slice can be reviewed to a high standard without becoming a mixed-scope audit.

---

## §5. Required Workflow

Every implementation slice follows:

1. `git checkout -b ga/GA_NN_<name> main`
2. Implement the slice doc's file-level changes in small atomic commits.
3. Run all acceptance gates in the slice doc.
4. Run adversarial review:

```bash
ait run \
  --adapter codex \
  --review-mode adversarial \
  --base main \
  --branch ga/GA_NN_<name> \
  --slice-doc docs/slices/GA_NN_<name>.md \
  --review-budget deep
```

5. Fix every Blocker, Major, and Minor in-slice.
6. Repeat review up to 5 rounds.
7. If round 5 still has findings, run Staff+ arbitration and follow its final decision.
8. Merge to main with `--no-ff`, push `origin/main`, and write memory.

---

## §6. Acceptance Gate Matrix

| Gate | Required when |
|---|---|
| `cargo build` and affected `cargo test` | Rust service or crate touched |
| Python tests | SDK, plugin template, scripts, or docs validation Python touched |
| `helm template charts/spendguard --set chart.profile=demo` | Any chart, deployment, or operator doc touched |
| Production Helm render with required values | Any chart, release, production values, or security config touched |
| Postgres 16 migration verification | Any SQL, migration doc, rollback doc, or DB-facing code touched |
| `make demo-up DEMO_MODE=<relevant>` | Any runtime flow, runbook, demo, or acceptance proof touched |
| `verify-chain` or audit DB probe | Any audit chain, outbox, canonical ingest, signing, replay, or plugin registration path touched |
| Evidence under `docs/reviews/ga-readiness/<slice>/` | Any benchmark, soak, load, or operational drill claim |

No slice may claim a gate passed without running the command or recording why the gate does not apply.

---

## §7. Evidence Standard

Each GA evidence bundle must include:

- commit SHA
- branch name
- date in `YYYY-MM-DD`
- command line
- environment profile
- machine or cluster descriptor
- pass/fail result
- JSON output when the script can produce structured output
- Markdown summary for human review

Performance evidence must not average away p99 or lag tail behavior. Soak evidence must include periodic snapshots, not only final status.

---

## §8. Operational SLO Contract

This phase extends the existing L1-L9 operations surface with predictor-specific SLO evidence:

- tokenizer request p99
- output predictor p99
- run cost projector p99
- canonical ingest append p99
- audit outbox p99 lag
- stats aggregation freshness
- replay dedup rejection rate
- plugin SVID validation failure rate
- prediction drift alert rate and dedup behavior

Every dashboard or alert must reference an actually emitted metric or be marked non-GA before merge.

---

## §9. Security Baseline

GA readiness preserves all HARDEN invariants:

- container `USER 65532`
- `readOnlyRootFilesystem: true`
- `allowPrivilegeEscalation: false`
- `capabilities.drop: [ALL]`
- DB URLs only through Kubernetes Secrets
- RLS policies use the established writer `set_config` pattern, not `BYPASSRLS`
- CloudEvents use the `spendguard.audit.*` prefix when routed to ImmutableAuditLog
- `AppendEventsRequest` requires producer_id, schema_bundle, and route
- tenant IDs remain `uuid::Uuid` in runtime boundaries
- Strategy C plugin identity uses exact per-tenant SVID URI SAN validation

---

## §10. Staff+ Panel Roles

The design panel for this phase is:

- Software Architect: phase decomposition, invariants, rollback boundaries
- Release Engineering Architect: release artifacts, tags, changelog, packaging
- SRE/Operations Architect: metrics, dashboards, alerts, runbooks, drills
- Performance/Database Architect: load, soak, index plans, connection pools, retention
- Security Engineer: threat model, SVID, secrets, RLS, supply chain
- Customer Plugin/Backend Architect: onboarding, conformance, error taxonomy, backlog triage

Every slice records who decided what in its adoption history.

---

## §11. Completion Criteria

This phase is complete only when:

1. All GA_01 through GA_10 slice docs exist and are design-complete.
2. All GA_01 through GA_10 implementation branches are merged to main.
3. Every slice has codex CLI adversarial review closure or Staff+ arbitration recorded.
4. Release packaging, production Helm, migration/rollback, dashboards, alerts, soak, load, security, onboarding, and backlog triage gates all pass.
5. `origin/main` is pushed.
6. Memory is updated for every slice and for the phase summary.

---

## §12. Locked Decisions

| ID | Decision |
|---|---|
| GA-LD-01 | Docs-first; no implementation before this spec and all GA slice docs land. |
| GA-LD-02 | No GitHub PRs for this all-AI workflow. |
| GA-LD-03 | Codex CLI through AIT adversarial mode is the required reviewer. |
| GA-LD-04 | Max 5 review rounds; Staff+ arbitration after R5 is final. |
| GA-LD-05 | Findings are fixed in-slice unless explicitly cross-slice. |
| GA-LD-06 | Real-stack evidence is required for soak/load claims. |
| GA-LD-07 | Security and supply-chain signoff are GA gates, not post-GA chores. |

---

## §13. Adoption History

| Role | Decision | Outcome |
|---|---|---|
| Release Engineering Architect | Split release artifacts, versioning, Helm values, migration rollback, and release gates | GA_01 through GA_04 plus standards docs |
| SRE/Operations Architect | Separate dashboards from alert/runbook drills | GA_05 and GA_06 |
| Performance/Database Architect | Real-stack load, high-cardinality DB proof, and soak evidence cannot be shim-only | GA_07 and GA_08 gates require evidence bundles |
| Security Engineer | Supply-chain, SVID, secrets, RLS, replay, and PII boundaries need independent signoff | GA_09 |
| Customer Plugin/Backend Architect | Customer plugin certification and non-P1 backlog triage need a dedicated slice | GA_10 |

---

## §14. Merge Checklist

- [ ] Master spec committed
- [ ] Acceptance standard committed
- [ ] Review standard committed
- [ ] GA_01 through GA_10 slice docs committed
- [ ] Design branch pushed
- [ ] Implementation begins only after docs land

---

*Spec version: ga-readiness v1alpha1 | Base: main `38fdab1` | Branch: `design/ga-operational-readiness`*
