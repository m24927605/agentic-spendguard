-- COV_D41S_05 session_reservation demo hard gate (ledger DB).

\echo
\echo === D41 session_reservation lifecycle row ===
SELECT session_reservation_id, status, reserved_amount_atomic::text,
       committed_amount_atomic::text, released_amount_atomic::text
  FROM session_reservations
 WHERE tenant_id = '00000000-0000-4000-8000-000000000001'
	   AND session_id IN (
	       'd41-session-reservation-demo',
	       'd41-session-reservation-deny-demo',
	       'd41-session-reservation-expire-demo'
	   )
 ORDER BY session_id;

\echo
\echo === D41 session_reservation hard assertions ===
DO $$
DECLARE
    v_session_id UUID;
    v_denied_session_id UUID;
    v_expired_session_id UUID;
    v_reserved TEXT;
    v_committed TEXT;
    v_released TEXT;
    v_status TEXT;
    v_denied_reserved TEXT;
    v_denied_committed TEXT;
    v_denied_released TEXT;
    v_denied_status TEXT;
    v_denied_reason TEXT;
    v_expired_reserved TEXT;
    v_expired_committed TEXT;
    v_expired_released TEXT;
    v_expired_status TEXT;
    v_applied INT;
    v_denied INT;
    v_decisions INT;
    v_outcomes INT;
    v_pending INT;
    v_reserve_events INT;
    v_commit_events INT;
    v_denied_events INT;
    v_release_events INT;
    v_expired_events INT;
    v_available TEXT;
    v_reserved_hold TEXT;
    v_committed_spend TEXT;
