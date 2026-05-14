-- =====================================================================
-- 0043: Cost Advisor proposal write path — DSL patch allowlist + state-
--       change NOTIFY for bundle_registry LISTEN. (CA-P3 / issue #55+#54)
-- =====================================================================
--
-- Owner-acks from CA-P1.5 follow-up:
--   * #55 (bundle_registry) — "Allowlist 3-5 specific RFC-6902 JSON
--     Pointer paths". This migration ships 5 paths matching the real
--     contract DSL field shape (see services/sidecar/src/contract/parse.rs
--     for the YAML schema; fields are FLAT — `claim_amount_atomic_gt`
--     not nested `claim/amount_atomic_gt`):
--       /budgets/<i>/limit_amount_atomic
--       /budgets/<i>/reservation_ttl_seconds
--       /rules/<i>/when/claim_amount_atomic_gt
--       /rules/<i>/then/decision
--       /rules/<i>/then/approver_role
--     where <i> is an RFC 6901 array index (`0` or `[1-9][0-9]*` — no
--     leading zeros per RFC 6901 §4). The bundle_registry CD pipeline
--     resolves the patch against the current contract bundle. If the
--     rule's array position has shifted, the apply fails — that's a
--     feature: cost_advisor re-proposes against the new bundle.
--     Codex CA-P3 r1 P1: original allowlist had nested-style paths
--     that didn't match parse.rs; corrected to flat style.
--   * #54 (bundle_registry) — "Postgres LISTEN/NOTIFY". This migration
--     emits NOTIFY on channel `approval_requests_state_change` AFTER
--     UPDATE OF state. Bundle_registry's worker LISTENs on the channel
--     and triggers the contract-bundle build when state → approved
--     AND proposal_source = 'cost_advisor'.
--
-- Allowed RFC-6902 operations: `replace` ONLY. cost_advisor adjusts
-- existing fields; structural changes (add/remove rules or budgets)
-- are out of scope for v0.1 closed-loop.

-- ---------------------------------------------------------------------
-- Part 1: audit_decision_event_id is NOT applicable to cost_advisor
-- proposals (no originating audit.decision event). Make it nullable +
-- gate NOT NULL behind proposal_source = 'sidecar_decision'.
-- ---------------------------------------------------------------------
--
-- 0026 declared the column NOT NULL because the sidecar_decision flow
-- always has a corresponding audit.decision in canonical_events. The
-- cost_advisor flow doesn't (it emits cost_findings, not audit events
-- — see CA-P1.6 / integration-doc §1 closed loop). The constraint
-- needs to relax for cost_advisor and stay tight for sidecar_decision.
--
-- The 0029 immutability trigger marks audit_decision_event_id as
-- ALWAYS-frozen, so once a row is INSERTed with NULL it stays NULL.
-- That's the desired behavior for cost_advisor.

ALTER TABLE approval_requests
    ALTER COLUMN audit_decision_event_id DROP NOT NULL;

ALTER TABLE approval_requests
    ADD CONSTRAINT approval_requests_audit_decision_required_for_sidecar
    CHECK (
        proposal_source <> 'sidecar_decision'
        OR audit_decision_event_id IS NOT NULL
    ) NOT VALID;

ALTER TABLE approval_requests
    VALIDATE CONSTRAINT approval_requests_audit_decision_required_for_sidecar;

COMMENT ON COLUMN approval_requests.audit_decision_event_id IS
    'Cost Advisor / CA-P3: foreign key into canonical_events for the originating audit.decision event. Required (NOT NULL via approval_requests_audit_decision_required_for_sidecar CHECK) when proposal_source=sidecar_decision; nullable when proposal_source=cost_advisor (which has no audit.decision; the originating evidence is in cost_findings, addressed by proposing_finding_id instead).';

