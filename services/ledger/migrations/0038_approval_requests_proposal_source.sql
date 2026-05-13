-- =====================================================================
-- 0038: approval_requests proposal_source + proposed_dsl_patch
--       (Cost Advisor P0 — spec §9 P3.5 wiring + control-plane
--        integration design)
-- =====================================================================
--
-- Extends the existing approval queue so that proposals authored by
-- cost_advisor flow through the same operator approval workflow
-- (instead of forking a new product surface). Per spec §1.1 closed
-- loop: rule detects waste → emits a proposed contract DSL patch →
-- queued in approval_requests with proposal_source='cost_advisor' →
-- operator reviews in existing dashboard (filter by proposal_source) →
-- approves/denies via the existing state machine.
--
-- Schema changes:
--   * proposal_source TEXT (default 'sidecar_decision', frozen at INSERT).
--   * proposed_dsl_patch JSONB NULL (frozen at INSERT). NULL for the
--     legacy sidecar_decision rows; populated with the RFC-6902 patch
--     for cost_advisor rows.
--   * proposing_finding_id UUID NULL (frozen at INSERT). FK into
--     `spendguard_canonical.cost_findings.finding_id`. UNenforced across
--     databases — application validates before INSERT.
--
-- Also extends tenant_data_policy with the cost_findings retention
-- knobs (open / resolved windows). retention_sweeper (P1 sweep kind)
-- reads these to drive cost_findings DELETEs in the canonical DB.

-- =====================================================================
-- 1) Add the columns.
-- =====================================================================

ALTER TABLE approval_requests
    ADD COLUMN proposal_source TEXT NOT NULL DEFAULT 'sidecar_decision'
        CHECK (proposal_source IN ('sidecar_decision', 'cost_advisor'));

ALTER TABLE approval_requests
    ADD COLUMN proposed_dsl_patch JSONB;

ALTER TABLE approval_requests
    ADD COLUMN proposing_finding_id UUID;

-- Defense in depth: when proposal_source='cost_advisor', the patch +
-- finding pointer MUST be present. sidecar_decision rows leave both
-- NULL (the original semantics).
ALTER TABLE approval_requests
    ADD CONSTRAINT approval_requests_cost_advisor_fields_present
    CHECK (
        proposal_source <> 'cost_advisor'
        OR (proposed_dsl_patch IS NOT NULL AND proposing_finding_id IS NOT NULL)
    );

-- Filter index: the dashboard view that operators consume reads
-- "all pending cost_advisor proposals for tenant" frequently. The
-- partial index on (tenant_id, state, proposal_source) keeps that
-- query cheap as the table grows.
CREATE INDEX approval_requests_cost_advisor_pending_idx
    ON approval_requests (tenant_id, created_at DESC)
    WHERE proposal_source = 'cost_advisor' AND state = 'pending';

COMMENT ON COLUMN approval_requests.proposal_source IS
    'Cost Advisor P0: marks the origin of the proposal. sidecar_decision = legacy REQUIRE_APPROVAL flow (original 0026 schema). cost_advisor = rule emitted a contract DSL patch and queued it here. Dashboard filters on this column; no new UI surface required.';
COMMENT ON COLUMN approval_requests.proposed_dsl_patch IS
    'Cost Advisor P0: RFC-6902 contract DSL patch the operator would apply at approval time. Frozen at INSERT (see strengthened immutability trigger below).';
COMMENT ON COLUMN approval_requests.proposing_finding_id IS
    'Cost Advisor P0: FK into spendguard_canonical.cost_findings.finding_id. Cross-database FK is unenforced by Postgres — the writing service validates the finding exists before INSERT.';

-- =====================================================================
-- 2) Strengthen immutability trigger to cover the new columns.
-- =====================================================================
--
-- Pattern matches 0029 (strengthen the same function via CREATE OR
-- REPLACE; trigger binding stays untouched). The new columns are
-- "always frozen" — set at INSERT, never mutated. There is no SP that
-- needs to mutate them.

CREATE OR REPLACE FUNCTION approval_requests_block_immutable_updates()
    RETURNS TRIGGER LANGUAGE plpgsql AS $$
