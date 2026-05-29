-- Audit chain prediction extension (SLICE 01 — schema additions +
-- immutability trigger + TRUNCATE guard, atomic).
--
-- Spec: docs/audit-chain-prediction-extension-v1alpha1.md §2 + §4.1 + §5
-- Slice: docs/slices/SLICE_01_canonical_events_migration.md
--
-- 18 new nullable columns on audit_outbox:
--   * 11 decision-side prediction columns (§2.1) — note: 11 not 10 per
--     §2.4 reviewer-flagged promotion of cold_start_layer_used to a
--     first-class column.
--   * 3 run-level projection columns (§2.2)
--   * 4 commit-side actual columns (§2.3)
--
-- All ADD COLUMN with implicit NULL default. No backfill — existing rows
-- stay NULL forever (proto3 default-encoding semantics keep their
-- producer_signature valid; see §7 of the spec).
--
-- ADD COLUMN nullable is a metadata-only operation on Postgres 11+ — no
-- row rewrite even on partitioned audit_outbox. The migration is
-- effectively instantaneous regardless of partition count.
--
-- Producer code that writes these columns lands in SLICE_06+ (sidecar /
-- webhook_receiver / ttl_sweeper / ledger invoice_reconcile mirror); this
-- migration installs the schema substrate AND the immutability-trigger
-- update AND the TRUNCATE guard in a single atomic step so partial
-- application can never leave the table with the new columns mutable
-- (round-2 fix: 0047 merged here per Codex finding M7+m3+M13).
--
-- Migration runner wrapping convention: this project's migration runner
-- wraps each .sql file in its own transaction (per the 57 pre-existing
-- migrations 0000-0045 / 0001-0012 — none open explicit BEGIN/COMMIT).
-- Explicit transaction wrapping is deliberately omitted here to match
-- that convention; the runner's wrap covers ALL statements in this file
-- including the trigger update and TRUNCATE guard.
--
-- Cross-DB deployment ordering (round-2 fix M16): ledger DB migrations
-- (0046+0047+0048 — though 0047 is now merged here) MUST complete before
-- canonical_ingest DB migrations (0013) start. Reason: the canonical
-- mirror columns assume the ledger side has already accepted them; the
-- outbox_forwarder will not push rows whose ledger row failed to insert.
-- Operators using charts/spendguard/templates/migrations.yaml see this
-- ordering enforced by the script (ledger glob processed before
-- canonical glob in the apply loop).
--
-- ============================================================================
-- DESIGN RATIONALE — audit_outbox_global_keys deliberately NOT extended
-- (round-3 fix M15: moved here from step 8 inline block so the rationale
-- documents the file as a whole rather than appearing as a step with no
-- SQL).
--
-- The 18 new columns are calibration-aggregation evidence (token counts,
-- prediction confidence, run-level projections), not dedup keys. The
-- audit_outbox_global_keys mirror table exists to enforce cross-
-- partition uniqueness on dedup-relevant fields (decision_id,
-- producer_sequence, idempotency_key); extending it with prediction
-- columns would not add any uniqueness invariant and would force every
-- INSERT to write the same 18 columns twice with no functional benefit.
--
-- Cross-storage consistency between audit_outbox and audit_outbox_global_keys
-- on the dedup keys remains enforced by the post_ledger_transaction
-- stored proc (services/ledger/migrations/0012_post_ledger_transaction.sql).
--
-- ============================================================================
-- VALIDATE batching note (round-3 m3): the 14 sequential VALIDATE CONSTRAINT
-- statements in step 2 each take a SHARE UPDATE EXCLUSIVE lock on the table
-- and scan all rows. On an empty audit_outbox (fresh install) this is
-- instantaneous. On a production re-run against a populated audit_outbox
-- with N months of partitions, each VALIDATE scans every partition
-- sequentially. Operators should run this migration during a low-traffic
-- window — total elapsed scales linearly with row count × constraint count.
-- See the M6 deployment-safe NOT VALID + VALIDATE pattern.

-- ============================================================================
-- Step 1: Add the 18 new columns. INT → BIGINT for token-count columns
-- per round-2 finding M4 (anticipates 2^31-overflow over multi-year
-- aggregation; provider context windows already exceed 1M and BIGINT
-- costs are negligible vs INT on Postgres TOAST-eligible row paths).
-- prediction_confidence type is NUMERIC(4,3) per M12 for deterministic
-- AVG / GROUP BY semantics in calibration-report (REAL allows
-- non-deterministic IEEE-754 reordering).
-- ============================================================================