-- ---------------------------------------------------------------------
-- Part 2: DSL patch allowlist validator function.
-- ---------------------------------------------------------------------
--
-- Pure function (IMMUTABLE PARALLEL SAFE) so it's usable in a CHECK
-- constraint. Walks every op in the patch array; rejects if any op
-- violates the rules.
--
-- Rules:
--   1. Patch MUST be a non-empty JSON array (RFC-6902 shape).
--   2. Patch length <= 8 ops (DoS guard; real patches are 1-2 ops).
--   3. Every op MUST be an object with string `op` and string `path`.
--   4. `op` MUST be `replace`. (No add/remove/move/copy/test in v0.1.)
--   5. `path` MUST match the allowlist regex (5 specific paths).
--   6. Every op MUST have a `value` field, and the value MUST satisfy
--      the per-path schema (codex CA-P3 r1 P2: shallow type/enum gate
--      so bad patches fail HERE rather than at bundle apply time):
--        limit_amount_atomic   → digit string 1..38 chars (NUMERIC(38,0))
--        reservation_ttl_seconds → integer in [1, 86400]
--        claim_amount_atomic_gt → digit string 1..38 chars
--        decision              → enum {STOP, REQUIRE_APPROVAL, DEGRADE,
--                                       CONTINUE, SKIP}
--        approver_role         → non-empty string, [A-Za-z0-9_-]+, ≤ 64
--   7. Array indices follow RFC 6901 §4: `0` or `[1-9][0-9]*` (no
--      leading zeros). Codex CA-P3 r1 P2 tightening.
--
-- Returns TRUE if patch is valid, FALSE otherwise. NULL input returns
-- TRUE (the existing approval_requests_cost_advisor_fields_present
-- CHECK in 0038 already rejects NULL patches for cost_advisor rows).

CREATE OR REPLACE FUNCTION cost_advisor_validate_proposed_dsl_patch(p_patch JSONB)
RETURNS BOOLEAN
LANGUAGE plpgsql
IMMUTABLE
PARALLEL SAFE
SET search_path = pg_catalog, pg_temp
AS $$
DECLARE
    elem JSONB;
    op_value TEXT;
    path_value TEXT;
    value_field JSONB;
    -- RFC 6901 array index: `0` OR `[1-9][0-9]*` (no leading zeros).
    -- Combined into a single regex per allowed path.
    idx_pat CONSTANT TEXT := '(0|[1-9][0-9]*)';
    allowed_pattern CONSTANT TEXT :=
        '^/(budgets/' || idx_pat || '/(limit_amount_atomic|reservation_ttl_seconds)|' ||
        'rules/' || idx_pat || '/(when/claim_amount_atomic_gt|then/(decision|approver_role)))$';
    leaf TEXT;
    val_text TEXT;
    val_int BIGINT;
