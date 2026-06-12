-- COV_D41S_05 session_reservation demo hard gate (canonical DB).

CREATE TEMP TABLE d41_session_reservation_gate_ids (
    session_id UUID NOT NULL,
    denied_session_id UUID NOT NULL,
    expired_session_id UUID NOT NULL
);

INSERT INTO d41_session_reservation_gate_ids (
    session_id,
    denied_session_id,
    expired_session_id
) VALUES (
    :'session_id'::UUID,
    :'denied_session_id'::UUID,
    :'expired_session_id'::UUID
);

DO $d41$
DECLARE
    v_session_id UUID;
    v_denied_session_id UUID;
    v_expired_session_id UUID;
    v_decisions INT;
    v_outcomes INT;
    v_reserve_events INT;
    v_commit_events INT;
    v_denied_events INT;
    v_release_events INT;
    v_expired_events INT;
BEGIN
    SELECT session_id, denied_session_id, expired_session_id
      INTO v_session_id, v_denied_session_id, v_expired_session_id
      FROM d41_session_reservation_gate_ids;

    WITH session_events AS (
        SELECT event_type,
               convert_from(decode(payload_json->>'data_b64', 'base64'), 'UTF8')::jsonb AS data
          FROM canonical_events
         WHERE tenant_id = '00000000-0000-4000-8000-000000000001'
           AND producer_id = 'ledger:session-reservation-ledger'
    )
    SELECT
        COUNT(*) FILTER (WHERE event_type = 'spendguard.audit.decision'),
        COUNT(*) FILTER (WHERE event_type = 'spendguard.audit.outcome')
      INTO v_decisions, v_outcomes
      FROM session_events
     WHERE data->>'session_reservation_id' IN (
           v_session_id::TEXT,
           v_denied_session_id::TEXT,
           v_expired_session_id::TEXT
       )
       AND data->>'session_event_type' IN (
           'spendguard.audit.session.reserve',
           'spendguard.audit.session.commit_delta',
           'spendguard.audit.session.denied',
           'spendguard.audit.session.release',
           'spendguard.audit.session.expired'
       );

    IF v_decisions <> 8 OR v_outcomes <> 8 THEN
        RAISE EXCEPTION 'COV_D41S_GATE: expected exactly 8 canonical decision/outcome session audit pairs, got decisions=% outcomes=%',
            v_decisions, v_outcomes;
    END IF;

    WITH session_events AS (
        SELECT DISTINCT
               event_type,
               convert_from(decode(payload_json->>'data_b64', 'base64'), 'UTF8')::jsonb AS data
          FROM canonical_events
         WHERE tenant_id = '00000000-0000-4000-8000-000000000001'
           AND producer_id = 'ledger:session-reservation-ledger'
    )
    SELECT
        COUNT(*) FILTER (WHERE data->>'session_event_type' = 'spendguard.audit.session.reserve'),
        COUNT(*) FILTER (WHERE data->>'session_event_type' = 'spendguard.audit.session.commit_delta'),
        COUNT(*) FILTER (WHERE data->>'session_event_type' = 'spendguard.audit.session.denied'),
        COUNT(*) FILTER (WHERE data->>'session_event_type' = 'spendguard.audit.session.release'),
        COUNT(*) FILTER (WHERE data->>'session_event_type' = 'spendguard.audit.session.expired')
      INTO v_reserve_events, v_commit_events, v_denied_events,
           v_release_events, v_expired_events
     FROM session_events
     WHERE data->>'phase' = 'outcome'
       AND data->>'session_reservation_id' IN (
           v_session_id::TEXT,
           v_denied_session_id::TEXT,
           v_expired_session_id::TEXT
       );

    IF (v_reserve_events, v_commit_events, v_denied_events, v_release_events, v_expired_events)
       IS DISTINCT FROM (2, 2, 2, 1, 1) THEN
        RAISE EXCEPTION 'COV_D41S_GATE: canonical session event distribution mismatch reserve=% commit=% denied=% release=% expired=%',
            v_reserve_events, v_commit_events, v_denied_events, v_release_events, v_expired_events;
    END IF;

    RAISE NOTICE 'COV_D41S CANONICAL OK: decisions=% outcomes=% reserve=% commit=% denied=% release=% expired=%',
        v_decisions, v_outcomes, v_reserve_events, v_commit_events,
        v_denied_events, v_release_events, v_expired_events;
END;
$d41$;