ALTER TABLE audit_outbox
    -- === Decision-side prediction columns (11 total per §2.1) ===
    ADD COLUMN predicted_a_tokens         BIGINT,
    ADD COLUMN predicted_b_tokens         BIGINT,
    ADD COLUMN predicted_c_tokens         BIGINT,
    ADD COLUMN reserved_strategy          TEXT,
    ADD COLUMN prediction_strategy_used   TEXT,
    ADD COLUMN prediction_policy_used     TEXT,
    ADD COLUMN tokenizer_tier             TEXT,
    ADD COLUMN tokenizer_version_id       UUID,
    ADD COLUMN prediction_confidence      NUMERIC(4,3),
    ADD COLUMN prediction_sample_size     BIGINT,
    ADD COLUMN cold_start_layer_used      TEXT,

    -- === Run-level projection columns (3 total per §2.2) ===
    ADD COLUMN run_projection_at_decision_atomic NUMERIC(38,0),
    ADD COLUMN run_predicted_remaining_steps     INT,
    ADD COLUMN run_steps_completed_so_far        BIGINT,

    -- === Commit-side actual columns (4 total per §2.3) ===
    ADD COLUMN actual_input_tokens   BIGINT,
    ADD COLUMN actual_output_tokens  BIGINT,
    ADD COLUMN delta_b_ratio         REAL,
    ADD COLUMN delta_c_ratio         REAL;

-- ============================================================================
-- Step 2: Domain CHECK constraints (per spec §4.1 verbatim + round-2
-- additions M2 / M3 / M5 / M12).
--
-- All constraints declared NOT VALID first then VALIDATE'd in a second
-- pass (round-2 fix M6 / M18): keeps lock escalation predictable on
-- production partitioned tables. Eager validation is safe here because
-- the table has no rows that violate any constraint (all NULL on legacy
-- rows), but the two-step form is the deployment-safe pattern for the
-- 2026-08+ partitions pre-created in step 5 below — those WILL accept
-- rows once SLICE_06 producers land, so future re-runs against extant
-- data benefit from the same NOT VALID + VALIDATE pattern.
--
-- On the partitioned parent table the constraint applies to every child
-- partition automatically.
-- ============================================================================

ALTER TABLE audit_outbox
    -- Enum-string domain checks (mirror of v1alpha1 §4.1).
    ADD CONSTRAINT audit_outbox_reserved_strategy_chk
        CHECK (reserved_strategy IS NULL OR reserved_strategy IN ('A','B','C'))
        NOT VALID,
    ADD CONSTRAINT audit_outbox_prediction_strategy_used_chk
        CHECK (prediction_strategy_used IS NULL
               OR prediction_strategy_used IN ('A','B','C'))
        NOT VALID,
    ADD CONSTRAINT audit_outbox_prediction_policy_used_chk
        CHECK (prediction_policy_used IS NULL OR prediction_policy_used IN (
            'STRICT_CEILING','EMPIRICAL_RUN_CEILING',
            'ADAPTIVE_CEILING','SHADOW_ONLY'))
        NOT VALID,
    ADD CONSTRAINT audit_outbox_tokenizer_tier_chk
        CHECK (tokenizer_tier IS NULL OR tokenizer_tier IN ('T1','T2','T3'))
        NOT VALID,
    ADD CONSTRAINT audit_outbox_prediction_confidence_chk
        CHECK (prediction_confidence IS NULL
               OR (prediction_confidence >= 0.000
                   AND prediction_confidence <= 1.000))
        NOT VALID,
    ADD CONSTRAINT audit_outbox_cold_start_layer_used_chk
        CHECK (cold_start_layer_used IS NULL
               OR cold_start_layer_used IN ('L1','L2','L3','L4'))
        NOT VALID,

    -- === Sentinel discipline (round-2 fix M3, per spec §3.3 + §6.3) ===
    -- Token counts non-negative. NULL allowed because the column is
    -- nullable; the sentinel discipline only constrains populated rows.
    ADD CONSTRAINT audit_outbox_predicted_tokens_chk
        CHECK ((predicted_a_tokens IS NULL OR predicted_a_tokens >= 0)
           AND (predicted_b_tokens IS NULL OR predicted_b_tokens >= 0)
           AND (predicted_c_tokens IS NULL OR predicted_c_tokens >= 0))
        NOT VALID,
    ADD CONSTRAINT audit_outbox_actual_tokens_chk
        CHECK ((actual_input_tokens IS NULL OR actual_input_tokens >= 0)
           AND (actual_output_tokens IS NULL OR actual_output_tokens >= 0))
        NOT VALID,
    -- run_predicted_remaining_steps uses -1 as a sentinel for
    -- "projector unreachable" per spec §3.3; legal range is therefore
    -- [-1, ∞). run_steps_completed_so_far is a counter, [0, ∞).
    ADD CONSTRAINT audit_outbox_run_steps_chk
        CHECK ((run_predicted_remaining_steps IS NULL
                  OR run_predicted_remaining_steps >= -1)
           AND (run_steps_completed_so_far IS NULL
                  OR run_steps_completed_so_far >= 0))
        NOT VALID,
    -- NUMERIC(38,0) field is non-negative AND must not exceed int64 max
    -- (round-2 fix M5) — the CloudEvent proto mirror is int64 per
    -- common.proto:367; values beyond 2^63-1 would silently round-trip
    -- to negative on the wire.
    ADD CONSTRAINT audit_outbox_run_projection_chk
        CHECK (run_projection_at_decision_atomic IS NULL
               OR run_projection_at_decision_atomic >= 0)
        NOT VALID,
    ADD CONSTRAINT audit_outbox_run_projection_int64_chk
        CHECK (run_projection_at_decision_atomic IS NULL
               OR run_projection_at_decision_atomic <= 9223372036854775807)
        NOT VALID,
    -- prediction_sample_size is a sample count, [0, ∞).
    ADD CONSTRAINT audit_outbox_prediction_sample_size_chk
        CHECK (prediction_sample_size IS NULL OR prediction_sample_size >= 0)
        NOT VALID,
    -- delta_*_ratio: non-negative AND NaN-reject. The
    -- `delta_b_ratio = delta_b_ratio` clause rejects NaN per IEEE 754
    -- (NaN never equals itself), keeping calibration-report aggregations
    -- well-defined.
    ADD CONSTRAINT audit_outbox_delta_b_ratio_chk
        CHECK (delta_b_ratio IS NULL
               OR (delta_b_ratio >= 0.0 AND delta_b_ratio = delta_b_ratio))
        NOT VALID,
    ADD CONSTRAINT audit_outbox_delta_c_ratio_chk
        CHECK (delta_c_ratio IS NULL
               OR (delta_c_ratio >= 0.0 AND delta_c_ratio = delta_c_ratio))
        NOT VALID;

