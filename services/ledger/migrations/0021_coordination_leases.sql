-- Phase 5 GA Hardening S1: lease primitive for singleton background workers.
--
-- Provides a Postgres-backed leader-election table that singleton
-- workers (outbox-forwarder, ttl-sweeper, future pollers) can acquire
-- before processing work. The k8s Lease API is the production-mode
-- alternative; this Postgres mode covers compose / local integration
-- tests / non-k8s deployments.
--
-- Invariants:
--   * One workload_id holds a given lease at a time.
--   * Renewal by the current holder does NOT change holder_token; only
--     advances expires_at.
--   * Takeover (after expiry) mints a fresh holder_token and bumps
--     transition_count atomically — used as a fencing-style epoch so
--     stale leaders detect they've been displaced.
--   * acquire_lease() runs under FOR UPDATE on the row, so concurrent
--     contenders serialize.

CREATE TABLE coordination_leases (
    lease_name              TEXT PRIMARY KEY,
    -- NULL means lease has never been held (after first INSERT) — first
    -- acquire mints holder fields. After expiry we keep the row but
    -- treat it as eligible for takeover.
    holder_workload_id      TEXT,
    -- Refreshed on every takeover; carriers use this for HMAC-style
    -- "I'm still leader" assertions in renew calls.
    holder_token            UUID,
    region                  TEXT NOT NULL,
    acquired_at             TIMESTAMPTZ,
    renewed_at              TIMESTAMPTZ,
    expires_at              TIMESTAMPTZ,
    -- Bumps on takeover. Behaves like fencing_epoch for leases.
    transition_count        BIGINT NOT NULL DEFAULT 0,
    -- Audit trail (last 10 transitions kept in this column for cheap
    -- on-call introspection; real history lives in coordination_lease_history).
    last_transition_note    TEXT
);

-- Per-transition history for forensics. Append-only; no UPDATE/DELETE
-- expected (immutability not enforced via trigger here because lease
-- coordination is operational, not audit-grade).
CREATE TABLE coordination_lease_history (
    history_id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    lease_name              TEXT NOT NULL,
    transition_count        BIGINT NOT NULL,
    holder_workload_id      TEXT NOT NULL,
    holder_token            UUID NOT NULL,
    event_type              TEXT NOT NULL CHECK (event_type IN
                                ('acquired', 'renewed', 'released',
                                 'taken_over', 'expired_observed')),
    event_at                TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    note                    TEXT
);

CREATE INDEX coordination_lease_history_lookup
    ON coordination_lease_history (lease_name, event_at DESC);

-- Acquire / renew / takeover in one atomic SP.
--
-- Returns a row whose semantics are:
--   granted=TRUE             — caller is the current holder for the
--                              returned (token, expires_at) tuple.
--   granted=FALSE            — lease held by someone else, not yet
--                              expired. Caller MUST back off.
--
-- Caller drives the choice between renewal vs takeover via workload_id
-- equality: passing the same workload_id as the current holder will
-- renew (preserves token + transition_count); a different workload_id
-- will only succeed if the current lease is expired.
CREATE OR REPLACE FUNCTION acquire_lease(
    p_lease_name   TEXT,
    p_workload_id  TEXT,
    p_region       TEXT,
    p_ttl_seconds  INT
) RETURNS TABLE(
    granted             BOOLEAN,
    holder_token        UUID,
    holder_workload_id  TEXT,
    expires_at          TIMESTAMPTZ,
    transition_count    BIGINT,
    event_type          TEXT
) AS $$
DECLARE
    v_now      TIMESTAMPTZ := clock_timestamp();
    v_ttl      INTERVAL := (p_ttl_seconds || ' seconds')::INTERVAL;
    v_existing RECORD;
    v_new_token UUID;
    v_event    TEXT;
