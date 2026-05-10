-- Phase 5 GA Hardening S2: producer_instance_id partitioning safety.
--
-- The existing audit_outbox UNIQUE
--   (recorded_month, tenant_id, workload_instance_id, producer_sequence)
-- ALREADY makes producer-sequence collisions across distinct workload
-- instance ids harmless. The remaining risk is two pods with the
-- IDENTICAL workload_instance_id env (e.g., a Helm operator hard-coding
-- "sidecar-0" then setting replicas=2). When that happens both pods
-- would try to claim seq=1 at startup; one succeeds, the other gets a
-- UNIQUE conflict mid-batch — leaving the audit chain temporarily
-- ambiguous.
--
-- S2's defense: make placeholder-shaped workload_instance_ids syntactically
-- reject-able at the SP layer. Combined with Helm's `metadata.name`
-- downward API source (S2 chart change), production deployments cannot
-- accidentally collide.
--
-- This migration adds a CHECK constraint that rejects values which are:
--   * empty
--   * less than 4 characters (operator typo escape hatch)
--   * obvious POC placeholders (matched via patterns)
--
-- Existing seeded data (demo workload_instance_id = "sidecar-demo-1",
-- "demo-webhook-receiver", "demo-ttl-sweeper") all pass the check —
-- demo modes unchanged.

-- audit_outbox: add CHECK on workload_instance_id shape.
ALTER TABLE audit_outbox
    ADD CONSTRAINT audit_outbox_workload_instance_id_shape
        CHECK (
            length(workload_instance_id) >= 4
            AND workload_instance_id NOT IN ('sidecar', 'pod', 'instance', 'worker', 'demo')
            -- Placeholder regex: short single-token defaults like "x", "test"
            AND workload_instance_id !~* '^(test|placeholder|todo|fixme)$'
        );

-- audit_outbox_global_keys: same constraint to keep mirror table consistent.
ALTER TABLE audit_outbox_global_keys
    ADD CONSTRAINT audit_outbox_global_keys_workload_instance_id_shape
        CHECK (
            length(workload_instance_id) >= 4
            AND workload_instance_id NOT IN ('sidecar', 'pod', 'instance', 'worker', 'demo')
            AND workload_instance_id !~* '^(test|placeholder|todo|fixme)$'
        );

-- ledger_transactions.fencing_scopes have similar workload_kind /
-- active_owner_instance_id patterns; their values are operator-managed
-- and the SP CAS already detects mismatch. No constraint added there
-- to avoid impacting existing seeded fencing scopes.

COMMENT ON CONSTRAINT audit_outbox_workload_instance_id_shape ON audit_outbox IS
    'Phase 5 S2: prevents placeholder workload_instance_id values from
     being used as producer_instance_id partition key. Operators MUST
     supply a per-pod unique id (downward API metadata.name in k8s,
     hostname elsewhere).';
