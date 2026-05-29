# Slice 01 — `canonical_events` schema migration

> **Branch**: `slice/SLICE_01_canonical_events_migration`
> **Status**: draft (spec-approved; awaiting implementation)
> **Spec ancestor(s)**: `audit-chain-prediction-extension-v1alpha1.md` (primary), `predictor-architecture-spec-v1alpha1.md` (umbrella)
> **Depends on prior slices**: none (foundational slice)
> **Blocks subsequent slices**: ALL — every other slice writes to or reads from these columns
> **Estimated PR size**: small-medium (1 migration file, 1 trigger function update, 1 proto bump, ~600 LOC)

---

## §0. TL;DR

Add 18 new audit columns + 1 new tokenizer_versions table + immutability-trigger update + CloudEvent proto mirror at tag 300+. Strictly additive nullable schema; old rows unaffected; `verify-chain` regression must pass. This is the evidence-substrate slice — every subsequent slice depends on these columns existing.

> Round-2 note: column count is **18** not 17 per audit-chain-prediction-extension §2.4 — `cold_start_layer_used` promoted from metadata blob to first-class column. Wherever this slice doc previously said "17" it now reads "18" with the same intent.

---

## §1. Architectural context

per `predictor-architecture-spec-v1alpha1.md` §6 + `audit-chain-prediction-extension-v1alpha1.md` §1.1: audit chain 是 calibration evidence 的根；新欄位必須是 first-class（not nested in cloudevent_payload）才能讓 `calibration-report` CLI 高效 SQL aggregation；trigger update 是 audit-chain extension §5.2 已 identify 的 risk closure（Step 4 discrepancy #4）。

SLICE 01 serves Q1/Q2/Q3/Q4 of HANDOFF §5 indirectly — every downstream pillar writes to these columns.

---

## §2. Scope (must-do)

- Add 18 columns to `audit_outbox` table（11 decision-side + 3 run-level + 4 commit-side per audit-chain extension §2.1-§2.3，**含 §2.4 reviewer-flagged 11th decision-side column `cold_start_layer_used`** that brought the total from 17 to 18）
- Add columns to `canonical_events` table（mirror schema；replicated via outbox_forwarder unchanged）
- Update `reject_audit_outbox_immutable_columns` trigger to include all 18 new columns in OLD/NEW comparison list
- Create `tokenizer_versions` registry table（per `tokenizer-service-spec-v1alpha1.md` §6.1）
- Bump `proto/spendguard/common/v1/common.proto` `CloudEvent` message with 18 new fields at tags 300-317
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

- `services/ledger/migrations/0046_audit_outbox_prediction_columns.sql`
- `services/ledger/migrations/0048_tokenizer_versions.sql` (registry table + FK to audit_outbox)
- `services/canonical_ingest/migrations/0013_canonical_events_prediction_columns.sql`
- `services/canonical_ingest/migrations/0015_audit_outcome_quarantine_prediction_columns.sql` (round-3 B2: quarantine table mirror so the release path can carry forward the 18 prediction columns)
- Down-migrations for each (round-2 fix m2; round-3 fixes M4 + M10 + B3)

### 4.2 Modified files

- `proto/spendguard/common/v1/common.proto` — add tags 300-317 to `CloudEvent`
- `services/ledger/migrations/0011_immutability_triggers.sql`-style follow-up migration — update `reject_audit_outbox_immutable_columns` function (CREATE OR REPLACE FUNCTION; new tuple compare)
- `services/canonical_ingest/src/lib.rs` (if reverify schema bundle id rotation logic needed)
- `services/<each-producer>/src/audit.rs` — initial NOOP changes (mirror logic comes in later slices)
- `verify-chain` CLI binary or canonical_ingest sub-command — `--check-prediction-mirror` flag scaffolding

### 4.3 Helm / config changes

