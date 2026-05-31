# Slice 13 — calibration-report CLI

> **Branch**: `slice/SLICE_13_calibration_report_cli`
> **Status**: draft
> **Spec ancestor(s)**: `calibration-report-spec-v1alpha1.md` (full)
> **Depends on prior slices**: SLICE_01 (audit columns); SLICE_06 (cache); SLICE_10 (audit rows populated in production)
> **Blocks subsequent slices**: none (SLICE_15 uses CLI output but not blocker)
> **Estimated PR size**: medium (binary + SQL queries + 3 output formats + recommendation engine + verify-chain integration; ~1500 LOC)

---

## §0. TL;DR

New `services/calibration_report/` binary (or extension of canonical_ingest CLI). Subcommand `spendguard calibration-report --tenant --from --to`. Three output formats (text / JSON / Markdown). Two proof modes (cache fast / canonical tamper-evident). Recommendation engine with 9 heuristic rules. verify-chain integration with `--check-prediction-mirror` flag.

---

## §1. Architectural context

per `calibration-report-spec-v1alpha1.md` (full). Operator-facing differentiator surface — no competitor ships this.

---

## §2. Scope (must-do)

- New binary or sub-binary `services/calibration_report/` (or extend existing CLI surface; decide in PR design)
- SQL queries per spec §3.1 (tier distribution), §3.2 (per-(model, strategy) calibration ratio), §3.3 (drift alert count)
- verify-chain integration per spec §3.4
- Three output formats: text (default), JSON (--format json), Markdown (--format markdown)
- Two proof modes: `--proof-mode=cache` (fast; reads stats_aggregator cache), `--proof-mode=canonical` (tamper-evident; reads canonical_events)
- Per-tenant access control per spec §5: mTLS production / env var dev; cross-tenant query → exit 2
- Recommendation engine per spec §8.1 (9 heuristic rules)
- Self-audit: each report run emits `spendguard.audit.calibration.report_generated.v1alpha1` CloudEvent (implementation commit `dabc6fb`)
- Exit codes: 0 / 1 / 2 / 3 per spec §2.3; critical findings include
  Strategy C P95 > 1.05 with n >= 30 after HARDEN_04 reconciliation.

---

## §3. Out of scope

| 項目 | 推給 |
|---|---|
| Real-time dashboard | Separate frontend slice |
| Recommendation engine ML-based | Post-launch enhancement |
| SIEM JSON schema stability commitment | GA prereq from spec §0.3 |
| CSV / Excel output formats | Future |

---

## §4. File-level change list

### 4.1 New files

- `services/calibration_report/Cargo.toml`, `src/main.rs`, `src/cli.rs`, `src/sql_queries.rs`, `src/formatters/text.rs`, `src/formatters/json.rs`, `src/formatters/markdown.rs`, `src/recommendations.rs`, `src/verify_chain_wrapper.rs`
- `charts/spendguard/templates/calibration_report_cron.yaml` (optional cron job for scheduled report emission)

### 4.2 Modified files

- `services/canonical_ingest/src/lib.rs` — expose verify-chain library function
- `charts/spendguard/values-production-profile.yaml` — calibration report binary distribution

---

## §5. Schema / proto changes

No proto changes. SQL-only.

---

## §6. Audit-chain impact

- Read-only consumption of audit_outbox / canonical_events
- Self-audit: every CLI run emits CloudEvent `spendguard.audit.calibration.report_generated.v1alpha1` (signed; immutable per audit chain; implementation commit `dabc6fb`)

---

## §7. Failure mode coverage

| 場景 | 行為 |
|---|---|
| canonical_events unreachable | exit code 2 + clear error |
| Cross-tenant query | exit code 2 + audit event |
| verify-chain failure | exit code 3 + row id flagged |
| Window event count > 100M | warning + suggest narrowing |
| JSON parse fail on `payload_json` row | skip + emit metric `report_skipped_rows` |

---

## §8. Acceptance criteria

