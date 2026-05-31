-- GA_05: support the outbox-forwarder audit lag gauge.
--
-- The dashboard lag query asks for the oldest pending audit_outbox row across
-- partitions:
--
--   SELECT MIN(recorded_at)
--     FROM audit_outbox
--    WHERE pending_forward = TRUE;
--
-- The original forwarder index begins with recorded_month, which is ideal for
-- batched replay ordering but does not support the cross-partition MIN query.
-- This partial index keeps the GA_05 metrics poll cheap even when every
-- outbox-forwarder pod refreshes the gauge.

CREATE INDEX IF NOT EXISTS audit_outbox_pending_age_idx
    ON audit_outbox (recorded_at)
    WHERE pending_forward = TRUE;

DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1
          FROM pg_indexes
         WHERE schemaname = 'public'
           AND indexname = 'audit_outbox_pending_age_idx'
           AND indexdef LIKE '%(recorded_at)%'
           AND indexdef LIKE '%pending_forward = true%'
    ) THEN
        RAISE EXCEPTION 'audit_outbox_pending_age_idx missing after GA_05 migration';
    END IF;
END $$;
