-- ============================================================================
-- 0024_event_replay_dedup_reaper.sql
--
-- Bounded growth for canonical_event_replay_dedup (added in 0020).
--
-- 0020 defined `expires_at TIMESTAMPTZ NOT NULL` plus the index
-- `canonical_event_replay_dedup_expires_idx (expires_at)` and a comment
-- ("Replay horizon expiry used by cleanup jobs."), but no cleanup job
-- ever shipped. claim_replay_key inserts one row per (producer_id,
-- event_id) forever and nothing prunes them, so the hot append path's
-- replay ledger grows without bound — an availability/cost problem (the
-- replay protection itself stays correct, even stronger).
--
-- This migration adds the missing reaper as a SECURITY DEFINER function
-- so the application role can prune without holding raw DELETE on the
-- table (least privilege: 0020 granted only SELECT/INSERT/UPDATE). It is
-- additive only — no change to the 0020 schema, indexes, or GRANTs.
--
-- Invocation: operators schedule it (pg_cron, or the ingest service's
-- maintenance loop) e.g.
--   SELECT reap_canonical_event_replay_dedup();            -- defaults
--   SELECT reap_canonical_event_replay_dedup('1 hour', 5000, 20);
-- ============================================================================

CREATE OR REPLACE FUNCTION reap_canonical_event_replay_dedup(
    p_grace        INTERVAL DEFAULT INTERVAL '15 minutes',
    p_batch_size   INTEGER  DEFAULT 5000,
    p_max_batches  INTEGER  DEFAULT 100
)
RETURNS BIGINT
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = pg_catalog, public, pg_temp
AS $$
DECLARE
    v_deleted_total BIGINT  := 0;
    v_deleted_batch BIGINT;
    v_cutoff        TIMESTAMPTZ;
    v_i             INTEGER;
BEGIN
    -- Reject nonsensical arguments rather than silently misbehaving.
    IF p_batch_size IS NULL OR p_batch_size <= 0 THEN
        RAISE EXCEPTION 'reap_canonical_event_replay_dedup: p_batch_size must be > 0';
    END IF;
    IF p_max_batches IS NULL OR p_max_batches <= 0 THEN
        RAISE EXCEPTION 'reap_canonical_event_replay_dedup: p_max_batches must be > 0';
    END IF;

    -- Grace margin: only prune rows whose horizon expired at least
    -- p_grace ago. This avoids racing a producer that is mid-claim
    -- right at the expiry boundary and keeps the replay window strictly
    -- conservative (we never drop a still-protective row early).
    v_cutoff := clock_timestamp() - COALESCE(p_grace, INTERVAL '0');

    -- Batched deletes keep each transaction short so the hot append path
    -- (claim_replay_key) is never blocked behind one giant DELETE. The
    -- LIMIT uses the (expires_at) index via the ctid subselect.
    --
    -- Preservation guarantees:
    --   * reservation_only rows (legacy quarantine backfills) are kept.
    --   * 'infinity' expires_at rows are kept (they never satisfy the
    --     `expires_at < v_cutoff` predicate, but the explicit guard makes
    --     the intent obvious and is defence in depth).
    FOR v_i IN 1..p_max_batches LOOP
        DELETE FROM canonical_event_replay_dedup
        WHERE ctid IN (
            SELECT ctid
            FROM canonical_event_replay_dedup
            WHERE expires_at < v_cutoff
              AND expires_at <> 'infinity'::TIMESTAMPTZ
              AND reservation_only = FALSE
            ORDER BY expires_at
            LIMIT p_batch_size
        );
        GET DIAGNOSTICS v_deleted_batch = ROW_COUNT;
        v_deleted_total := v_deleted_total + v_deleted_batch;
        EXIT WHEN v_deleted_batch < p_batch_size;
    END LOOP;

    RETURN v_deleted_total;
END;
$$;

COMMENT ON FUNCTION reap_canonical_event_replay_dedup(INTERVAL, INTEGER, INTEGER) IS
    'Prune expired canonical_event_replay_dedup rows in bounded batches. '
    'Keeps reservation_only and infinity rows (quarantine reservations stay '
    'reserved until canonical release / terminal orphan handling). Applies a '
    'grace margin so rows are only removed well after their replay horizon. '
    'Schedule via pg_cron or the ingest maintenance loop.';

-- Least privilege: the application role can invoke the reaper but still
-- holds no raw DELETE on the table (SECURITY DEFINER runs the DELETE as
-- the function owner). 0020 granted only SELECT/INSERT/UPDATE.
REVOKE EXECUTE ON FUNCTION reap_canonical_event_replay_dedup(INTERVAL, INTEGER, INTEGER) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION reap_canonical_event_replay_dedup(INTERVAL, INTEGER, INTEGER)
    TO canonical_ingest_application_role;

-- Smoke check: the function exists and is callable.
DO $$
BEGIN
    SET LOCAL search_path = pg_catalog, pg_temp;
    PERFORM 1
      FROM pg_proc p
      JOIN pg_namespace n ON n.oid = p.pronamespace
     WHERE n.nspname = 'public'
       AND p.proname = 'reap_canonical_event_replay_dedup';
    IF NOT FOUND THEN
        RAISE EXCEPTION 'reap_canonical_event_replay_dedup function missing';
    END IF;
END $$;
