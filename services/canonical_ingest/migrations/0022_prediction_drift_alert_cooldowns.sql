-- ============================================================================
-- 0022_prediction_drift_alert_cooldowns.sql — POST_GA_06 drift alert dedup.
--
-- Issues:
--   - #157: 24h cooldown/dedup per (tenant, model, agent_id, prompt_class)
--   - #162: keep non-finite z-scores out of alert state/payloads
--
-- This is mutable derived state owned by stats_aggregator. It gates
-- prediction_drift_alert emission before a new immutable audit event is sent
-- to canonical_ingest, so repeated drift in the same bucket does not spam the
-- audit chain. It deliberately lives beside canonical_events because the
-- stats_aggregator already connects to the canonical DB for aggregation state.
-- ============================================================================

CREATE TABLE prediction_drift_alert_cooldowns (
    tenant_id       UUID        NOT NULL,
    model           TEXT        NOT NULL CHECK (char_length(model) BETWEEN 1 AND 64),
    agent_id        TEXT        NOT NULL CHECK (char_length(agent_id) BETWEEN 1 AND 128),
    prompt_class    TEXT        NOT NULL CHECK (prompt_class IN (
        'chat_short', 'chat_long', 'code_gen', 'summarization',
        'rag', 'tool_calling', 'vision'
    )),
    last_emitted_at TIMESTAMPTZ,
    suppress_until  TIMESTAMPTZ,
    last_z_score    REAL CHECK (
        last_z_score IS NULL OR (
        last_z_score <> 'NaN'::REAL
        AND last_z_score <> 'Infinity'::REAL
        AND last_z_score <> '-Infinity'::REAL
        )
    ),
    pending_event_id    UUID,
    pending_event_time  TIMESTAMPTZ,
    pending_event_proto BYTEA,
    pending_z_score     REAL CHECK (
        pending_z_score IS NULL OR (
        pending_z_score <> 'NaN'::REAL
        AND pending_z_score <> 'Infinity'::REAL
        AND pending_z_score <> '-Infinity'::REAL
        )
    ),
    pending_created_at  TIMESTAMPTZ,
    pending_expires_at  TIMESTAMPTZ,
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),

    PRIMARY KEY (tenant_id, model, agent_id, prompt_class),
    CHECK (
        (
            last_emitted_at IS NULL
            AND suppress_until IS NULL
            AND last_z_score IS NULL
        )
        OR (
            last_emitted_at IS NOT NULL
            AND suppress_until IS NOT NULL
            AND last_z_score IS NOT NULL
            AND suppress_until > last_emitted_at
        )
    ),
    CHECK (
        num_nonnulls(
            pending_event_id,
            pending_event_time,
            pending_event_proto,
            pending_z_score,
            pending_created_at,
            pending_expires_at
        ) IN (0, 6)
    ),
    CHECK (pending_event_proto IS NULL OR octet_length(pending_event_proto) > 0),
    CHECK (
        pending_expires_at IS NULL
        OR pending_expires_at > pending_created_at
    )
);

CREATE INDEX prediction_drift_alert_cooldowns_suppress_until_idx
    ON prediction_drift_alert_cooldowns (suppress_until);

ALTER TABLE prediction_drift_alert_cooldowns ENABLE ROW LEVEL SECURITY;
ALTER TABLE prediction_drift_alert_cooldowns FORCE ROW LEVEL SECURITY;

-- No nil-UUID sentinel: a missing/empty app.current_tenant_id casts to NULL,
-- making the comparison false and the WITH CHECK fail closed.
CREATE POLICY prediction_drift_alert_cooldowns_tenant_isolation
    ON prediction_drift_alert_cooldowns
    FOR ALL
    USING (
        tenant_id = NULLIF(current_setting('app.current_tenant_id', TRUE), '')::uuid
    )
    WITH CHECK (
        tenant_id = NULLIF(current_setting('app.current_tenant_id', TRUE), '')::uuid
    );

REVOKE SELECT, INSERT, UPDATE, DELETE ON prediction_drift_alert_cooldowns FROM PUBLIC;

GRANT SELECT, INSERT, UPDATE, DELETE
    ON prediction_drift_alert_cooldowns
    TO canonical_ingest_application_role;

GRANT SELECT ON prediction_drift_alert_cooldowns TO canonical_ingest_reader_role;

COMMENT ON TABLE prediction_drift_alert_cooldowns IS
    'POST_GA_06 mutable dedup state for stats_aggregator prediction_drift_alert CloudEvents. PRIMARY KEY is exactly (tenant_id, model, agent_id, prompt_class); key constraints mirror canonical_events aggregator columns; rows suppress repeat immutable audit alerts for 24h.';
COMMENT ON COLUMN prediction_drift_alert_cooldowns.suppress_until IS
    'Rolling cooldown expiry. stats_aggregator may emit the next alert for the same bucket only when suppress_until <= now().';
COMMENT ON COLUMN prediction_drift_alert_cooldowns.last_z_score IS
    'Finite z-score that triggered the latest emitted alert. CHECK explicitly rejects NaN and +/-Infinity.';
COMMENT ON COLUMN prediction_drift_alert_cooldowns.pending_event_proto IS
    'Signed CloudEvent proto bytes reserved before append. Retries reuse these exact bytes/id so canonical_ingest can dedupe commit-then-timeout retries.';

DO $$
BEGIN
    SET LOCAL search_path = pg_catalog, pg_temp;
    PERFORM 1 FROM pg_class
        WHERE relname = 'prediction_drift_alert_cooldowns' AND relrowsecurity = TRUE;
    IF NOT FOUND THEN
        RAISE EXCEPTION 'prediction_drift_alert_cooldowns RLS not enabled after migration';
    END IF;
    PERFORM 1 FROM pg_policy
        WHERE polname = 'prediction_drift_alert_cooldowns_tenant_isolation';
    IF NOT FOUND THEN
        RAISE EXCEPTION 'prediction_drift_alert_cooldowns_tenant_isolation policy missing';
    END IF;
    PERFORM 1 FROM pg_indexes
        WHERE schemaname = 'public'
          AND indexname = 'prediction_drift_alert_cooldowns_suppress_until_idx';
    IF NOT FOUND THEN
        RAISE EXCEPTION 'prediction_drift_alert_cooldowns_suppress_until_idx missing';
    END IF;
END $$;