-- VALIDATE pass — runs immediately because the table is empty of rows
-- that could violate. On a future re-run on populated production the
-- two-step form keeps the share-lock window short (NOT VALID takes
-- short-duration ACCESS EXCLUSIVE; VALIDATE upgrades to SHARE UPDATE
-- EXCLUSIVE and scans without blocking writers).
ALTER TABLE audit_outbox VALIDATE CONSTRAINT audit_outbox_reserved_strategy_chk;
ALTER TABLE audit_outbox VALIDATE CONSTRAINT audit_outbox_prediction_strategy_used_chk;
ALTER TABLE audit_outbox VALIDATE CONSTRAINT audit_outbox_prediction_policy_used_chk;
ALTER TABLE audit_outbox VALIDATE CONSTRAINT audit_outbox_tokenizer_tier_chk;
ALTER TABLE audit_outbox VALIDATE CONSTRAINT audit_outbox_prediction_confidence_chk;
ALTER TABLE audit_outbox VALIDATE CONSTRAINT audit_outbox_cold_start_layer_used_chk;
ALTER TABLE audit_outbox VALIDATE CONSTRAINT audit_outbox_predicted_tokens_chk;
ALTER TABLE audit_outbox VALIDATE CONSTRAINT audit_outbox_actual_tokens_chk;
ALTER TABLE audit_outbox VALIDATE CONSTRAINT audit_outbox_run_steps_chk;
ALTER TABLE audit_outbox VALIDATE CONSTRAINT audit_outbox_run_projection_chk;
ALTER TABLE audit_outbox VALIDATE CONSTRAINT audit_outbox_run_projection_int64_chk;
ALTER TABLE audit_outbox VALIDATE CONSTRAINT audit_outbox_prediction_sample_size_chk;
ALTER TABLE audit_outbox VALIDATE CONSTRAINT audit_outbox_delta_b_ratio_chk;
ALTER TABLE audit_outbox VALIDATE CONSTRAINT audit_outbox_delta_c_ratio_chk;

