-- =====================================================================
-- 0042: real FK from approval_requests.proposing_finding_id
--       → cost_findings.finding_id  (CA-P1.6 / issue #56 follow-on)
-- =====================================================================
--
-- Context: 0038 added proposing_finding_id as a "soft FK" because
-- cost_findings lived in `spendguard_canonical` (cross-DB; Postgres
-- can't enforce). 0040 + 0041 (CA-P1.6) moved cost_findings into
-- `spendguard_ledger`. Now both tables live in the same DB, so we
-- promote the soft pointer to a real FK.
--
-- The protection chain we want:
--
--     approval_requests ──FK ON DELETE RESTRICT──▶ cost_findings_id_keys
--                                                          │
--                                                          ▼
--                                           ──FK ON DELETE CASCADE──▶ cost_findings
--
-- Why TWO FKs (codex CA-P1.6 r1 P1):
--   * The cost_findings PK is partitioned: (tenant_id, detected_at,
--     finding_id). Postgres FKs can only target a UNIQUE-on-FK-columns
--     target. A single-column FK on finding_id therefore needs a
--     non-partitioned mirror with PRIMARY KEY (finding_id).
--   * That mirror is `cost_findings_id_keys` below.
--   * BUT: a mirror with no FK back to cost_findings can be deleted
--     out from under approval_requests if retention runs
--     `DELETE FROM cost_findings ...` directly. The mirror would still
--     point at a dead row; the approval_requests FK against the mirror
--     would stay valid; the evidence row is gone. That re-introduces
--     the retention-driven dangling hole this PR is meant to close.
--   * Fix: add a SECOND FK from id_keys back to the partitioned
--     cost_findings (tenant_id, detected_at, finding_id) ON DELETE
--     CASCADE. Now:
--       - DELETE FROM cost_findings (unreferenced) → cascades to
--         id_keys → succeeds (no approval references the mirror, so
--         the chain terminates).
--       - DELETE FROM cost_findings (referenced) → tries to cascade
--         to id_keys → id_keys DELETE is REJECTED by the approval_
--         requests RESTRICT FK → whole DELETE aborts.
--   * That's the desired retention semantics enforced by Postgres
--     alone — no reconciler.

CREATE TABLE cost_findings_id_keys (
    finding_id   UUID NOT NULL PRIMARY KEY,
    tenant_id    UUID NOT NULL,
    detected_at  TIMESTAMPTZ NOT NULL,
    -- finding_id alone is the natural FK target; the composite
    -- (tenant_id, finding_id) is the UNIQUE target used by the
    -- tenant-scoped FK from approval_requests below.
    CONSTRAINT cost_findings_id_keys_tenant_idx
        UNIQUE (tenant_id, finding_id),
    -- Back-pointer FK into the partitioned cost_findings row. ON
    -- DELETE CASCADE so retention DELETEs on cost_findings clean the
    -- mirror naturally; but when a cost_findings row has any
    -- approval_requests pointing at it, the cascade is blocked by
    -- the RESTRICT FK on approval_requests below.
    CONSTRAINT cost_findings_id_keys_cost_findings_fkey
        FOREIGN KEY (tenant_id, detected_at, finding_id)
        REFERENCES cost_findings (tenant_id, detected_at, finding_id)
        ON DELETE CASCADE
);

CREATE INDEX cost_findings_id_keys_partition_idx
    ON cost_findings_id_keys (tenant_id, detected_at);

COMMENT ON TABLE cost_findings_id_keys IS
    'CA-P1.6: non-partitioned mirror of cost_findings.finding_id used as the FK target for approval_requests.proposing_finding_id. Maintained in lockstep by cost_findings_upsert(). Two FKs anchor the protection chain: approval_requests --RESTRICT--> id_keys --CASCADE--> cost_findings. Retention DELETEs on unreferenced findings cascade through the mirror cleanly; DELETEs on referenced findings are rejected by the RESTRICT step.';

-- ---------------------------------------------------------------------
-- Real FK on approval_requests.proposing_finding_id.
-- ---------------------------------------------------------------------
--
-- NOT VALID + VALIDATE pattern (same as 0011 + 0038): the ADD
-- CONSTRAINT itself is metadata-only; VALIDATE CONSTRAINT runs
-- the scan under SHARE UPDATE EXCLUSIVE.
--
-- Tenant-scoped composite FK (codex CA-P1.6 r1 P2): the single-column
-- FK on (proposing_finding_id) would let tenant A's approval row
-- reference tenant B's finding if a writer bug supplies the wrong
-- UUID. The composite (tenant_id, proposing_finding_id) → id_keys
-- (tenant_id, finding_id) prevents that — and id_keys already has
-- UNIQUE (tenant_id, finding_id) as a valid FK target.

ALTER TABLE approval_requests
    ADD CONSTRAINT approval_requests_proposing_finding_id_fkey
    FOREIGN KEY (tenant_id, proposing_finding_id)
    REFERENCES cost_findings_id_keys (tenant_id, finding_id)
    ON DELETE RESTRICT
    NOT VALID;

-- Backfill cost_findings_id_keys for any pre-existing cost_findings
-- rows (codex CA-P1.6 r4 YELLOW: moved BEFORE the VALIDATE so an
-- upgrade DB with existing approval_requests.proposing_finding_id
-- rows passes validation). Fresh installs (v0.1 greenfield) have no
-- rows yet — this is a no-op there.
INSERT INTO cost_findings_id_keys (finding_id, tenant_id, detected_at)
    SELECT cf.finding_id, cf.tenant_id, cf.detected_at
      FROM cost_findings cf
     ON CONFLICT (finding_id) DO NOTHING;

ALTER TABLE approval_requests
    VALIDATE CONSTRAINT approval_requests_proposing_finding_id_fkey;

-- Child-side FK index (codex CA-P1.6 r1 P2). Postgres does not
-- auto-index FK children; without this, every DELETE/RESTRICT check
-- on cost_findings_id_keys would seq-scan the audit-sized
-- approval_requests table. Partial index keeps it cheap since most
-- approval_requests rows have proposing_finding_id IS NULL
-- (sidecar_decision flow doesn't set it).
CREATE INDEX approval_requests_proposing_finding_id_fkey_idx
    ON approval_requests (tenant_id, proposing_finding_id)
    WHERE proposing_finding_id IS NOT NULL;

-- Override the 0038 column comment that still claimed cross-DB +
-- unenforced (codex CA-P1.6 r1 P3).
COMMENT ON COLUMN approval_requests.proposing_finding_id IS
    'Cost Advisor / CA-P1.6: FK into spendguard_ledger.cost_findings_id_keys (finding_id), enforced via the composite (tenant_id, proposing_finding_id) → (tenant_id, finding_id) constraint. Was a soft cross-DB pointer in 0038 (cost_findings then lived in spendguard_canonical); 0040/0041/0042 moved the table over and added the real FK. ON DELETE RESTRICT semantics: retention on cost_findings is rejected when any approval_request references the finding.';

-- ---------------------------------------------------------------------
-- Update cost_findings_upsert SP to maintain the new mirror.
-- ---------------------------------------------------------------------
--
-- The SP from 0040 inserts/updates cost_findings + cost_findings_
-- fingerprint_keys. Now it ALSO writes to cost_findings_id_keys so
-- the FK from approval_requests has a target row to reference.
--
-- ORDER MATTERS (codex CA-P1.6 r1 P1): cost_findings_id_keys has a
-- back-FK to cost_findings ON DELETE CASCADE. INSERTs into id_keys
-- must therefore happen AFTER INSERT/UPDATE on cost_findings (parent
-- must exist before child references it).
--
-- ON CONFLICT semantics:
--   * 'inserted' path: INSERT cost_findings, then INSERT id_keys.
--   * 'updated' path: id_keys row already exists; same finding_id,
--     no change needed.
--   * 'reinstated' path: the canonical row was deleted out-of-band
--     and we're re-creating it under a NEW finding_id (per the
--     stale-mirror self-heal in 0040). The OLD finding_id's id_keys
--     row must be removed; if any approval_requests pointed at it,
--     that's an invariant breach and the RESTRICT FK rejects the
--     DELETE, aborting the SP. Order:
--       (a) DELETE old id_keys row (may abort via FK violation —
--           that's the invariant check)
--       (b) UPDATE fingerprint_keys to point at new finding_id
--       (c) INSERT new cost_findings row (parent for the new id_keys)
--       (d) INSERT new id_keys row (child, points at the new parent)

CREATE OR REPLACE FUNCTION cost_findings_upsert(
    p_finding_id          UUID,
    p_fingerprint         CHAR(64),
    p_tenant_id           UUID,
    p_detected_at         TIMESTAMPTZ,
    p_rule_id             TEXT,
    p_rule_version        INT,
    p_category            TEXT,
    p_severity            TEXT,
    p_confidence          NUMERIC,
    p_agent_id            TEXT,
    p_run_id              TEXT,
    p_contract_bundle_id  TEXT,
    p_evidence            JSONB,
    p_estimated_waste     BIGINT,
    p_sample_decision_ids UUID[]
) RETURNS TABLE (
    outcome           TEXT,
    finding_id        UUID,
    finding_detected_at TIMESTAMPTZ
) LANGUAGE plpgsql AS $$
DECLARE
    v_claimed_finding_id UUID;
    v_existing_finding_id UUID;
    v_existing_detected_at TIMESTAMPTZ;
BEGIN
    -- All table refs are schema-qualified (public.X) per codex CA-P1.6
    -- r3 P1: SECURITY DEFINER hardening. Unqualified names can be
    -- shadowed by pg_temp objects a malicious caller creates before
    -- invoking; schema-qualifying defeats the attack.

    -- Phase 1: try to claim the fingerprint slot.
    INSERT INTO public.cost_findings_fingerprint_keys
        (tenant_id, fingerprint, finding_id, detected_at)
        VALUES (p_tenant_id, p_fingerprint, p_finding_id, p_detected_at)
        ON CONFLICT (tenant_id, fingerprint) DO NOTHING
        RETURNING public.cost_findings_fingerprint_keys.finding_id
        INTO v_claimed_finding_id;

    IF v_claimed_finding_id IS NOT NULL THEN
        INSERT INTO public.cost_findings (
            finding_id, fingerprint, tenant_id, detected_at,
            rule_id, rule_version, category, severity, confidence,
            agent_id, run_id, contract_bundle_id,
            evidence, estimated_waste_micros_usd, sample_decision_ids
        ) VALUES (
            p_finding_id, p_fingerprint, p_tenant_id, p_detected_at,
            p_rule_id, p_rule_version, p_category, p_severity, p_confidence,
            p_agent_id, p_run_id, p_contract_bundle_id,
            p_evidence, p_estimated_waste, p_sample_decision_ids
        );
        INSERT INTO public.cost_findings_id_keys (finding_id, tenant_id, detected_at)
        VALUES (p_finding_id, p_tenant_id, p_detected_at);
        RETURN QUERY SELECT 'inserted'::TEXT, p_finding_id, p_detected_at;
        RETURN;
    END IF;

    SELECT m.finding_id, m.detected_at
      INTO v_existing_finding_id, v_existing_detected_at
      FROM public.cost_findings_fingerprint_keys m
     WHERE m.tenant_id = p_tenant_id AND m.fingerprint = p_fingerprint
     FOR UPDATE;

    UPDATE public.cost_findings SET
        evidence                   = p_evidence,
        severity                   = p_severity,
        confidence                 = p_confidence,
        estimated_waste_micros_usd = p_estimated_waste,
        sample_decision_ids        = p_sample_decision_ids
     WHERE public.cost_findings.tenant_id   = p_tenant_id
       AND public.cost_findings.detected_at = v_existing_detected_at
       AND public.cost_findings.finding_id  = v_existing_finding_id;

    IF NOT FOUND THEN
        DELETE FROM public.cost_findings_id_keys k WHERE k.finding_id = v_existing_finding_id;

        UPDATE public.cost_findings_fingerprint_keys
           SET finding_id = p_finding_id,
               detected_at = p_detected_at
         WHERE tenant_id = p_tenant_id AND fingerprint = p_fingerprint;

        INSERT INTO public.cost_findings (
            finding_id, fingerprint, tenant_id, detected_at,
            rule_id, rule_version, category, severity, confidence,
            agent_id, run_id, contract_bundle_id,
            evidence, estimated_waste_micros_usd, sample_decision_ids
        ) VALUES (
            p_finding_id, p_fingerprint, p_tenant_id, p_detected_at,
            p_rule_id, p_rule_version, p_category, p_severity, p_confidence,
            p_agent_id, p_run_id, p_contract_bundle_id,
            p_evidence, p_estimated_waste, p_sample_decision_ids
        );
        INSERT INTO public.cost_findings_id_keys (finding_id, tenant_id, detected_at)
        VALUES (p_finding_id, p_tenant_id, p_detected_at);

        RETURN QUERY SELECT 'reinstated'::TEXT, p_finding_id, p_detected_at;
        RETURN;
    END IF;

    RETURN QUERY SELECT 'updated'::TEXT, v_existing_finding_id, v_existing_detected_at;
END;
$$;

COMMENT ON FUNCTION cost_findings_upsert IS
    'Cost Advisor §11.5 A1 + codex r6 P1 + CA-P1.6: SOLE legal writer for cost_findings. Atomically claims (tenant_id, fingerprint) in the mirror, INSERTs/UPDATEs/reinstates the canonical partition row, AND maintains cost_findings_id_keys mirror in the correct order (parent before child) for the FK chain approval_requests --RESTRICT--> id_keys --CASCADE--> cost_findings. The reinstated DELETE on id_keys is blocked by the RESTRICT FK if any approval_requests row still references the stale finding — surfacing the invariant breach loudly.';

-- ---------------------------------------------------------------------
-- Update cost_findings_ensure_next_month_partition() to maintain
-- cost_findings_id_keys when draining DEFAULT-partition rows.
-- (codex CA-P1.6 r2 P1)
-- ---------------------------------------------------------------------
--
-- The 0040 drain does DELETE FROM cost_findings_default + re-INSERT
-- via the partition root. After 0042's back-FK with ON DELETE
-- CASCADE, the DELETE cascades to cost_findings_id_keys; the
-- re-INSERT into cost_findings does NOT re-create the id_keys rows.
-- Without this override, the drain leaves the mirror permanently
-- orphaned for any drained row.
--
-- Fix: after re-INSERTing cost_findings, also re-INSERT
-- cost_findings_id_keys from the staged _drain rows.
--
-- Referenced-row edge case: if any DEFAULT-partition row has an
-- approval_requests pointing at it via id_keys, the DELETE FROM
-- cost_findings_default cascades to id_keys → RESTRICT fires on
-- approval_requests → whole DELETE aborts. The drain function fails
-- loudly with foreign_key_violation. Codex CA-P1.6 r4 corrected the
-- v1 framing here: this is NOT a routine operator-remediation case
-- (proposing_finding_id is frozen by the immutability trigger so
-- resolving the approval does NOT clear the FK reference). If this
-- aborts, it indicates a deeper invariant breach to investigate:
-- cost_advisor should not be proposing approvals against findings
-- in DEFAULT.

CREATE OR REPLACE FUNCTION cost_findings_ensure_next_month_partition()
    RETURNS TEXT LANGUAGE plpgsql AS $$
DECLARE
    v_next_start DATE;
    v_next_end   DATE;
    v_part_name  TEXT;
    v_drained    INT;
BEGIN
    v_next_start := date_trunc('month', now() + INTERVAL '1 month')::DATE;
    v_next_end   := v_next_start + INTERVAL '1 month';
    v_part_name  := 'cost_findings_' || to_char(v_next_start, 'YYYY_MM');

    IF to_regclass('public.' || v_part_name) IS NOT NULL THEN
        RETURN NULL;
    END IF;

    LOCK TABLE public.cost_findings IN ACCESS EXCLUSIVE MODE;

    IF to_regclass('public.' || v_part_name) IS NOT NULL THEN
        RETURN NULL;
    END IF;

    CREATE TEMP TABLE _drain ON COMMIT DROP AS
      SELECT * FROM public.cost_findings_default
       WHERE detected_at >= v_next_start AND detected_at < v_next_end;

    GET DIAGNOSTICS v_drained = ROW_COUNT;

    IF v_drained > 0 THEN
        -- The DELETE here will cascade to cost_findings_id_keys; if any
        -- id_keys row has an approval_requests pointing at it, the
        -- cascade trips the RESTRICT FK on approval_requests and this
        -- whole statement aborts. That is intentional — we don't want
        -- the drain to silently orphan an in-flight proposal.
        --
        -- Why this should never happen in practice (codex CA-P1.6 r4
        -- YELLOW correction): cost_advisor only proposes approvals
        -- for findings emitted by current-day rule runs, so DEFAULT-
        -- partition rows (which represent future-month data the
        -- partition manager hasn't created a slot for) should never
        -- be referenced. If this abort fires, it's an invariant
        -- breach to investigate — NOT a routine remediation path.
        -- (approval_requests.proposing_finding_id is frozen by the
        -- immutability trigger, so resolving the approval does NOT
        -- clear the reference; the only way to break the cycle is
        -- to investigate why a DEFAULT-partition row got proposed.)
        DELETE FROM public.cost_findings_default
         WHERE detected_at >= v_next_start AND detected_at < v_next_end;
    END IF;

    EXECUTE format(
        'CREATE TABLE public.%I PARTITION OF public.cost_findings FOR VALUES FROM (%L) TO (%L)',
        v_part_name, v_next_start, v_next_end
    );

    IF v_drained > 0 THEN
        -- Re-insert cost_findings first (parent), then id_keys (child).
        -- The CASCADE above wiped id_keys for the drained rows so we
        -- repopulate from _drain.
        INSERT INTO public.cost_findings SELECT * FROM _drain;
        INSERT INTO public.cost_findings_id_keys (finding_id, tenant_id, detected_at)
          SELECT finding_id, tenant_id, detected_at FROM _drain;
    END IF;

    RETURN v_part_name;
END;
$$;

COMMENT ON FUNCTION cost_findings_ensure_next_month_partition IS
    'Cost Advisor P0 + CA-P1.6: idempotent forward-partition creator. If rows already landed in DEFAULT for the target month they are MOVED into the new partition (codex r5 P1-6 — no data loss). Maintains cost_findings_id_keys mirror across the drain (codex CA-P1.6 r2 P1). Holds ACCESS EXCLUSIVE on cost_findings briefly during the rare backfill case. Aborts cleanly via foreign_key_violation if any DEFAULT-partition row is referenced by approval_requests — this represents an invariant breach (cost_advisor should never propose against DEFAULT-partition rows) and requires investigation, not routine remediation.';

-- ---------------------------------------------------------------------
-- SECURITY DEFINER on cost_findings_upsert
-- (codex CA-P1.6 r2 P2 + r3 P1 — least-privilege grants + hardening)
-- ---------------------------------------------------------------------
--
-- The documented grant block in control-plane-integration.md §5.1
-- gives the cost_advisor_application_role only SELECT + EXECUTE on
-- the SP, NOT direct INSERT/UPDATE on cost_findings or the mirrors.
-- Without SECURITY DEFINER, the SP runs with caller privileges and
-- the internal INSERT/UPDATE statements fail with insufficient_
-- privilege.
--
-- Hardening (codex r3 P1):
--   * search_path = pg_catalog (NOT pg_catalog,public): defeats
--     temp-schema shadow attacks. The SP body schema-qualifies every
--     relation as public.X so it doesn't need public on search_path.
--   * REVOKE EXECUTE ON FUNCTION ... FROM PUBLIC: Postgres default
--     grants EXECUTE to PUBLIC; for a SECURITY DEFINER on the audit
--     surface that's an unbounded privilege-escalation vector.
--     P1 will GRANT EXECUTE TO cost_advisor_application_role; until
--     then only the function owner (postgres / migration runner) can
--     invoke it.

-- search_path = pg_catalog, pg_temp (codex CA-P1.6 r4 P1):
-- if pg_temp is NOT explicitly listed, Postgres places it FIRST
-- implicitly — a temp schema "text"/"int" composite type could
-- shadow the casts at lines like 'inserted'::TEXT. Explicitly
-- putting pg_temp LAST defeats that.
ALTER FUNCTION cost_findings_upsert(
    UUID, CHAR, UUID, TIMESTAMPTZ, TEXT, INT, TEXT, TEXT, NUMERIC,
    TEXT, TEXT, TEXT, JSONB, BIGINT, UUID[]
) SECURITY DEFINER SET search_path = pg_catalog, pg_temp;

REVOKE ALL ON FUNCTION cost_findings_upsert(
    UUID, CHAR, UUID, TIMESTAMPTZ, TEXT, INT, TEXT, TEXT, NUMERIC,
    TEXT, TEXT, TEXT, JSONB, BIGINT, UUID[]
) FROM PUBLIC;

-- (Backfill of cost_findings_id_keys for pre-existing rows is
-- performed above, BETWEEN ADD CONSTRAINT NOT VALID and VALIDATE,
-- so that upgrade DBs with pre-existing approval_requests pass
-- validation. Greenfield installs see this as a no-op.)