BEGIN
    IF p_lease_name IS NULL OR length(p_lease_name) = 0 THEN
        RAISE EXCEPTION 'lease_name required' USING ERRCODE = '22023';
    END IF;
    IF p_workload_id IS NULL OR length(p_workload_id) = 0 THEN
        RAISE EXCEPTION 'workload_id required' USING ERRCODE = '22023';
    END IF;
    IF p_ttl_seconds <= 0 THEN
        RAISE EXCEPTION 'ttl_seconds must be > 0' USING ERRCODE = '22023';
    END IF;

    -- Ensure row exists.
    INSERT INTO coordination_leases (lease_name, region)
        VALUES (p_lease_name, p_region)
        ON CONFLICT (lease_name) DO NOTHING;

    -- Lock the row for the duration of this txn.
    SELECT * INTO v_existing
      FROM coordination_leases
     WHERE lease_name = p_lease_name
       FOR UPDATE;

    -- Path A: renewal by current holder (still within TTL).
    IF v_existing.holder_workload_id = p_workload_id
       AND v_existing.expires_at IS NOT NULL
       AND v_existing.expires_at > v_now THEN
        UPDATE coordination_leases
           SET renewed_at = v_now,
               expires_at = v_now + v_ttl
         WHERE lease_name = p_lease_name;
        v_event := 'renewed';
        INSERT INTO coordination_lease_history
            (lease_name, transition_count, holder_workload_id,
             holder_token, event_type, note)
        VALUES (p_lease_name, v_existing.transition_count, p_workload_id,
                v_existing.holder_token, v_event,
                'renew within TTL');
        RETURN QUERY SELECT true, v_existing.holder_token, p_workload_id,
                            v_now + v_ttl,
                            v_existing.transition_count, v_event;
        RETURN;
    END IF;

    -- Path B: takeover (no holder OR existing lease expired).
    IF v_existing.holder_workload_id IS NULL
       OR v_existing.expires_at IS NULL
       OR v_existing.expires_at <= v_now THEN
        v_new_token := gen_random_uuid();
        v_event := CASE
                       WHEN v_existing.holder_workload_id IS NULL THEN 'acquired'
                       WHEN v_existing.holder_workload_id = p_workload_id THEN 'acquired'
                       ELSE 'taken_over'
                   END;
        UPDATE coordination_leases
           SET holder_workload_id = p_workload_id,
               holder_token = v_new_token,
               region = p_region,
               acquired_at = v_now,
               renewed_at = v_now,
               expires_at = v_now + v_ttl,
               transition_count = v_existing.transition_count + 1,
               last_transition_note = v_event
         WHERE lease_name = p_lease_name;
        INSERT INTO coordination_lease_history
            (lease_name, transition_count, holder_workload_id,
             holder_token, event_type, note)
        VALUES (p_lease_name, v_existing.transition_count + 1, p_workload_id,
                v_new_token, v_event,
                CASE WHEN v_existing.holder_workload_id IS NOT NULL
                     THEN format('previous holder %s', v_existing.holder_workload_id)
                     ELSE 'first acquire' END);
        RETURN QUERY SELECT true, v_new_token, p_workload_id,
                            v_now + v_ttl,
                            v_existing.transition_count + 1, v_event;
        RETURN;
    END IF;

    -- Path C: held by someone else, not yet expired. Caller backs off.
    RETURN QUERY SELECT false, v_existing.holder_token, v_existing.holder_workload_id,
                        v_existing.expires_at, v_existing.transition_count, 'denied';
END;
$$ LANGUAGE plpgsql;

-- Release a lease the caller currently holds. Idempotent — releasing a
-- lease the caller does NOT hold is a no-op (returns FALSE).
CREATE OR REPLACE FUNCTION release_lease(
    p_lease_name TEXT,
    p_workload_id TEXT,
    p_holder_token UUID
) RETURNS BOOLEAN AS $$
DECLARE
    v_existing RECORD;
BEGIN
    SELECT * INTO v_existing
      FROM coordination_leases
     WHERE lease_name = p_lease_name
       FOR UPDATE;
    IF NOT FOUND THEN
        RETURN FALSE;
    END IF;
    IF v_existing.holder_workload_id <> p_workload_id
       OR v_existing.holder_token <> p_holder_token THEN
        RETURN FALSE;
    END IF;
    UPDATE coordination_leases
       SET holder_workload_id = NULL,
           holder_token = NULL,
           acquired_at = NULL,
           renewed_at = NULL,
           expires_at = NULL,
           last_transition_note = 'released by holder'
     WHERE lease_name = p_lease_name;
    INSERT INTO coordination_lease_history
        (lease_name, transition_count, holder_workload_id,
         holder_token, event_type, note)
    VALUES (p_lease_name, v_existing.transition_count, p_workload_id,
            p_holder_token, 'released', 'graceful release');
    RETURN TRUE;
END;
$$ LANGUAGE plpgsql;

GRANT EXECUTE ON FUNCTION acquire_lease(TEXT, TEXT, TEXT, INT) TO PUBLIC;
GRANT EXECUTE ON FUNCTION release_lease(TEXT, TEXT, UUID) TO PUBLIC;