-- ============================================================================
-- Step 3: Partial NOT-NULL via CHECK on event_type (round-2 fix M2,
-- per spec §2.1-§2.3 "Nullable: NO" columns).
--
-- Spec §2.1 lists 7 columns as "Nullable: NO" on .decision events:
--   predicted_a_tokens, reserved_strategy, prediction_strategy_used,
--   prediction_policy_used, tokenizer_tier,
--   run_projection_at_decision_atomic, run_steps_completed_so_far
-- Spec §2.3 lists 2 columns as "Nullable: NO" on .outcome events:
--   actual_input_tokens, actual_output_tokens
--
-- These cannot be enforced as SQL `NOT NULL` because the columns are
-- nullable on the OTHER event_type. Instead we enforce them as
-- event-type-scoped CHECK constraints.
--
-- Round-3 fix B5: cutoff extended from 2026-07-01 to 2027-01-01. The
-- 2026-07 cutoff was a calendar bomb — if SLICE_06 slipped past it,
-- every audit.decision INSERT would start failing the CHECK without any
-- code change to surface the deadline. 2027-01-01 gives SLICE_06+
-- producer slices ample runway; SLICE_06 deployment plan MUST land
-- before then. Recommended ops practice: schedule a recurring calendar
-- reminder + GitHub issue at 2026-10-01 to verify SLICE_06 status.
--
-- Round-4 fix B4: DROP CONSTRAINT IF EXISTS prepended so the cutoff
-- tweaks in B5 (2026-07-01 → 2027-01-01) don't error on re-application
-- against a database that previously ran the round-2 form. Same
-- constraint name + different CHECK body would either fail with 42710
-- (duplicate object) or silently keep the old body. The drop-then-add
-- pair runs inside the migration-runner transaction so the constraint
-- is never temporarily absent from an observer's perspective.
-- ============================================================================

ALTER TABLE audit_outbox
    DROP CONSTRAINT IF EXISTS audit_outbox_decision_required_cols_chk,
    DROP CONSTRAINT IF EXISTS audit_outbox_outcome_required_cols_chk;

ALTER TABLE audit_outbox
    ADD CONSTRAINT audit_outbox_decision_required_cols_chk
        CHECK (event_type <> 'spendguard.audit.decision'
               OR recorded_at < '2027-01-01'::timestamptz
               OR (predicted_a_tokens IS NOT NULL
                   AND reserved_strategy IS NOT NULL
                   AND prediction_strategy_used IS NOT NULL
                   AND prediction_policy_used IS NOT NULL
                   AND tokenizer_tier IS NOT NULL
                   AND run_projection_at_decision_atomic IS NOT NULL
                   AND run_steps_completed_so_far IS NOT NULL))
        NOT VALID,
    ADD CONSTRAINT audit_outbox_outcome_required_cols_chk
        CHECK (event_type <> 'spendguard.audit.outcome'
               OR recorded_at < '2027-01-01'::timestamptz
               OR (actual_input_tokens IS NOT NULL
                   AND actual_output_tokens IS NOT NULL))
        NOT VALID;

ALTER TABLE audit_outbox VALIDATE CONSTRAINT audit_outbox_decision_required_cols_chk;
ALTER TABLE audit_outbox VALIDATE CONSTRAINT audit_outbox_outcome_required_cols_chk;

-- ============================================================================
-- Step 3a: Outcome-side cold_start_layer_used must be NULL (round-4 fix M3).
--
-- Per spec §2.1 cold_start_layer_used describes the cold-start fallback
-- layer the decision-time predictor used; it is meaningless on outcome
-- events. Without this CHECK an outcome row could carry a populated
-- value, masking calibration-report aggregations that assume
-- "WHERE event_type = '...decision' AND cold_start_layer_used IS NOT NULL"
-- has full coverage.
-- ============================================================================

ALTER TABLE audit_outbox
    DROP CONSTRAINT IF EXISTS audit_outbox_cold_start_layer_outcome_chk;

ALTER TABLE audit_outbox
    ADD CONSTRAINT audit_outbox_cold_start_layer_outcome_chk
        CHECK (event_type <> 'spendguard.audit.outcome'
               OR cold_start_layer_used IS NULL)
        NOT VALID;

ALTER TABLE audit_outbox VALIDATE CONSTRAINT audit_outbox_cold_start_layer_outcome_chk;

-- ============================================================================
-- Step 3b: Sentinel-collision guards (round-3 fix M13; round-4 fix B4
-- idempotent re-application).
--
-- The proto3 sentinel mapping (`spendguard-prediction-mirror` crate, spec
-- §6.3) maps SQL NULL ↔ proto-default 0 for token-count fields. This
-- creates a collision: a populated row with `predicted_b_tokens = 0`
-- (e.g., misconfigured Strategy B emitted 0 tokens predicted) would be
-- indistinguishable from "Strategy B was null at decision time" once
-- re-encoded.
--
-- Closing the collision at the SQL boundary: ban 0 token-count values on
-- rows that explicitly populate the strategy column. The producer-side
-- precondition is documented in
-- crates/spendguard-prediction-mirror/src/lib.rs preamble.
--
-- predicted_a_tokens is constrained globally on .decision rows because
-- the Strategy A reservation has no semantic interpretation for 0
-- (the ceiling is always > 0).
-- predicted_b_tokens and predicted_c_tokens are constrained only on
-- rows where prediction_strategy_used is B or C respectively — the
-- value semantics are strategy-conditional.
--
-- Round-4 fix B4: DROP CONSTRAINT IF EXISTS prepended so the
-- 2027-01-01 cutoff body matches the round-3 decision-required CHECK;
-- same reason as Step 3.
-- ============================================================================

