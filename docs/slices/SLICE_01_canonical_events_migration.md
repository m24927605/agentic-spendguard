# Slice 01 — `canonical_events` schema migration

> **Branch**: `slice/SLICE_01_canonical_events_migration`
> **Status**: draft (spec-approved; awaiting implementation)
> **Spec ancestor(s)**: `audit-chain-prediction-extension-v1alpha1.md` (primary), `predictor-architecture-spec-v1alpha1.md` (umbrella)
> **Depends on prior slices**: none (foundational slice)
> **Blocks subsequent slices**: ALL — every other slice writes to or reads from these columns
> **Estimated PR size**: small-medium (1 migration file, 1 trigger function update, 1 proto bump, ~600 LOC)

---

## §0. TL;DR

Add 17 new audit columns + 1 new tokenizer_versions table + immutability-trigger update + CloudEvent proto mirror at tag 300+. Strictly additive nullable schema; old rows unaffected; `verify-chain` regression must pass. This is the evidence-substrate slice — every subsequent slice depends on these columns existing.

---

## §1. Architectural context

per `predictor-architecture-spec-v1alpha1.md` §6 + `audit-chain-prediction-extension-v1alpha1.md` §1.1: audit chain 是 calibration evidence 的根；新欄位必須是 first-class（not nested in cloudevent_payload）才能讓 `calibration-report` CLI 高效 SQL aggregation；trigger update 是 audit-chain extension §5.2 已 identify 的 risk closure（Step 4 discrepancy #4）。

SLICE 01 serves Q1/Q2/Q3/Q4 of HANDOFF §5 indirectly — every downstream pillar writes to these columns.

---

## §2. Scope (must-do)

- Add 17 columns to `audit_outbox` table（11 decision-side + 3 run-level + 4 commit-side per audit-chain extension §2.1-§2.3，**含 §2.1 reviewer-flagged 11th column `cold_start_layer_used`**）
- Add columns to `canonical_events` table（mirror schema；replicated via outbox_forwarder unchanged）
- Update `reject_audit_outbox_immutable_columns` trigger to include all 17 new columns in OLD/NEW comparison list
- Create `tokenizer_versions` registry table（per `tokenizer-service-spec-v1alpha1.md` §6.1）
- Bump `proto/spendguard/common/v1/common.proto` `CloudEvent` message with 17 new fields at tags 300-317
- Add indexes for calibration-report queries (per audit-chain extension §4.1)
- Migration file: `services/canonical_ingest/migrations/00XX_audit_outbox_prediction_columns.sql`（編號 per current state）
- Ledger migration file: `services/ledger/migrations/00XX_audit_outbox_prediction_columns.sql`（same DDL on ledger DB）
- Update `services/canonical_ingest/src/verifier.rs` ONLY if proto change requires update（per audit-chain extension §6.1：no change needed in v1alpha1 because proto3 additive evolution）
- Verify-chain CLI 加 `--check-prediction-mirror` flag（per audit-chain extension §11.3）

---

## §3. Out of scope

| 項目 | 為何不在這 slice | 推給 |
|---|---|---|
| Logic that writes the new columns | Schema-only slice | SLICE 03+ |
| Tokenizer service implementation | Separate concern | SLICE 03 |
| Contract DSL bump | Different proto | SLICE 02 |
| Recommendation engine 在 CLI | CLI work | SLICE 13 |

---

## §4. File-level change list

### 4.1 New files

- `services/canonical_ingest/migrations/00XX_audit_outbox_prediction_columns.sql`
- `services/ledger/migrations/00XX_audit_outbox_prediction_columns.sql` (same DDL)
- `services/ledger/migrations/00XX_tokenizer_versions.sql` (registry table)

### 4.2 Modified files

- `proto/spendguard/common/v1/common.proto` — add tags 300-317 to `CloudEvent`
- `services/ledger/migrations/0011_immutability_triggers.sql`-style follow-up migration — update `reject_audit_outbox_immutable_columns` function (CREATE OR REPLACE FUNCTION; new tuple compare)
- `services/canonical_ingest/src/lib.rs` (if reverify schema bundle id rotation logic needed)
- `services/<each-producer>/src/audit.rs` — initial NOOP changes (mirror logic comes in later slices)
- `verify-chain` CLI binary or canonical_ingest sub-command — `--check-prediction-mirror` flag scaffolding