BEGIN
    IF p_patch IS NULL THEN RETURN TRUE; END IF;
    IF jsonb_typeof(p_patch) <> 'array' THEN RETURN FALSE; END IF;
    IF jsonb_array_length(p_patch) = 0 THEN RETURN FALSE; END IF;
    IF jsonb_array_length(p_patch) > 8 THEN RETURN FALSE; END IF;

    FOR elem IN SELECT * FROM jsonb_array_elements(p_patch)
    LOOP
        IF jsonb_typeof(elem) <> 'object' THEN RETURN FALSE; END IF;

        op_value := elem->>'op';
        IF op_value IS NULL OR op_value <> 'replace' THEN
            RETURN FALSE;
        END IF;

        path_value := elem->>'path';
        IF path_value IS NULL THEN RETURN FALSE; END IF;
        IF path_value !~ allowed_pattern THEN RETURN FALSE; END IF;

        IF NOT (elem ? 'value') THEN RETURN FALSE; END IF;
        value_field := elem->'value';

        -- Per-path value-schema gate (codex CA-P3 r1 P2).
        -- Extract the leaf segment (everything after the last `/`).
        leaf := split_part(path_value, '/', regexp_count(path_value, '/') + 1);

        IF leaf IN ('limit_amount_atomic', 'claim_amount_atomic_gt') THEN
            -- Atomic amount: string of 1..38 ASCII digits.
            IF jsonb_typeof(value_field) <> 'string' THEN RETURN FALSE; END IF;
            val_text := value_field #>> '{}';
            IF val_text IS NULL OR val_text = '' OR length(val_text) > 38 THEN
                RETURN FALSE;
            END IF;
            IF val_text !~ '^[0-9]+$' THEN RETURN FALSE; END IF;

        ELSIF leaf = 'reservation_ttl_seconds' THEN
            -- Integer in [1, 86400].
            IF jsonb_typeof(value_field) <> 'number' THEN RETURN FALSE; END IF;
            BEGIN
                val_int := (value_field #>> '{}')::BIGINT;
            EXCEPTION WHEN OTHERS THEN
                RETURN FALSE;
            END;
            IF val_int < 1 OR val_int > 86400 THEN RETURN FALSE; END IF;

        ELSIF leaf = 'decision' THEN
            -- Enum from contract DSL.
            IF jsonb_typeof(value_field) <> 'string' THEN RETURN FALSE; END IF;
            val_text := value_field #>> '{}';
            IF val_text IS NULL OR val_text NOT IN
                ('STOP', 'REQUIRE_APPROVAL', 'DEGRADE', 'CONTINUE', 'SKIP')
            THEN
                RETURN FALSE;
            END IF;

        ELSIF leaf = 'approver_role' THEN
            -- Non-empty role name, ≤ 64 chars, [A-Za-z0-9_-]+
            IF jsonb_typeof(value_field) <> 'string' THEN RETURN FALSE; END IF;
            val_text := value_field #>> '{}';
            IF val_text IS NULL OR val_text = '' OR length(val_text) > 64 THEN
                RETURN FALSE;
            END IF;
            IF val_text !~ '^[A-Za-z0-9_-]+$' THEN RETURN FALSE; END IF;

        ELSE
            -- Allowlist regex above should have rejected; defense-in-depth.
            RETURN FALSE;
        END IF;
    END LOOP;

    RETURN TRUE;
END;
$$;

COMMENT ON FUNCTION cost_advisor_validate_proposed_dsl_patch IS
    'CA-P3 / owner-ack #55: validates an RFC-6902 patch against the cost_advisor allowlist (5 specific JSON Pointer paths). Used as a CHECK constraint on approval_requests so disallowed patches cannot be INSERTed. IMMUTABLE PARALLEL SAFE so it can live in a CHECK constraint without rewriting on every INSERT.';

-- ---------------------------------------------------------------------
-- Part 3: CHECK constraint enforcing the allowlist on cost_advisor
-- proposals.
-- ---------------------------------------------------------------------

ALTER TABLE approval_requests
    ADD CONSTRAINT approval_requests_cost_advisor_patch_allowlist
    CHECK (
        proposal_source <> 'cost_advisor'
        OR cost_advisor_validate_proposed_dsl_patch(proposed_dsl_patch)
    ) NOT VALID;

ALTER TABLE approval_requests
    VALIDATE CONSTRAINT approval_requests_cost_advisor_patch_allowlist;

-- ---------------------------------------------------------------------
-- Part 4: pg_notify trigger for bundle_registry LISTEN (owner-ack #54)
-- ---------------------------------------------------------------------
--
-- Bundle_registry's CD worker LISTENs on `approval_requests_state_change`
-- and triggers a contract-bundle rebuild when a cost_advisor row
-- transitions to state='approved'. The notification payload is small
-- (well under the 8KB Postgres NOTIFY limit) and contains just the
-- pointer fields the worker needs to fetch the full row.
--
-- AFTER UPDATE OF state (not INSERT) — we only care about transitions,
-- not the initial 'pending' insert. WHEN clause further restricts to
-- actual state changes (the immutability trigger would already block
-- a no-op UPDATE, but the WHEN filter spares pg_notify the call).

CREATE OR REPLACE FUNCTION approval_requests_notify_state_change()
RETURNS TRIGGER
LANGUAGE plpgsql
SET search_path = pg_catalog, pg_temp
AS $$
DECLARE
    payload_json TEXT;
BEGIN
    payload_json := json_build_object(
        'approval_id',     NEW.approval_id,
        'tenant_id',       NEW.tenant_id,
        'proposal_source', NEW.proposal_source,
        'old_state',       OLD.state,
        'new_state',       NEW.state,
        'resolved_at',     NEW.resolved_at
    )::text;
    PERFORM pg_notify('approval_requests_state_change', payload_json);
    RETURN NEW;
END;
$$;

COMMENT ON FUNCTION approval_requests_notify_state_change IS
    'CA-P3 / owner-ack #54: emits NOTIFY on channel approval_requests_state_change when a row transitions state. Bundle_registry CD worker LISTENs and triggers contract-bundle rebuild on approved cost_advisor proposals.';

CREATE TRIGGER approval_requests_state_change_notify
    AFTER UPDATE OF state ON approval_requests
    FOR EACH ROW
    WHEN (OLD.state IS DISTINCT FROM NEW.state)
    EXECUTE FUNCTION approval_requests_notify_state_change();

COMMENT ON TRIGGER approval_requests_state_change_notify ON approval_requests IS
    'CA-P3 / owner-ack #54: fires the LISTEN/NOTIFY signal for bundle_registry. Restricted to actual state transitions (WHEN clause filters no-op UPDATEs).';

-- ---------------------------------------------------------------------
-- Part 5: cost_advisor_create_proposal SP — SECURITY DEFINER
-- (codex CA-P3 r1 P1: direct INSERT bypass risk)
-- ---------------------------------------------------------------------
--
-- The default INSERT path lets a caller write `state='approved'` with
-- resolved_* fields populated, skipping the resolve_approval_request
-- SP entirely and therefore skipping the approval_events audit chain
-- AND the pg_notify trigger (which fires on UPDATE only, not INSERT).
-- A buggy or compromised cost_advisor writer could thereby push fake
-- "approved" proposals straight into bundle_registry's CD path.
--
-- Defense: a SECURITY DEFINER SP that is the SOLE legal writer for
-- cost_advisor proposals. It hard-codes:
--   * state = 'pending'
--   * resolved_at / resolved_by_* = NULL
--   * proposal_source = 'cost_advisor'
--   * audit_decision_event_id = NULL (cost_advisor has no audit event)
--
-- Caller can only set the safe fields (decision_id, patch, finding_id,
-- ttl). Future P3.5 work GRANTs EXECUTE to cost_advisor_application_role
-- and REVOKEs INSERT on approval_requests from that role entirely.
--
-- Hardening (codex CA-P1.6 r3 pattern, reused):
--   * search_path locked to pg_catalog, pg_temp (pg_temp explicit-last)
--   * Schema-qualified relation refs (public.X)
--   * REVOKE ALL FROM PUBLIC

CREATE OR REPLACE FUNCTION cost_advisor_create_proposal(
    p_tenant_id           UUID,
    p_decision_id         UUID,
    p_proposed_dsl_patch  JSONB,
    p_proposing_finding_id UUID,
    p_ttl_expires_at      TIMESTAMPTZ
) RETURNS TABLE (
    approval_id   UUID,
    outcome       TEXT
) LANGUAGE plpgsql AS $$
DECLARE
    v_approval_id UUID;
BEGIN
    INSERT INTO public.approval_requests (
        approval_id, tenant_id, decision_id, state,
        proposal_source, proposed_dsl_patch, proposing_finding_id,
        ttl_expires_at, created_at,
        approver_policy, requested_effect, decision_context,
        audit_decision_event_id,
        resolved_at, resolved_by_subject, resolved_by_issuer, resolution_reason
    ) VALUES (
        gen_random_uuid(), p_tenant_id, p_decision_id, 'pending',
        'cost_advisor', p_proposed_dsl_patch, p_proposing_finding_id,
        p_ttl_expires_at, clock_timestamp(),
        '{}'::jsonb, '{}'::jsonb, '{}'::jsonb,
        NULL,
        NULL, NULL, NULL, NULL
    )
    ON CONFLICT (tenant_id, decision_id) DO NOTHING
    RETURNING public.approval_requests.approval_id INTO v_approval_id;

    IF v_approval_id IS NULL THEN
        RETURN QUERY SELECT NULL::UUID, 'already_exists'::TEXT;
    ELSE
        RETURN QUERY SELECT v_approval_id, 'inserted'::TEXT;
    END IF;
END;
$$;

ALTER FUNCTION cost_advisor_create_proposal(UUID, UUID, JSONB, UUID, TIMESTAMPTZ)
    SECURITY DEFINER SET search_path = pg_catalog, pg_temp;

REVOKE ALL ON FUNCTION cost_advisor_create_proposal(UUID, UUID, JSONB, UUID, TIMESTAMPTZ)
    FROM PUBLIC;

COMMENT ON FUNCTION cost_advisor_create_proposal IS
    'CA-P3 + codex r1 P1: SOLE legal writer for cost_advisor proposals. SECURITY DEFINER + REVOKE FROM PUBLIC. Hard-codes state=pending + NULL resolution fields so callers cannot bypass the resolve_approval_request transition (which is what fires approval_events + the pg_notify trigger). On conflict (tenant_id, decision_id) returns outcome=already_exists, otherwise inserted.';