- `charts/spendguard/templates/migrations.yaml` — round-2 (B2 + M15): production-profile fail-gate that requires the new migration ConfigMaps + the SLICE_01 files; demo-profile keeps the optional-ConfigMap no-op semantics. **Round-3 (B1)**: fail-gate now wrapped in `.Release.IsInstall / .Release.IsUpgrade` so `helm template` (no cluster) skips the lookup — CI helm-validate.yml dry-render passes; real install/upgrade still enforces. **Round-3 (m6)**: demo profile now emits a multi-line WARNING when ConfigMaps lack the SLICE_01 files (operator observability without aborting demo).
- `charts/spendguard/templates/NOTES.txt` — round-2 (B2): explicit "SLICE_01 UPGRADE NOTICE" block with the regeneration commands + the ledger-before-canonical ordering + the prost rollout invariant warning.
- `charts/spendguard/Chart.yaml` — round-2 (B2): version bump 0.1.0-alpha.1 → 0.1.0-alpha.2. **Round-3 (m5)**: declared `kubeVersion: ">=1.24.0"`; version bumped again 0.1.0-alpha.2 → 0.1.0-alpha.3 to surface the round-3 fixes.
- No new ConfigMap / Secret needed by the chart itself; the operator regenerates the existing migration ConfigMaps to include the new files.

**Cross-DB migration ordering** (round-2 fix M16): ledger DB migrations MUST complete before canonical_ingest DB migrations. Reason: the canonical mirror columns assume the ledger side has already accepted them; the outbox_forwarder will not push rows whose ledger row failed to insert. The Helm chart's apply loop enforces this by processing the ledger glob before the canonical glob; out-of-band migration workflows must follow the same order manually. Within each DB the lexicographic file order is also load-bearing: 0046 (audit_outbox columns + trigger + TRUNCATE guard, atomic) MUST land before 0048 (tokenizer_versions + FK) so the FK target exists; 0013 (canonical_events mirror) MUST land before 0015 (quarantine table mirror — round-3 B2 added; round-2's 0014 schema_bundle placeholder was deleted in round-3 B3).

---

## §5. Schema / proto changes

per `audit-chain-prediction-extension-v1alpha1.md` §4.1 (full SQL DDL block); §3.2 (proto block).

Summary:
- 11 prediction columns + 3 run-level + 4 commit-side = 18 total, all nullable
- Indexes: `audit_outbox_calibration_idx`, `audit_outbox_tier_idx`, `audit_outbox_outcome_calibration_idx` (round-2 added)
- Proto `CloudEvent` adds 18 fields tagged 300-317 (additive evolution)
- Sentinel values per §6.3 of audit-chain extension

---

## §6. Audit-chain impact

- **New columns**: 18 total on both `audit_outbox` and `canonical_events`
- **Canonical bytes**: NO change needed — proto3 additive evolution carries new fields automatically (per audit-chain extension §7.1). **Round-2 caveat** (per audit-chain extension §7.2 update): prost 0.13 does NOT preserve unknown fields → canonical_ingest pods must be upgraded BEFORE any producer starts writing tag-300+ fields.
- **verify_cloudevent compatibility**: 既有 rows verify 仍 OK; new rows verify with new fields populated（proto3 unknown-field round-trip preservation property required — see round-2 caveat above）
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
- Trigger function with new column list: UPDATE on each new column raises 42P10 (`services/ledger/tests/migrations/test_prediction_columns_trigger.sql` round-3 fix M8 fixture)
- Trigger function: forwarder's UPDATE on `pending_forward` etc. passes (no false positive)
- **Round-3 fix B2**: outcome-before-decision quarantine + release path preserves all 18 prediction columns end-to-end (`services/canonical_ingest/tests/migrations/test_quarantine_release_prediction_cols.sql`)

### 8.2 Integration tests

- Insert audit row with all new columns populated → trigger allows initial INSERT
- Replay audit chain over mixed (old + new) rows → verify-chain passes
- prost round-trip: encode CloudEvent with tags 300-317 set, decode in v1alpha1-only verifier, re-encode, signature still verifies

### 8.3 Property tests

- For any populated combination of new columns + canonical CloudEvent, `verify_cloudevent` succeeds
- For any UPDATE attempt on the 18 new immutable columns, trigger raises 42P10

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