### 4.3 Helm / config changes

- `charts/spendguard/templates/migrations-job.yaml` — bumped migration list (if file lists migrations explicitly; else auto-pickup)
- No new ConfigMap / Secret needed

---

## §5. Schema / proto changes

per `audit-chain-prediction-extension-v1alpha1.md` §4.1 (full SQL DDL block); §3.2 (proto block).

Summary:
- 11 prediction columns + 3 run-level + 4 commit-side, all nullable
- Indexes: `audit_outbox_calibration_idx`, `audit_outbox_tier_idx`
- Proto `CloudEvent` adds 18 fields tagged 300-317 (additive evolution)
- Sentinel values per §6.3 of audit-chain extension

---

## §6. Audit-chain impact

- **New columns**: 17 total on both `audit_outbox` and `canonical_events`
- **Canonical bytes**: NO change needed — proto3 additive evolution carries new fields automatically (per audit-chain extension §7.1)
- **verify_cloudevent compatibility**: 既有 rows verify 仍 OK; new rows verify with new fields populated（proto3 unknown-field round-trip preservation property required）
- **Immutability trigger**: MUST update `reject_audit_outbox_immutable_columns` to include new columns; **adversarial review must verify UPDATE on each new column raises 42P10**
- **Storage class**: unchanged; new fields land in `immutable_audit_log` per Trace §10.2

---

## §7. Failure mode coverage

| 依賴 | 失敗情境 | 預期行為 |
|---|---|---|
| migration script runs on existing audit_outbox | partial completion | ROLLBACK；migration runner retries |
| proto codegen breaks downstream services | binary incompat | refuse-to-deploy；fail-fast |
| verify-chain hits NULL columns on existing rows | normal | verify pass (proto3 default = unset = signed identically) |
| schema_bundle_id rotation not coordinated | gradual rolling deployment | dual_read accept both old + new bundle_ids per `trace-schema-spec-v1alpha1.md` §6 dual_read |
| ledger migration vs canonical_ingest migration ordering | partial sync | apply both in same maintenance window |
| forwarder UPDATE path touches new column | unintended write | trigger raises 42P10 (test required) |
| **prost 0.13 unknown-field rollout invariant** (round-2 M8) | rolling upgrade gap | **canonical_ingest pods 必須全部 upgrade BEFORE 任何 sidecar / webhook_receiver / ttl_sweeper 開始寫 tag-300+ fields**；否則舊 verifier decode 丟掉 unknown fields → re-encode bytes ≠ producer signed bytes → verify FAIL（per audit-chain extension §7.2 round-2 update）|

---

## §8. Acceptance criteria

### 8.1 Unit tests

- Migration runs on fresh Postgres → all expected tables / columns / indexes exist
- Migration runs on existing demo Postgres → existing rows unchanged (NULL new columns)
- Trigger function with new column list: UPDATE on each new column raises 42P10
- Trigger function: forwarder's UPDATE on `pending_forward` etc. passes (no false positive)

### 8.2 Integration tests

- Insert audit row with all new columns populated → trigger allows initial INSERT
- Replay audit chain over mixed (old + new) rows → verify-chain passes
- prost round-trip: encode CloudEvent with tags 300-317 set, decode in v1alpha1-only verifier, re-encode, signature still verifies

### 8.3 Property tests

- For any populated combination of new columns + canonical CloudEvent, `verify_cloudevent` succeeds
- For any UPDATE attempt on the 17 new immutable columns, trigger raises 42P10

### 8.4 Benchmarks

- Migration completes on 1M-row demo audit_outbox in < 30 seconds
- INSERT throughput baseline measure before / after migration; regression < 5%

### 8.5 Audit invariant tests

- `verify-chain` regression: must run on (a) demo modes existing rows + (b) freshly written rows; both 全綠
- Mirror cross-check (per audit-chain extension §11.2) green

### 8.6 Demo-mode regression

`make demo-up DEMO_MODE=<each>` continues to pass for: `proxy / decision / deny / approval / ttl_sweep / agent_real / agent_real_anthropic / agent_real_langgraph / agent_real_openai_agents / agent_real_openai_agents_proxy / litellm_real / litellm_deny / approval_hot_reload / multi_provider_usd`.