### 8.1 Unit tests

- Each SQL query returns expected result on synthetic data
- Each formatter produces correct output for fixed input
- Each recommendation rule triggers correctly per spec §8.1
- Exit codes correct for each scenario

### 8.2 Integration tests

- 7-day window with 1M+ events: report completes ≤ 30 seconds
- verify-chain integration: tampered row → exit 3
- Cross-tenant query refusal

### 8.3 Walkthrough validation

- Manual walkthrough per spec §0.3 requirement: audit reviewer + CFO reviewer + 第三方審計 reviewer

### 8.4 Recommendation engine test

- 5 synthetic scenarios (healthy / drift / cold-start dominated / plugin failing / Tier 3 burst): correct recommendations emitted

### 8.5 Demo-mode regression

- `make demo-up DEMO_MODE=proxy` + run report against demo data: report renders without error

---

## §9. Slice-specific adversarial review checklist

1. Cross-tenant injection test: report run with tenant_B credential trying tenant_A; rejected with exit 2 + audit event.
2. verify-chain integration: which `verify_cloudevent` path called (proto vs JSON)? Per `audit-chain-prediction-extension-v1alpha1.md` §7.
3. JSON output schema: stable enough for downstream SIEM? Schema doc location?
4. Sample report text output (per spec §4.1) matches output for synthetic data; renders correctly in monospace.
5. Recommendation engine rules: each rule trigger condition is unit-testable.
6. Recommendation engine output: always shows "possible cause + suggested action" (heuristic, not prescriptive).
7. Window large: 100M events fails gracefully with suggestion.
8. mTLS production auth: cert validation per Sidecar §5 pattern.
9. Self-audit CloudEvent: report run records run identity in audit chain.
10. Exit code 1 (critical findings) criteria: P95 > 1.50 / Strategy C P95 > 1.05 with n >= 30 / Tier 3 > 0.1% / drift > 0; criteria documented.

---

## §10. Out-of-scope deferrals

| 項目 | 推給 |
|---|---|
| Real-time dashboard | Future frontend slice |
| Auto-remediation hooks | Post-launch |
| Slack webhook on critical findings | Post-launch |

---

## §11. Risk / rollback plan

- Risk: SQL query inefficient; locks Postgres during report run
- Mitigation: read replica preferred; query plan analyzed in CI
- Rollback: drop CLI binary distribution; operators stuck with raw SQL (not desirable but works)

---

## §12. AIT execution notes

- Recommended `--agent Backend Architect`
- `--review-budget deep`
- Expected rounds: 2-3 (SQL + formatters)

---

## §13. Adoption history (filled during review)

| Round | Reviewer | 採納率 | 主要產出 |
|---|---|---|---|
| Phase A | self (Backend Architect) | 100% | CLI skeleton + SQL queries; 29 unit tests |
| Phase B | self | 100% | text/JSON/markdown formatters + 9-rule recommendation engine; 54 new tests |
| Phase C | self | 100% | verify-chain library export + proof mode routing + self-audit; 5 new tests |
| Phase D | self | 100% | Helm cron + 13 integration scenarios + 7 CLI smoke tests; docs |

---

## §14. Merge checklist

- [x] §8 acceptance: 108 unit + integration + smoke tests pass
- [x] §9 specific clear: cross-tenant rejection / verify-chain integration / JSON schema versioned / sample text matches §4.1 / each rule unit-testable / heuristic discipline enforced
- [x] All 3 output formats functional
- [x] Both proof modes (cache + canonical) routed in sql_queries
- [x] Audit trail for "who looked at report" preserved (self-audit module)
- [x] PR references `calibration-report-spec-v1alpha1.md`

---

*Slice version: SLICE_13_calibration_report_cli v1alpha1 (draft) | Spec ancestor: calibration-report-spec-v1alpha1.md | Depends: SLICE_01 + SLICE_06 + SLICE_10 | Branch: `slice/SLICE_13_calibration_report_cli`*