BEGIN
    SELECT session_reservation_id, reserved_amount_atomic::TEXT,
           committed_amount_atomic::TEXT, released_amount_atomic::TEXT, status
      INTO v_session_id, v_reserved, v_committed, v_released, v_status
      FROM session_reservations
     WHERE tenant_id = '00000000-0000-4000-8000-000000000001'
       AND session_id = 'd41-session-reservation-demo';

    IF v_session_id IS NULL THEN
        RAISE EXCEPTION 'COV_D41S_GATE: session_reservation row missing';
    END IF;
    IF (v_reserved, v_committed, v_released, v_status)
       IS DISTINCT FROM ('100000', '3000', '97000', 'released') THEN
        RAISE EXCEPTION 'COV_D41S_GATE: lifecycle mismatch reserved=% committed=% released=% status=%',
            v_reserved, v_committed, v_released, v_status;
    END IF;

    SELECT session_reservation_id, reserved_amount_atomic::TEXT,
           committed_amount_atomic::TEXT, released_amount_atomic::TEXT, status,
           reserve_outcome->>'reason'
      INTO v_denied_session_id, v_denied_reserved, v_denied_committed,
           v_denied_released, v_denied_status, v_denied_reason
      FROM session_reservations
     WHERE tenant_id = '00000000-0000-4000-8000-000000000001'
       AND session_id = 'd41-session-reservation-deny-demo';

    IF v_denied_session_id IS NULL THEN
        RAISE EXCEPTION 'COV_D41S_GATE: denied session_reservation row missing';
    END IF;
    IF (v_denied_reserved, v_denied_committed, v_denied_released, v_denied_status, v_denied_reason)
       IS DISTINCT FROM ('999999', '0', '0', 'denied', 'INSUFFICIENT_AVAILABLE_BUDGET') THEN
        RAISE EXCEPTION 'COV_D41S_GATE: reserve denial mismatch reserved=% committed=% released=% status=% reason=%',
            v_denied_reserved, v_denied_committed, v_denied_released,
            v_denied_status, v_denied_reason;
    END IF;

    SELECT session_reservation_id, reserved_amount_atomic::TEXT,
           committed_amount_atomic::TEXT, released_amount_atomic::TEXT, status
      INTO v_expired_session_id, v_expired_reserved, v_expired_committed,
           v_expired_released, v_expired_status
      FROM session_reservations
     WHERE tenant_id = '00000000-0000-4000-8000-000000000001'
       AND session_id = 'd41-session-reservation-expire-demo';

    IF v_expired_session_id IS NULL THEN
        RAISE EXCEPTION 'COV_D41S_GATE: expired session_reservation row missing';
    END IF;
    IF (v_expired_reserved, v_expired_committed, v_expired_released, v_expired_status)
       IS DISTINCT FROM ('5000', '0', '5000', 'expired') THEN
        RAISE EXCEPTION 'COV_D41S_GATE: expiry mismatch reserved=% committed=% released=% status=%',
            v_expired_reserved, v_expired_committed, v_expired_released, v_expired_status;
    END IF;

    SELECT COUNT(*) INTO v_applied
      FROM session_commit_deltas
     WHERE session_reservation_id = v_session_id
       AND applied = TRUE;
    SELECT COUNT(*) INTO v_denied
      FROM session_commit_deltas
     WHERE session_reservation_id = v_session_id
       AND applied = FALSE
       AND commit_outcome->>'reason' = 'OVERRUN_RESERVATION';
    IF v_applied <> 2 THEN
        RAISE EXCEPTION 'COV_D41S_GATE: expected 2 applied deltas, got %', v_applied;
    END IF;
    IF v_denied <> 1 THEN
        RAISE EXCEPTION 'COV_D41S_GATE: expected 1 overrun denial delta, got %', v_denied;
    END IF;

    SELECT COALESCE(SUM(CASE le.direction WHEN 'credit' THEN le.amount_atomic WHEN 'debit' THEN -le.amount_atomic END), 0)::TEXT
      INTO v_available
      FROM ledger_accounts la
      LEFT JOIN ledger_entries le ON le.ledger_account_id = la.ledger_account_id
     WHERE la.tenant_id = '00000000-0000-4000-8000-000000000001'
       AND la.budget_id = '44444444-4444-4444-8444-444444444444'
       AND la.window_instance_id = '55555555-5555-4555-8555-555555555555'
       AND la.unit_id = '88888888-8888-4888-8888-888888888888'
       AND la.account_kind = 'available_budget';
    SELECT COALESCE(SUM(CASE le.direction WHEN 'credit' THEN le.amount_atomic WHEN 'debit' THEN -le.amount_atomic END), 0)::TEXT
      INTO v_reserved_hold
      FROM ledger_accounts la
      LEFT JOIN ledger_entries le ON le.ledger_account_id = la.ledger_account_id
     WHERE la.tenant_id = '00000000-0000-4000-8000-000000000001'
       AND la.budget_id = '44444444-4444-4444-8444-444444444444'
       AND la.window_instance_id = '55555555-5555-4555-8555-555555555555'
       AND la.unit_id = '88888888-8888-4888-8888-888888888888'
       AND la.account_kind = 'reserved_hold';
    SELECT COALESCE(SUM(CASE le.direction WHEN 'credit' THEN le.amount_atomic WHEN 'debit' THEN -le.amount_atomic END), 0)::TEXT
      INTO v_committed_spend
      FROM ledger_accounts la
      LEFT JOIN ledger_entries le ON le.ledger_account_id = la.ledger_account_id
     WHERE la.tenant_id = '00000000-0000-4000-8000-000000000001'
       AND la.budget_id = '44444444-4444-4444-8444-444444444444'
       AND la.window_instance_id = '55555555-5555-4555-8555-555555555555'
       AND la.unit_id = '88888888-8888-4888-8888-888888888888'
       AND la.account_kind = 'committed_spend';

    IF (v_available, v_reserved_hold, v_committed_spend)
       IS DISTINCT FROM ('97000', '0', '3000') THEN
        RAISE EXCEPTION 'COV_D41S_GATE: balance mismatch available=% reserved_hold=% committed_spend=%',
            v_available, v_reserved_hold, v_committed_spend;
    END IF;

    WITH session_audit AS (
        SELECT event_type,
               pending_forward,
               convert_from(decode(cloudevent_payload->>'data_b64', 'base64'), 'UTF8')::jsonb AS data
          FROM audit_outbox
         WHERE tenant_id = '00000000-0000-4000-8000-000000000001'
           AND workload_instance_id = 'session-reservation-ledger'
    )
    SELECT
        COUNT(*) FILTER (WHERE event_type = 'spendguard.audit.decision'),
        COUNT(*) FILTER (WHERE event_type = 'spendguard.audit.outcome'),
        COUNT(*) FILTER (WHERE pending_forward = TRUE)
      INTO v_decisions, v_outcomes, v_pending
      FROM session_audit
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
        RAISE EXCEPTION 'COV_D41S_GATE: expected exactly 8 decision/outcome session audit pairs, got decisions=% outcomes=%',
            v_decisions, v_outcomes;
    END IF;
    IF v_pending <> 0 THEN
        RAISE EXCEPTION 'COV_D41S_GATE: session audit rows still pending_forward=true after drain: %', v_pending;
    END IF;

    WITH session_audit AS (
        SELECT DISTINCT
               event_type,
               convert_from(decode(cloudevent_payload->>'data_b64', 'base64'), 'UTF8')::jsonb AS data
          FROM audit_outbox
         WHERE tenant_id = '00000000-0000-4000-8000-000000000001'
           AND workload_instance_id = 'session-reservation-ledger'
    )
    SELECT
        COUNT(*) FILTER (WHERE data->>'session_event_type' = 'spendguard.audit.session.reserve'),
        COUNT(*) FILTER (WHERE data->>'session_event_type' = 'spendguard.audit.session.commit_delta'),
        COUNT(*) FILTER (WHERE data->>'session_event_type' = 'spendguard.audit.session.denied'),
        COUNT(*) FILTER (WHERE data->>'session_event_type' = 'spendguard.audit.session.release'),
        COUNT(*) FILTER (WHERE data->>'session_event_type' = 'spendguard.audit.session.expired')
      INTO v_reserve_events, v_commit_events, v_denied_events,
           v_release_events, v_expired_events
      FROM session_audit
     WHERE data->>'phase' = 'outcome'
       AND data->>'session_reservation_id' IN (
           v_session_id::TEXT,
           v_denied_session_id::TEXT,
           v_expired_session_id::TEXT
       );

    IF (v_reserve_events, v_commit_events, v_denied_events, v_release_events, v_expired_events)
       IS DISTINCT FROM (2, 2, 2, 1, 1) THEN
        RAISE EXCEPTION 'COV_D41S_GATE: session event distribution mismatch reserve=% commit=% denied=% release=% expired=%',
            v_reserve_events, v_commit_events, v_denied_events, v_release_events, v_expired_events;
    END IF;

    RAISE NOTICE 'COV_D41S LEDGER OK: session=% decisions=% outcomes=% available=% committed=%',
        v_session_id, v_decisions, v_outcomes, v_available, v_committed_spend;
END;
$$;