1. Is `cold_start_layer_used` properly included as the 11th decision-side column? Spec §2.4 reviewer note: this column was promoted from metadata to first-class, bringing the total to 18. Confirm via spec + migration consistency.
2. For each of the 18 new columns, what is the corresponding CloudEvent proto field tag and what is the sentinel value mapping (per audit-chain extension §6.3)? Show the table verbatim in the PR description.
3. Is the trigger function CREATE OR REPLACE rather than DROP + CREATE? Required to avoid trigger downtime.
4. What happens if the migration is partially applied (e.g., audit_outbox columns added but trigger not updated)? Show transaction wrapping ensures atomicity within DDL session.
5. What is the schema_bundle_id rotation plan? Does it coordinate with all 4 producer services (sidecar / webhook_receiver / ttl_sweeper / ledger invoice_reconcile)? **Round-3 answer (B3 reversal of round-2 M10)**: schema_bundle_id rotation is now **deferred to SLICE_06 producer slice**. Round-2 landed `0014_schema_bundle_prediction_v1alpha1.sql` with a placeholder hash + `cosign_verified_at = NULL`; both were security-hostile (the placeholder hash was reversible from a public string — `sha256("spendguard.v1alpha1+prediction")` — so any attacker holding the producer signing key could synthesise events that the placeholder bundle "verified", and the NULL cosign field meant nothing in code enforced "production must have cosign-verified bundles"). Round-3 DELETES 0014 + 0014_down. SLICE_01 ships the schema substrate only; producers MUST register a new **cosigned** bundle row BEFORE writing tag-300+ fields (the operator-side bundle builder per Trace §12 computes the real sha256 of canonicalized proto bytes + records cosign verification). canonical_ingest accepts both old and new bundles concurrently per Trace §6 dual_read.
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
| **schema_bundle_id rotation (round-3 B3)** | The placeholder bundle in round-2's 0014 was security-hostile (public-string hash + NULL cosign); cosigned bundle row insertion requires the operator-side bundle builder per Trace §12 | SLICE 06 |
| **`verify-chain --check-prediction-mirror` full implementation (round-2 B3 / round-3 M5)** | Per-row scan path requires producer-side mirror writes; the round-3 CLI scaffold exits 2 (fail-closed) on the default flag so CI gates don't silent-pass | SLICE 06 |
| **Rolling-upgrade enforcement via pre-upgrade Helm Job (round-2 M10 / round-3 M12)** | Round-2 only documented the rolling-upgrade invariant (canonical_ingest pods MUST upgrade before producer pods start writing tag 300+ fields). Round-3 explicitly defers programmatic enforcement to SLICE_06 along with the producer-side mirror writes — at that point a pre-upgrade Job hook can compare image versions or call a version endpoint on canonical_ingest. SLICE_01 ships the documented invariant only (charts/NOTES.txt §"PROST 0.13 ROLLOUT INVARIANT") | SLICE 06 |

---

## §11. Risk / rollback plan

- Worst case: migration runs but trigger update fails → audit chain immutability invariant broken on new columns
- Mitigation (round-2 fix M7): 0046 atomically applies schema + CHECK + indexes + trigger + TRUNCATE guard in a single file. The migration runner wraps it in a transaction so partial application is impossible — either every change lands or none does.
- Rollback: down-migrations live under `services/{ledger,canonical_ingest}/migrations/down/` (round-2 fix m2; round-3 fix B3 deleted 0014 + 0014_down; round-3 fix B2 added 0015 + 0015_down):
  - `services/ledger/migrations/down/0046_audit_outbox_prediction_columns_down.sql`
  - `services/ledger/migrations/down/0048_tokenizer_versions_down.sql`
  - `services/canonical_ingest/migrations/down/0013_canonical_events_prediction_columns_down.sql`
  - `services/canonical_ingest/migrations/down/0015_audit_outcome_quarantine_prediction_columns_down.sql`
- **Rollback order** (per round-2 fix M16 cross-DB ordering + round-3 B2/B3 updates):
  1. Stop all 4 producer services (sidecar / webhook_receiver / ttl_sweeper / ledger invoice_reconcile) so no new tag-300+ writes happen.
  2. `SET spendguard.allow_destructive_down = on;` on the target Postgres session (round-3 fix M4 destructive-down guard).
  3. Apply canonical_ingest down-migrations: 0015_down → 0013_down. Order rationale (round-3 fix M11 rewrite of round-2 step 2): 0015_down first because the quarantine table's 18 prediction columns reference no foreign keys to canonical_events; dropping them first removes the dependency edge so 0013_down can then drop the canonical_events columns. (Round-2's claim that 0014 had to come first was based on a non-existent FK chain — 0014 only inserted a schema_bundles row.)
  4. Apply ledger down-migrations: 0048_down → 0046_down (downstream first; the FK on audit_outbox.tokenizer_version_id is dropped in 0048_down before 0046_down drops the column).
  5. Roll back canonical_ingest pods to the pre-SLICE_01 image (recovers from the prost rollout invariant M8).
  6. Roll back producer pods to the pre-SLICE_01 image.
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
