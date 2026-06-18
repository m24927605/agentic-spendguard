# Post-GA Backlog Spec v1alpha1

> **Status**: draft
> **Phase**: post-GA backlog execution
> **Base**: `main` at `c80a1e2` after GA_10
> **Drives**: `docs/internal/slices/POST_GA_01_*.md` through `docs/internal/slices/POST_GA_10_*.md`
> **Owner**: Staff+ post-GA panel; codex CLI implementer and adversarial reviewer

---

## §0. Executive Summary

GA_10 closed the production-readiness blocker set and mapped the
remaining non-P1 issues into ten named post-GA implementation slices.
This spec turns that triage table into executable design, implementation,
test, acceptance, and review standards.

The post-GA backlog is not part of the GA production-ready gate. It is a
controlled follow-up program for P2/P3 polish, performance, deeper test
coverage, and spec cleanup. Each slice remains small enough for
adversarial review to converge without hiding unrelated work.

---

## §1. Architectural Context

The predictor upgrade is now merged through GA_10. The locked production
invariants remain:

- no reopening of predictor architecture or GA security decisions
- no destructive migration rewrite; only forward migrations
- tenant IDs remain `uuid::Uuid` across runtime trust boundaries
- append-only audit behavior remains immutable
- container, RLS, mTLS, SVID, and secret-handling baselines remain at
  HARDEN/GA standards

Post-GA work must improve the shipped system without weakening its
enforcement path. When a cleanup conflicts with a locked invariant, the
invariant wins and the slice records the tradeoff.

---

## §2. Scope

This phase covers the open #85-#177 non-P1 backlog items assigned by
GA_10:

- ledger release replay semantics
- Contract DSL and spec/doc cleanup
- tokenizer runtime hardening
- tokenizer asset and performance work
- Tier 1 provider coverage expansion
- stats drift hygiene
- output predictor API evolution
- database index and RLS polish
- Strategy C resilience
- test quality improvements

---

## §3. Out of Scope

| Item | Why |
|---|---|
| New product UI | Product design work; not required for backlog closure |
| New managed SaaS control plane | Outside current self-hosted GA scope |
| Rewriting shipped migrations | Breaks upgrade path; use additive migrations |
| Relaxing GA security gates | Not permitted |
| Closing unrelated GitHub issues | Only issues explicitly mapped here are in phase scope |

---

## §4. Slice Inventory

| Slice | Scope | Issues |
|---|---|---|
| POST_GA_01_ledger_replay_semantics | Release reservation replay, fencing, and status semantics | #85, #86, #87 |
| POST_GA_02_contract_spec_cleanup | Documentation/spec title and wording drift | #91, #93, #97, #99, #101, #113, #121, #123, #131, #136, #141, #147, #154, #158, #159, #167, #177 |
| POST_GA_03_tokenizer_runtime_hardening | Tokenizer readiness, rate limits, request IDs, UDS docs, parity, security, partition/retention, serialization concerns, tests | #92, #94, #96, #98, #100, #103, #105, #110, #111, #112, #114, #115, #117, #118, #119, #126, #127, #129, #133, #135, #148, #149, #151, #152, #156 |
| POST_GA_04_tokenizer_asset_performance | Tokenizer asset size, dispatch performance, duplication cleanup, encoder benchmark expansion | #95, #102, #104, #108, #116, #120, #122, #125, #130, #134, #140 |
| POST_GA_05_provider_coverage | Cohere and Llama Tier 1 provider clients and envelope tuning | #139 |
| POST_GA_06_stats_drift_hygiene | Prediction drift alert source, dedup, cooldown, and NaN guard | #157, #162 |
| POST_GA_07_predictor_api_evolution | Output predictor response/policy shape and per-tenant Predict API rate limits | #161, #165 |
| POST_GA_08_db_index_and_rls_polish | Output cache index cardinality, nil UUID sentinel, advisory-lock runbook, migration hardening | #146, #163, #164, #166 |
| POST_GA_09_strategy_c_resilience | Strategy C stale cache, herd control, input caps, reset audit enrichment, reason caps | #172, #173, #174, #175, #176 |
| POST_GA_10_test_quality | Cross-check fixtures and remaining smoke-test improvements | #109, #124 |

---

## §5. Implementation Workflow

Every slice follows:

1. `git checkout -b post-ga/POST_GA_NN_<name> main`
2. Implement only that slice's §4 file-level change list.
3. Commit in small units with `Co-Authored-By: Codex <codex@openai.com>`.
4. Run all acceptance gates in the slice doc.
5. Dispatch codex CLI review directly.
6. Fix every Blocker, Major, and Minor in-slice.
7. Repeat review up to 5 rounds.
8. If round 5 still has findings, run Staff+ arbitration and follow the
   final ruling.
