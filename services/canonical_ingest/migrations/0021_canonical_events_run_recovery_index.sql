-- ============================================================================
-- 0021_canonical_events_run_recovery_index.sql — GA_08 run_cost_projector
-- cold-cache recovery access path.
--
-- run_cost_projector reads canonical_events (not ledger.audit_outbox) when a
-- run state cache entry is missing. The lookup shape is:
--
--   tenant_id = ?
--   event_type = 'spendguard.audit.decision'
--   run_id_mirror = ?
--   agent_id = ?
--   recorded_month >= replay-window month floor
--   ingest_at >= replay-window timestamp floor
--   ORDER BY producer_sequence DESC
--   LIMIT 1
--
-- The existing aggregator index is keyed for GROUP BY scans
-- (tenant_id, recorded_month, agent_id, run_id_mirror), so a recorded_month
-- range prevents a direct seek to one run. This index is intentionally
-- recovery-specific and partial to keep write amplification bounded.
-- ============================================================================

CREATE INDEX canonical_events_run_recovery_idx
    ON canonical_events (tenant_id, run_id_mirror, agent_id, producer_sequence DESC)
    INCLUDE (
        recorded_month,
        ingest_at,
        run_steps_completed_so_far,
        run_projection_at_decision_atomic
    )
    WHERE event_type = 'spendguard.audit.decision'
      AND run_id_mirror IS NOT NULL
      AND agent_id IS NOT NULL;

COMMENT ON INDEX canonical_events_run_recovery_idx IS
    'GA_08 recovery path for run_cost_projector cold cache miss: seek latest decision row by tenant/run/agent without scanning tenant-month decision volume.';

DO $$
BEGIN
    SET LOCAL search_path = pg_catalog, pg_temp;
    PERFORM 1 FROM pg_indexes
     WHERE schemaname = 'public'
       AND indexname = 'canonical_events_run_recovery_idx';
    IF NOT FOUND THEN
        RAISE EXCEPTION 'canonical_events_run_recovery_idx missing';
    END IF;
END $$;
