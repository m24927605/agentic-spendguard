-- ============================================================================
-- 0065_tokenizer_t1_samples_self_extending_partitions.sql
--
-- Eliminate the tokenizer_t1_samples partition cliff.
--
-- ## Problem
--
-- 0051_tokenizer_t1_samples.sql pre-created only three monthly partitions
-- (2026-05, 2026-06, 2026-07), covering through `2026-08-01 00:00:00+00`.
-- The table has NO DEFAULT partition by design (a missing-month INSERT must
-- raise `no partition of relation` and fail loud rather than silently route
-- into a catch-all that defeats the `DROP TABLE tokenizer_t1_samples_YYYYMM`
-- retention model — see 0051 lines 131-151). 0051 documented the forward-
-- partition creation as an operator obligation / deferred SLICE-extra cron.
--
-- Consequence: after `2026-08-01`, every Tier-1 shadow-worker INSERT FAILS
-- with a partition-routing error (cardinality_violation 23514). The prior
-- pass hardened the worker to fail-closed on this (suppress the drift alert,
-- tick spendguard_tokenizer_shadow_sample_insert_failed_total) but the
-- runway itself remained a standing operational cliff.
--
-- ## Fix (additive — 0051 is NOT edited)
--
-- (a) Define `tokenizer_t1_samples_ensure_partition(target)`, an idempotent
--     forward-partition creator mirroring the EXISTING repo idiom in
--     `cost_findings_ensure_next_month_partition()` (0040 + 0042):
--       * cheap to_regclass() pre-check before lock acquisition,
--       * LOCK TABLE ... IN ACCESS EXCLUSIVE MODE then re-check after the
--         lock so concurrent callers cannot both `CREATE TABLE ... PARTITION
--         OF` the same month (codex r6 P1 on cost_findings),
--       * EXECUTE format('CREATE TABLE %I PARTITION OF ... FOR VALUES
--         FROM (%L) TO (%L)') with schema-qualified, identifier-quoted names,
--       * GRANT/REVOKE the SAME privilege set 0051 applied to the existing
--         monthly partitions so a freshly-minted partition inherits the
--         identical lock-down (partition-level grants do NOT auto-inherit
--         from the parent in Postgres 16).
--     Unlike cost_findings this table has no DEFAULT partition, so there is
--     NO drain branch — the SP is strictly simpler (create-if-absent only).
--     It is parameterized by a target timestamp/date so the same routine
--     serves both the bulk runway seed below AND the ongoing self-extension
--     call from the shadow worker.
--
-- (b) Pre-create a multi-year runway forward to 2031-01-01 by calling the SP
--     in a loop. This removes any NEAR cliff: even with zero ongoing wiring,
--     inserts succeed through the end of 2030.
--
-- (c) Ongoing self-extension: the Tier-1 shadow worker's SqlSamplePersister
--     calls `SELECT tokenizer_t1_samples_ensure_partition($1)` and retries
--     the INSERT once when a partition-routing failure is observed (see
--     services/tokenizer/src/shadow/persistence.rs). This mirrors how
--     cost_findings' SP is driven by its writer/worker path rather than a
--     standalone cron — the SP is the durable mechanism, the caller keeps it
--     ahead of the wallclock. The multi-year seed in (b) means this retry
--     path is effectively never exercised before 2031, but it guarantees the
--     cliff cannot reappear at the seed horizon.
--
-- ## Privilege boundary
--
-- The SP runs SECURITY DEFINER (owned by the migration superuser) so the
-- shadow worker — running as ledger_application_role, which has only
-- INSERT/DELETE/column-UPDATE on the table — can create a partition without
-- being granted DDL. EXECUTE is granted to ledger_application_role only;
-- PUBLIC EXECUTE is revoked. search_path is pinned (CVE-2018-1058) so a
-- malicious schema cannot shadow the catalog functions the SP calls.
--
-- ## Idempotency
--
-- Re-running the whole migration is safe: CREATE OR REPLACE FUNCTION is
-- idempotent, and the runway loop is built on the SP's own create-if-absent
-- semantics (to_regclass pre-check → RETURN NULL when the partition already
-- exists). No partition is dropped or recreated; existing data is untouched.
--
-- ## Migration runner note
--
-- psql autocommit per SLICE_01 R5 — each statement commits independently.
-- The DO-block runway seed is a single statement that commits as a unit; the
-- SP's per-partition CREATE TABLE runs inside that block's transaction.
-- ============================================================================

-- ----------------------------------------------------------------------------
-- (a) Idempotent forward-partition creator.
-- ----------------------------------------------------------------------------
--
-- `p_target` is any timestamp/date inside the desired month. The SP computes
-- the month boundaries, derives the canonical partition name
-- `tokenizer_t1_samples_YYYY_MM`, and creates it if absent. Returns the new
-- partition name, or NULL when it already existed (so callers can log
-- "created X" without false-positive noise on repeat invocations — same
-- contract as cost_findings_ensure_next_month_partition).
CREATE OR REPLACE FUNCTION tokenizer_t1_samples_ensure_partition(p_target TIMESTAMPTZ)
    RETURNS TEXT
    LANGUAGE plpgsql
    SECURITY DEFINER
    SET search_path = pg_catalog, pg_temp
    AS $$
DECLARE
    v_month_start TIMESTAMPTZ;
    v_month_end   TIMESTAMPTZ;
    v_part_name   TEXT;
BEGIN
    -- Normalize to the UTC month boundaries. The parent partition key is
    -- sampled_at TIMESTAMPTZ; 0051's bounds are expressed at +00, so we
    -- truncate in UTC to keep partition edges aligned with calendar months
    -- regardless of the session TimeZone.
    v_month_start := date_trunc('month', p_target AT TIME ZONE 'UTC') AT TIME ZONE 'UTC';
    v_month_end   := v_month_start + INTERVAL '1 month';
    v_part_name   := 'tokenizer_t1_samples_' || to_char(v_month_start AT TIME ZONE 'UTC', 'YYYY_MM');

    -- Cheap pre-check: skip the ACCESS EXCLUSIVE lock if the partition is
    -- already present (mirrors cost_findings SP).
    IF to_regclass('public.' || v_part_name) IS NOT NULL THEN
        RETURN NULL;
    END IF;

    -- Concurrent callers can both pass the pre-check; the loser waits on the
    -- lock, then would hit a duplicate-table error. Recheck AFTER acquiring
    -- the lock (codex r6 P1 on cost_findings).
    LOCK TABLE public.tokenizer_t1_samples IN ACCESS EXCLUSIVE MODE;

    IF to_regclass('public.' || v_part_name) IS NOT NULL THEN
        -- Another concurrent caller won the race; nothing left to do.
        RETURN NULL;
    END IF;

    EXECUTE format(
        'CREATE TABLE public.%I PARTITION OF public.tokenizer_t1_samples '
        || 'FOR VALUES FROM (%L) TO (%L)',
        v_part_name, v_month_start, v_month_end
    );

    -- Replicate the 0051 + 0054 privilege lock-down on the new partition.
    -- Partition-level grants are NOT inherited from the parent in PG16, so a
    -- freshly-minted partition would otherwise default to PUBLIC's table
    -- privileges. Mirror exactly:
    --   * 0051: REVOKE INSERT/UPDATE/DELETE FROM PUBLIC;
    --           GRANT INSERT, DELETE TO ledger_application_role;
    --           GRANT UPDATE (drift_alert_emitted_at) TO ledger_application_role;
    --           GRANT SELECT TO ledger_reader_role;
    --   * 0054: REVOKE SELECT FROM PUBLIC.
    EXECUTE format('REVOKE INSERT, UPDATE, DELETE ON public.%I FROM PUBLIC', v_part_name);
    EXECUTE format('REVOKE SELECT ON public.%I FROM PUBLIC', v_part_name);
    EXECUTE format('GRANT INSERT, DELETE ON public.%I TO ledger_application_role', v_part_name);
    EXECUTE format(
        'GRANT UPDATE (drift_alert_emitted_at) ON public.%I TO ledger_application_role',
        v_part_name
    );
    EXECUTE format('GRANT SELECT ON public.%I TO ledger_reader_role', v_part_name);

    RETURN v_part_name;
END;
$$;

COMMENT ON FUNCTION tokenizer_t1_samples_ensure_partition(TIMESTAMPTZ) IS
    'Idempotent forward monthly-partition creator for tokenizer_t1_samples, mirroring cost_findings_ensure_next_month_partition (0040/0042). Creates tokenizer_t1_samples_YYYY_MM for the month containing p_target if absent and applies the 0051/0054 privilege lock-down; returns the new partition name or NULL when it already existed. SECURITY DEFINER so the shadow worker (ledger_application_role, no DDL) can self-extend the runway. Holds ACCESS EXCLUSIVE on the parent briefly during the rare create case; the post-lock recheck makes concurrent callers safe. No DEFAULT-partition drain (this table has no DEFAULT — missing partitions fail loud per 0051).';

-- Least-privilege EXECUTE: only the application role (the shadow worker)
-- needs to call this. PUBLIC must not be able to invoke a SECURITY DEFINER
-- DDL routine.
REVOKE EXECUTE ON FUNCTION tokenizer_t1_samples_ensure_partition(TIMESTAMPTZ) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION tokenizer_t1_samples_ensure_partition(TIMESTAMPTZ)
    TO ledger_application_role;

-- ----------------------------------------------------------------------------
-- (b) Pre-create the forward runway through 2031-01-01.
-- ----------------------------------------------------------------------------
--
-- 0051 already created 2026-05 / 2026-06 / 2026-07; the SP's create-if-absent
-- pre-check makes re-creating them a no-op, so we simply iterate every month
-- from 2026-05 to 2030-12 inclusive. This is ~56 months — well within a few
-- years of headroom — so there is no near cliff even with zero ongoing
-- wiring. The shadow worker's retry path (c) extends past 2031 if the service
-- somehow outlives this seed without a follow-up migration.
DO $$
DECLARE
    v_cursor TIMESTAMPTZ := TIMESTAMPTZ '2026-05-01 00:00:00+00';
    v_horizon TIMESTAMPTZ := TIMESTAMPTZ '2031-01-01 00:00:00+00';
BEGIN
    SET LOCAL search_path = pg_catalog, pg_temp;
    WHILE v_cursor < v_horizon LOOP
        PERFORM public.tokenizer_t1_samples_ensure_partition(v_cursor);
        v_cursor := v_cursor + INTERVAL '1 month';
    END LOOP;
END $$;

-- ----------------------------------------------------------------------------
-- Smoke assertion: prove the runway covers a date past the old 2026-08-01
-- cliff. A representative far-future month (2030-12) must now have a concrete
-- partition. Fails the migration loudly if the seed loop did not run.
-- ----------------------------------------------------------------------------
DO $$
BEGIN
    SET LOCAL search_path = pg_catalog, pg_temp;
    IF to_regclass('public.tokenizer_t1_samples_2026_08') IS NULL THEN
        RAISE EXCEPTION
            'tokenizer_t1_samples runway seed failed: 2026_08 partition (first month past the old cliff) is absent';
    END IF;
    IF to_regclass('public.tokenizer_t1_samples_2030_12') IS NULL THEN
        RAISE EXCEPTION
            'tokenizer_t1_samples runway seed failed: 2030_12 horizon partition is absent';
    END IF;
END $$;