9. Merge to main with `--no-ff`, push `origin/main`, and write memory.

No GitHub PR is opened for the all-AI workflow.

---

## §6. Audit / Security / Operational Impact

Post-GA slices may touch audit, tokenizer, predictor, ledger, stats, and
deployment surfaces. Required guardrails:

- audit rows stay append-only
- replay/idempotency fixes must be deterministic
- drift and tokenizer alerts must keep the `spendguard.audit.*` routing
  invariant when they enter immutable audit
- RLS changes use `FOR ALL` plus `WITH CHECK`; no `BYPASSRLS`
- production Helm examples keep DB URLs in Secrets
- mTLS/SVID behavior does not regress
- runtime performance work must include before/after evidence

---

## §7. Failure Modes

| Failure | Required handling |
|---|---|
| Slice reopens a GA invariant | Stop and record Staff+ decision before implementation |
| Issue coverage mismatch | Validator fails; doc must be corrected before implementation |
| Review finds mixed scope | Split or defer unrelated work to a named slice |
| Acceptance gate cannot run locally | Record concrete environment blocker and Staff+ decision |
| Five review rounds still have findings | Convene Staff+ arbitration; panel decision is final |

---

## §8. Acceptance Standard

Each slice must include:

- a command-level test plan
- affected service build/test gates
- Helm demo and production render gates when deploy surfaces are touched
- migration smoke gates when SQL is touched
- demo gates when runtime behavior is touched
- evidence files under `docs/internal/reviews/post-ga/POST_GA_NN_<name>/`
- an issue closure checklist

For docs-only slices, `scripts/ga/validate-post-ga-docs.sh`, `git diff
--check`, and relevant link/grep checks are the minimum gate.

---

## §9. Review Standard

The reviewer checks:

1. slice implementation matches §4
2. every mapped issue is actually addressed
3. acceptance gates ran and evidence is reproducible
4. no new production-readiness gap is introduced
5. no docs cite nonexistent conventions
6. no finding is deferred without a named cross-slice prerequisite

Every finding requires a fix unless Staff+ arbitration accepts it as
out-of-scope after R5.

---

## §10. Deferrals

| Deferral class | Rule |
|---|---|
| Product enhancements | Must move to roadmap issue outside #85-#177 |
| Large architecture changes | Require new spec before implementation |
| Provider-specific tuning | May remain in POST_GA_05 only when testable |
| Performance optimizations | Must include measured baseline and target |

---

## §11. Risk / Rollback

Post-GA changes are lower priority than GA correctness, but several can
touch critical paths. Rollback rules:

- docs-only changes revert normally
- runtime code changes need feature flags or narrow blast radius when
  feasible
- SQL changes use forward migrations and down scripts where the repo
  pattern already provides them
- performance changes must preserve the correctness test suite before
  benchmark gains count

---

## §12. Review Execution Notes

Reviewer: codex CLI via `codex review --base main`. Max 5 rounds. Staff+ panel arbitration if 5 rounds fail.

Run the review from the slice branch and include the slice doc path in
the review notes when recording findings. Do not use the claude-code
adapter.

---

## §13. Adoption History

| Role | Decision | Outcome |
|---|---|---|
| Software Architect | Keep post-GA as 10 slices, not one backlog mega-branch | §4 slice inventory |
| Backend Architect | Runtime slices must preserve shipped API compatibility unless explicitly versioned | §6 and slice-specific §5 |
| Security Engineer | Security issues stay in-slice even when labeled P2/P3 | POST_GA_03, POST_GA_08, POST_GA_09 |
| Database Optimizer | DB/index cleanup must prove planner or migration behavior | POST_GA_08 acceptance gates |
| SRE/Operations Architect | Runbooks and readiness fixes require executable checks | POST_GA_03 and POST_GA_08 |
| Domain Expert: SpendGuard | Remaining backlog is not GA-blocking but must be review-complete before closure | §8 and §9 |

---

## §14. Merge Checklist

- [ ] Master spec merged to design branch
- [ ] All ten POST_GA slice docs exist
- [ ] `scripts/ga/validate-post-ga-docs.sh` passes
- [ ] Staff+ decisions are recorded in §13 of every slice
- [ ] Review execution sentence appears in every slice §12
- [ ] Main implementation starts only after docs are merged/pushed