### 8.7 Backwards compat

- Pre-migration clients (v1alpha1 sidecars) continue to write audit rows successfully (new columns left NULL)
- Schema_bundle_id rotation properly coordinated with producers

---

## §9. Slice-specific adversarial review checklist

Layered on top of `docs/review-standards/predictor-review-checklist.md` §1:

1. Is `cold_start_layer_used` properly included as the 11th decision-side column? Spec §2.4 reviewer note: this column was promoted from metadata to first-class. Confirm via spec + migration consistency.
2. For each of the 17 new columns, what is the corresponding CloudEvent proto field tag and what is the sentinel value mapping (per audit-chain extension §6.3)? Show the table verbatim in the PR description.
3. Is the trigger function CREATE OR REPLACE rather than DROP + CREATE? Required to avoid trigger downtime.
4. What happens if the migration is partially applied (e.g., audit_outbox columns added but trigger not updated)? Show transaction wrapping ensures atomicity within DDL session.
5. What is the schema_bundle_id rotation plan? Does it coordinate with all 4 producer services (sidecar / webhook_receiver / ttl_sweeper / ledger invoice_reconcile)?
6. prost round-trip test: explicitly test old-verifier-on-new-row and new-verifier-on-old-row scenarios.
7. Is `verify-chain --check-prediction-mirror` flag default true for new versions? What's the upgrade migration for the CLI binary?
8. Are the new indexes covered for partition-safe (per `0009_audit_outbox.sql` partition convention)?
9. Tokenizer_versions table FK in `audit_outbox.tokenizer_version_id`: ON DELETE behavior? (Recommend RESTRICT to avoid losing audit lineage.)
10. Does the migration include INSERT for initial `tokenizer_versions` rows (cl100k_base, o200k_base, p50k_base for OpenAI Tier 2 SLICE 03 prerequisites)? Or deferred to SLICE 03?

---

## §10. Out-of-scope deferrals

| 項目 | 理由 | 推給 |
|---|---|---|
| Populating `tokenizer_versions` initial rows | Needs tokenizer dispatch table | SLICE 03 |
| Writing new column values | This slice is schema-only | SLICE 03+ |
| `prediction_drift_alert` event subscriber | Stats aggregator-side | SLICE 06 |
| Tokenizer asset signed bundle integration | Tokenizer service slice | SLICE 03 |

---

## §11. Risk / rollback plan

- Worst case: migration runs but trigger update fails → audit chain immutability invariant broken on new columns
- Mitigation: wrap migration in transaction; trigger CREATE OR REPLACE atomic
- Rollback: revert migration via down-migration (`ALTER TABLE DROP COLUMN ...` reverse); restore old trigger function from backup
- Demo regression: `make demo-up` should detect immediately if columns missing on producer side

---

## §12. AIT execution notes

- Recommended `--agent Database Optimizer`（per HANDOFF §10.1）or `Backend Architect`
- `--review-budget deep`（migration changes are high-stakes）
- Expected rounds: 2-3（schema-only typically clean）
- Risk factor: if mirror approach (audit-chain ext §3.4 design A) raises challenges in review, escalate per §9 questions 1-2

---

## §13. Adoption history (filled during review)

| Round | Reviewer | 採納率 | 主要產出 |
|---|---|---|---|
| (placeholder) | (placeholder) | (placeholder) | (placeholder) |

---

## §14. Merge checklist (before `ait apply`)

- [ ] §8.1-§8.7 all green
- [ ] §9 slice-specific checklist 全清
- [ ] `predictor-review-checklist.md` §1 universal checks 全清
- [ ] `verify-chain` regression green
- [ ] All 8+ demo modes 全綠
- [ ] commit metadata 含 attempt_id (AIT-native 自動)
- [ ] PR description link 回 spec ancestors (`audit-chain-prediction-extension-v1alpha1.md`)

---

*Slice version: SLICE_01_canonical_events_migration v1alpha1 (draft) | Spec ancestor: audit-chain-prediction-extension-v1alpha1.md | Blocks: every subsequent slice depends on these columns | Branch: `slice/SLICE_01_canonical_events_migration`*