BEGIN
    -- (a) Always-frozen columns. Set at creation; never change for
    -- the lifetime of the row, regardless of state.
    IF NEW.tenant_id                  IS DISTINCT FROM OLD.tenant_id
        OR NEW.decision_id            IS DISTINCT FROM OLD.decision_id
        OR NEW.audit_decision_event_id IS DISTINCT FROM OLD.audit_decision_event_id
        OR NEW.requested_effect       IS DISTINCT FROM OLD.requested_effect
        OR NEW.decision_context       IS DISTINCT FROM OLD.decision_context
        OR NEW.created_at             IS DISTINCT FROM OLD.created_at
        OR NEW.ttl_expires_at         IS DISTINCT FROM OLD.ttl_expires_at
        OR NEW.approver_policy        IS DISTINCT FROM OLD.approver_policy
        -- 0038 (Cost Advisor): freeze proposal provenance + patch body
        -- so an approve action can never silently substitute a
        -- different patch than the one that was reviewed.
        OR NEW.proposal_source        IS DISTINCT FROM OLD.proposal_source
        OR NEW.proposed_dsl_patch     IS DISTINCT FROM OLD.proposed_dsl_patch
        OR NEW.proposing_finding_id   IS DISTINCT FROM OLD.proposing_finding_id
    THEN
        RAISE EXCEPTION
            'approval_requests row %: immutable column changed (S14 + Cost Advisor invariant)',
            OLD.approval_id
            USING ERRCODE = '23514';
    END IF;

    -- (b) State-machine guard, unchanged from 0029.
    IF OLD.state <> 'pending' THEN
        IF NEW.state IS DISTINCT FROM OLD.state THEN
            RAISE EXCEPTION
                'approval_requests row %: terminal state % cannot transition to %',
                OLD.approval_id, OLD.state, NEW.state
                USING ERRCODE = '23514';
        END IF;
        IF NEW.resolved_at             IS DISTINCT FROM OLD.resolved_at
            OR NEW.resolved_by_subject IS DISTINCT FROM OLD.resolved_by_subject
            OR NEW.resolved_by_issuer  IS DISTINCT FROM OLD.resolved_by_issuer
            OR NEW.resolution_reason   IS DISTINCT FROM OLD.resolution_reason
        THEN
            RAISE EXCEPTION
                'approval_requests row %: terminal-row resolution metadata is frozen',
                OLD.approval_id
                USING ERRCODE = '23514';
        END IF;
    END IF;

    RETURN NEW;
END;
$$;

COMMENT ON FUNCTION approval_requests_block_immutable_updates IS
    'S14 + Codex round-4 + Cost Advisor P0: rejects UPDATEs that would mutate any frozen column. Always-frozen now includes proposal_source, proposed_dsl_patch, proposing_finding_id so cost_advisor-authored proposals carry the same forensic guarantees as sidecar decisions.';

-- =====================================================================
-- 3) tenant_data_policy: cost_findings retention windows.
-- =====================================================================
--
-- Per spec §11.5 Q5: 90 days for `open`, 30 days for `dismissed` /
-- `fixed`. retention_sweeper (P1) reads these into its
-- CostFindingsPurge sweep kind and DELETEs rows in
-- spendguard_canonical.cost_findings past the per-tenant window.
-- cost_findings has no immutability trigger (it's a derived artifact,
-- not an audit row) so DELETE is permitted by design.

ALTER TABLE tenant_data_policy
    ADD COLUMN cost_findings_retention_days_open INT NOT NULL DEFAULT 90
        CHECK (cost_findings_retention_days_open >= 1);

ALTER TABLE tenant_data_policy
    ADD COLUMN cost_findings_retention_days_resolved INT NOT NULL DEFAULT 30
        CHECK (cost_findings_retention_days_resolved >= 1);

COMMENT ON COLUMN tenant_data_policy.cost_findings_retention_days_open IS
    'Cost Advisor §11.5 Q5: window in days for status=open cost_findings before retention_sweeper DELETEs the row. Default 90.';
COMMENT ON COLUMN tenant_data_policy.cost_findings_retention_days_resolved IS
    'Cost Advisor §11.5 Q5: window in days for status IN (dismissed, fixed, superseded) cost_findings before retention_sweeper DELETEs the row. Default 30.';