ALTER TABLE audit_outbox
    DROP CONSTRAINT IF EXISTS audit_outbox_predicted_a_tokens_nonzero_chk,
    DROP CONSTRAINT IF EXISTS audit_outbox_predicted_b_tokens_nonzero_chk,
    DROP CONSTRAINT IF EXISTS audit_outbox_predicted_c_tokens_nonzero_chk;

ALTER TABLE audit_outbox
    ADD CONSTRAINT audit_outbox_predicted_a_tokens_nonzero_chk
        CHECK (event_type <> 'spendguard.audit.decision'
               OR recorded_at < '2027-01-01'::timestamptz
               OR predicted_a_tokens IS NULL
               OR predicted_a_tokens > 0)
        NOT VALID,
    ADD CONSTRAINT audit_outbox_predicted_b_tokens_nonzero_chk
        CHECK (prediction_strategy_used IS DISTINCT FROM 'B'
               OR predicted_b_tokens IS NULL
               OR predicted_b_tokens > 0)
        NOT VALID,
    ADD CONSTRAINT audit_outbox_predicted_c_tokens_nonzero_chk
        CHECK (prediction_strategy_used IS DISTINCT FROM 'C'
               OR predicted_c_tokens IS NULL
               OR predicted_c_tokens > 0)
        NOT VALID;

ALTER TABLE audit_outbox VALIDATE CONSTRAINT audit_outbox_predicted_a_tokens_nonzero_chk;
ALTER TABLE audit_outbox VALIDATE CONSTRAINT audit_outbox_predicted_b_tokens_nonzero_chk;
ALTER TABLE audit_outbox VALIDATE CONSTRAINT audit_outbox_predicted_c_tokens_nonzero_chk;

-- ============================================================================
-- Step 4: Calibration-report indexes (per spec §4.1, round-2 fix M9
-- column order + outcome-side covering index).
--
-- Round-1 led the composite key with recorded_month (a low-cardinality
-- DATE column truncated to the first day of the month). Postgres will
-- happily use such an index, but on a multi-tenant cluster the leading
-- column should be tenant_id so per-tenant queries can use an
-- index-only scan without a Bitmap Heap pass. recorded_month moves to
-- the second key — calibration-report's monthly aggregation still gets
-- a tight range scan inside the per-tenant slice.
--
-- audit_outbox_outcome_calibration_idx (NEW in round-2): the outcome
-- side was previously uncovered. The aggregation calibration-report
-- runs is "delta_b_ratio + delta_c_ratio + actual_output_tokens per
-- (tenant, month, strategy)" — this index covers it with INCLUDE so
-- the table heap is not touched on the hot path.
--
-- Partial-indexes scoped to (event_type = '...') keep them small —
-- outcome / decision rows do not populate each other's columns. All
-- defined on the partitioned parent; Postgres applies them per-partition.
-- ============================================================================

CREATE INDEX audit_outbox_calibration_idx
    ON audit_outbox (tenant_id, recorded_month,
                     prediction_strategy_used, prediction_policy_used)
    WHERE event_type = 'spendguard.audit.decision';

CREATE INDEX audit_outbox_tier_idx
    ON audit_outbox (tenant_id, recorded_month, tokenizer_tier)
    WHERE event_type = 'spendguard.audit.decision';

-- Round-4 fix M14: WHERE clause relaxed from
--   WHERE event_type = '...outcome'
--     AND (delta_b_ratio IS NOT NULL OR delta_c_ratio IS NOT NULL)
-- to just `WHERE event_type = '...outcome'`. Per PostgreSQL docs the
-- planner only considers a partial index when the query WHERE implies
-- the index predicate. If SLICE_13's calibration-report omits the
-- `OR delta_b_ratio IS NOT NULL OR delta_c_ratio IS NOT NULL` clause
-- — likely, because the natural query is "all outcomes per
-- (tenant, month, strategy)" — the planner would skip this index even
-- though it covers the requested columns.
--
-- Cost of relaxation: the index grows by however many outcome rows
-- have both delta_*_ratio NULL. Per spec §6.3 this corresponds to
-- Strategy A outcomes (predictions B/C were null at decision time so
-- ratios are NULL). On a fresh cluster with 100% Strategy A traffic
-- the index doubles in size; on a tenant with mixed strategies the
-- delta is smaller. The PG planner-friendliness wins because the
-- alternative is a sequential heap scan on every calibration-report
-- query that forgets the IS NOT NULL clause.
--
-- DROP INDEX IF EXISTS prepended so re-application against a database
-- that already has the round-3 form (the strict WHERE) replaces it
-- cleanly. CREATE INDEX would otherwise error with "relation already
-- exists" because the index name is unchanged.
DROP INDEX IF EXISTS audit_outbox_outcome_calibration_idx;
CREATE INDEX audit_outbox_outcome_calibration_idx
    ON audit_outbox (tenant_id, recorded_month, prediction_strategy_used)
    INCLUDE (delta_b_ratio, delta_c_ratio, actual_output_tokens)
    WHERE event_type = 'spendguard.audit.outcome';

-- Round-3 fix M7: partial index supporting the
-- audit_outbox_tokenizer_version_id_fk constraint declared in 0048.
-- Without this index, the FK's RESTRICT semantics would force a
-- sequential scan of audit_outbox on every DELETE from tokenizer_versions
-- (even though the delete is blocked by the immutability trigger, the
-- planner still checks). Partial-WHERE strips the all-NULL legacy rows.
CREATE INDEX audit_outbox_tokenizer_version_id_idx
    ON audit_outbox (tokenizer_version_id)
    WHERE tokenizer_version_id IS NOT NULL;

-- ============================================================================
-- Step 5: Immutability trigger update (round-2 fix M7 — merged from
-- the old 0047 to close the trigger-gap window).
--
-- Round-3 fix M14: this step now precedes partition pre-creation
-- (formerly step 5, now step 7). Reason: under partial-apply where
-- migration runner crashes between steps 5 and 6, the pre-created
-- partitions would inherit the OLD trigger function. Sequencing the
-- function update before the partition create ensures any partition —
-- pre-created here or auto-generated later by a partition-manager
-- service — sees the new 18-column tuple compare from creation time.
--
-- Spec: docs/audit-chain-prediction-extension-v1alpha1.md §5 (critical
-- surface) and §5.2 verbatim trigger body.
--
-- Critical risk this closes (HANDOFF Step 4 discrepancy #4): the
-- original trigger function (migration 0011) only compares the 14 base
-- columns. Without this update, the 18 new columns are silently mutable
-- by any UPDATE statement — DBA / forwarder ORM / attacker could
-- rewrite calibration evidence after INSERT and `verify-chain` would
-- still see the signature match if mirror was tampered alongside (only
-- the mirror cross-check from §11.2 would catch it). With this update,
-- any UPDATE to a prediction column raises Postgres errcode 42P10.
--
-- The function is CREATE OR REPLACE — no trigger DROP, no audit
-- downtime. The existing audit_outbox_immutability trigger continues to
-- fire BEFORE UPDATE; it just dispatches to the new function body.
--
-- Forwarder-state columns (pending_forward, forwarded_at,
-- forward_attempts, last_forward_error) remain UPDATE-able — they are
-- intentionally excluded from both OLD and NEW tuples, so a forwarder
-- UPDATE that only touches those four columns produces tuples that are
-- still equal under IS DISTINCT FROM, and the trigger passes silently.
-- See audit-chain-prediction-extension-v1alpha1.md §5.3.
-- ============================================================================

CREATE OR REPLACE FUNCTION reject_audit_outbox_immutable_columns()
RETURNS TRIGGER
SECURITY INVOKER
SET search_path = pg_catalog, pg_temp
AS $$
BEGIN
    IF (OLD.audit_outbox_id, OLD.audit_decision_event_id, OLD.decision_id,
        OLD.tenant_id, OLD.ledger_transaction_id, OLD.event_type,
        OLD.cloudevent_payload, OLD.cloudevent_payload_signature,
        OLD.ledger_fencing_epoch, OLD.workload_instance_id,
        OLD.recorded_at, OLD.recorded_month,
        OLD.producer_sequence, OLD.idempotency_key,
        -- === NEW prediction columns (per audit-chain-prediction-extension §5.2) ===
        OLD.predicted_a_tokens, OLD.predicted_b_tokens, OLD.predicted_c_tokens,
        OLD.reserved_strategy, OLD.prediction_strategy_used,
        OLD.prediction_policy_used, OLD.tokenizer_tier, OLD.tokenizer_version_id,
        OLD.prediction_confidence, OLD.prediction_sample_size,
        OLD.cold_start_layer_used,
        OLD.run_projection_at_decision_atomic,
        OLD.run_predicted_remaining_steps,
        OLD.run_steps_completed_so_far,
        OLD.actual_input_tokens, OLD.actual_output_tokens,
        OLD.delta_b_ratio, OLD.delta_c_ratio)
       IS DISTINCT FROM
       (NEW.audit_outbox_id, NEW.audit_decision_event_id, NEW.decision_id,
        NEW.tenant_id, NEW.ledger_transaction_id, NEW.event_type,
        NEW.cloudevent_payload, NEW.cloudevent_payload_signature,
        NEW.ledger_fencing_epoch, NEW.workload_instance_id,
        NEW.recorded_at, NEW.recorded_month,
        NEW.producer_sequence, NEW.idempotency_key,
        NEW.predicted_a_tokens, NEW.predicted_b_tokens, NEW.predicted_c_tokens,
        NEW.reserved_strategy, NEW.prediction_strategy_used,
        NEW.prediction_policy_used, NEW.tokenizer_tier, NEW.tokenizer_version_id,
        NEW.prediction_confidence, NEW.prediction_sample_size,
        NEW.cold_start_layer_used,
        NEW.run_projection_at_decision_atomic,
        NEW.run_predicted_remaining_steps,
        NEW.run_steps_completed_so_far,
        NEW.actual_input_tokens, NEW.actual_output_tokens,
        NEW.delta_b_ratio, NEW.delta_c_ratio) THEN
        RAISE EXCEPTION 'audit_outbox immutable columns cannot be changed (incl. prediction extension cols)'
            USING ERRCODE = '42P10';
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

-- ============================================================================
-- Step 6: TRUNCATE guard (round-2 fix M13). Uses the new generic
-- reject_truncate_on_immutable_table() function (round-3 fix M6) instead
-- of the misleading reject_immutable_ledger_entry_mutation(). The new
-- function reads TG_TABLE_NAME so the error message correctly names
-- audit_outbox (not ledger_entries).
-- ============================================================================

-- Round-3 fix M6 + round-4 fix B5: generic TRUNCATE-rejector function.
-- Reads TG_TABLE_NAME so it can be reused for tokenizer_versions and any
-- future immutable table without duplicating "TRUNCATE on ledger_entries
-- forbidden" error messages on tables that are not ledger_entries.
--
-- SECURITY INVOKER + SET search_path = pg_catalog, pg_temp closes
-- CVE-2018-1058: without the explicit search_path, an attacker with
-- CREATE privilege on any schema in the caller's search_path could
-- shadow `pg_catalog.RAISE` (or any built-in) with a malicious function
-- and have it executed under the table-owner's role when the trigger
-- fires. Locking the function's search_path to pg_catalog, pg_temp
-- forces unqualified references to resolve only to built-ins.
--
-- CONVENTION FOR FUTURE FUNCTION ADDITIONS IN THIS MIGRATION FAMILY:
-- every CREATE OR REPLACE FUNCTION in 0046+ MUST include the same
-- SECURITY INVOKER + SET search_path = pg_catalog, pg_temp lockdown
-- so the pattern is uniform and reviewers can grep for the absence.
CREATE OR REPLACE FUNCTION reject_truncate_on_immutable_table()
RETURNS TRIGGER
SECURITY INVOKER
SET search_path = pg_catalog, pg_temp
AS $$
BEGIN
    RAISE EXCEPTION 'TRUNCATE forbidden on immutable table %', TG_TABLE_NAME
        USING ERRCODE = '42501';
    RETURN NULL;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER audit_outbox_no_truncate
    BEFORE TRUNCATE ON audit_outbox
    FOR EACH STATEMENT
    EXECUTE FUNCTION reject_truncate_on_immutable_table();

-- ============================================================================
-- Step 7: Pre-create future partitions through 2026-10 (round-2 fix M14,
-- moved here in round-3 fix M14 — see header rationale). Baseline 0009
-- only pre-created up to 2026-07; without these partitions the table
-- would fall through to audit_outbox_default and raise an ops alert as
-- SLICE_06 producers begin writing in 2026-08+. Postgres ranges are
-- half-open; '2026-08-01' to '2026-09-01' captures August 2026 inclusive.
--
-- Sequencing: this step runs AFTER step 5 (function update) and step 6
-- (TRUNCATE guard) so that any partition created here inherits the new
-- 18-column trigger semantics and the TRUNCATE guard. The parent table's
-- trigger applies to all child partitions automatically — Postgres
-- propagates BEFORE-row triggers on the parent to children, so partition
-- order is not load-bearing for correctness, only for ops safety under
-- partial-apply.
-- ============================================================================

CREATE TABLE audit_outbox_2026_08 PARTITION OF audit_outbox
    FOR VALUES FROM ('2026-08-01') TO ('2026-09-01');
CREATE TABLE audit_outbox_2026_09 PARTITION OF audit_outbox
    FOR VALUES FROM ('2026-09-01') TO ('2026-10-01');
CREATE TABLE audit_outbox_2026_10 PARTITION OF audit_outbox
    FOR VALUES FROM ('2026-10-01') TO ('2026-11-01');

-- ============================================================================
-- Step 8: Column comments. Kept verbose because SLICE_06+ producers will
-- consult these via `\d+ audit_outbox` when wiring up the mirror logic
-- and the sentinel discipline is non-obvious from column names alone.
-- ============================================================================

COMMENT ON COLUMN audit_outbox.predicted_a_tokens IS
    'Strategy A token ceiling at decision time (always populated on .decision events; BIGINT for multi-year aggregation headroom). Per audit-chain-prediction-extension-v1alpha1.md §2.1.';
COMMENT ON COLUMN audit_outbox.predicted_b_tokens IS
    'Strategy B (empirical) prediction; NULL when sample bucket < 30. §2.1.';
COMMENT ON COLUMN audit_outbox.predicted_c_tokens IS
    'Strategy C (customer plugin) prediction; NULL when plugin unconfigured / failed / fallback. §2.1.';
COMMENT ON COLUMN audit_outbox.reserved_strategy IS
    'Strategy actually used to size the reservation (A/B/C). §2.1.';
COMMENT ON COLUMN audit_outbox.prediction_strategy_used IS
    'Strategy the predictor recommended (may differ from reserved_strategy under STRICT_CEILING). §2.1.';
COMMENT ON COLUMN audit_outbox.prediction_policy_used IS
    'Contract policy class governing this decision (STRICT_CEILING / EMPIRICAL_RUN_CEILING / ADAPTIVE_CEILING / SHADOW_ONLY). §2.1.';
COMMENT ON COLUMN audit_outbox.tokenizer_tier IS
    'Tokenizer tier that produced the input token count (T1/T2/T3). §2.1.';
COMMENT ON COLUMN audit_outbox.tokenizer_version_id IS
    'FK to tokenizer_versions(tokenizer_version_id). NULL on Tier 3 fallback. §2.1.';
COMMENT ON COLUMN audit_outbox.prediction_confidence IS
    'Predictor confidence for Strategy B/C (NUMERIC(4,3) range 0.000-1.000 for deterministic AVG semantics — round-2 fix M12); NULL for Strategy A. CloudEvent proto field 308 mirrors as float with absent = column-NULL on Strategy A row (round-2 fix M11; see audit-chain-prediction-extension-v1alpha1.md §6.3). §2.1.';
COMMENT ON COLUMN audit_outbox.prediction_sample_size IS
    'Sample count behind Strategy B/C; NULL for cold-start / A. BIGINT per round-2 fix M4 for multi-year aggregation headroom. §2.1.';
COMMENT ON COLUMN audit_outbox.cold_start_layer_used IS
    'Cold-start fallback layer (L1-L4) when B/C fell through; NULL when warm. Promoted from metadata to first-class per §2.4 reviewer note. §2.1.';
COMMENT ON COLUMN audit_outbox.run_projection_at_decision_atomic IS
    'Per-run projected cumulative cost (NUMERIC(38,0)) at decision time. Constrained to <= int64 max (round-2 fix M5) so the CloudEvent proto int64 mirror at tag 311 round-trips losslessly. §2.2.';
COMMENT ON COLUMN audit_outbox.run_predicted_remaining_steps IS
    'Predicted remaining run steps; NULL when run_cost_projector unreachable. Wire-level mirror at proto tag 312 uses -1 sentinel for unreachable; SQL side keeps NULL. §2.2.';
COMMENT ON COLUMN audit_outbox.run_steps_completed_so_far IS
    'Step counter from sidecar in-process state cache. BIGINT per round-2 fix M4. §2.2.';
COMMENT ON COLUMN audit_outbox.actual_input_tokens IS
    'Provider-reported input tokens at commit_estimated / provider_report time. §2.3.';
COMMENT ON COLUMN audit_outbox.actual_output_tokens IS
    'Provider-reported output tokens at commit_estimated / provider_report time. §2.3.';
COMMENT ON COLUMN audit_outbox.delta_b_ratio IS
    'actual_output_tokens / predicted_b_tokens; NULL when prediction B was null at decision time. CHECK guards NaN per IEEE 754 (round-2 fix M3). §2.3.';
COMMENT ON COLUMN audit_outbox.delta_c_ratio IS
    'actual_output_tokens / predicted_c_tokens; NULL when prediction C was null at decision time. CHECK guards NaN per IEEE 754 (round-2 fix M3). §2.3.';
