-- =====================================================================
-- 0044: Cost Advisor DSL allowlist — `/spec/` schema correction +
--       `test` op support for identity pinning + same-index pinning
--       invariant. (CA-P3.1 / closes codex CA-P3 r2 P1 positional-
--       patch hole AND codex CA-P3.1 r1 P1 wrong-schema hole.)
-- =====================================================================
--
-- Two corrections vs. 0043's allowlist:
--
-- 1. `/spec/` PREFIX. Codex CA-P3.1 r1 caught that the contract YAML
--    parsed by `services/sidecar/src/contract/parse.rs` nests
--    budgets/rules under `spec.budgets[]` and `spec.rules[]` (per
--    `docs/site/docs/contracts/yaml.md`), with the budget identity
--    field named `id` (not `budget_id`). All 0043 paths were wrong
--    against the real schema. 0044 supersedes:
--
--    test ops:
--      /spec/budgets/<i>/id                          (UUID value)
--    replace ops:
--      /spec/budgets/<i>/limit_amount_atomic
--      /spec/budgets/<i>/reservation_ttl_seconds
--      /spec/rules/<i>/when/claim_amount_atomic_gt
--      /spec/rules/<i>/then/decision
--      /spec/rules/<i>/then/approver_role
--
-- 2. SAME-INDEX PINNING INVARIANT. Codex CA-P3 r2 P1 / CA-P3.1 r1 P2
--    flagged that allowing a bare replace on /spec/budgets/<i>/*
--    leaves positional-mutation risk if a future caller of the SP
--    skips the test op. 0044 requires that any replace op on
--    /spec/budgets/<i>/* be preceded EARLIER IN THE SAME PATCH by a
--    test op on /spec/budgets/<i>/id at the SAME index. Without the
--    test op, the validator rejects the whole patch.
--
-- 3. The `test` op fails the WHOLE RFC-6902 patch at apply time if
--    the bundle's budget at array position <i> has a different `id`
--    than the proposal expected. Operator sees an apply failure; the
--    proposal stays in approval_requests for audit, but is NOT
--    auto-applicable. With approval_requests.proposed_dsl_patch
--    frozen by 0038's immutability trigger, the operator's recourse
--    on a failed-apply proposal is to reject + manually fix the
--    bundle (cost_advisor cannot re-propose under the same
--    (finding_id, rule_version) due to ON CONFLICT idempotency).
--    This is honest v0.1 UX given the constraint that cost_advisor
--    can't load contract bundles to find the right index.

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
    idx_pat CONSTANT TEXT := '(0|[1-9][0-9]*)';
    -- `replace`-op paths (CA-P3.1: now under /spec/ matching the
    -- contract YAML schema parsed by sidecar/src/contract/parse.rs).
    replace_pattern CONSTANT TEXT :=
        '^/spec/(budgets/' || idx_pat || '/(limit_amount_atomic|reservation_ttl_seconds)|' ||
        'rules/' || idx_pat || '/(when/claim_amount_atomic_gt|then/(decision|approver_role)))$';
    -- `test`-op paths. Identity-pinning only; cannot mutate. The
    -- field is `id` per the YAML schema (NOT `budget_id`).
    test_pattern CONSTANT TEXT :=
        '^/spec/budgets/' || idx_pat || '/id$';
    -- Same-index pinning invariant: any replace on
    -- /spec/budgets/<i>/* must be preceded by a test on
    -- /spec/budgets/<i>/id at the SAME <i>.
    budget_replace_idx_pattern CONSTANT TEXT :=
        '^/spec/budgets/' || idx_pat || '/(limit_amount_atomic|reservation_ttl_seconds)$';
    leaf TEXT;
    val_text TEXT;
    val_int BIGINT;
    -- Lowercase hyphenated UUID regex (8-4-4-4-12 hex).
    uuid_pat CONSTANT TEXT :=
        '^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$';
    pinned_budget_indices INT[] := ARRAY[]::INT[];
    extracted_idx INT;
    idx_match TEXT[];
BEGIN
    IF p_patch IS NULL THEN RETURN TRUE; END IF;
    IF jsonb_typeof(p_patch) <> 'array' THEN RETURN FALSE; END IF;
    IF jsonb_array_length(p_patch) = 0 THEN RETURN FALSE; END IF;
    IF jsonb_array_length(p_patch) > 8 THEN RETURN FALSE; END IF;

    FOR elem IN SELECT * FROM jsonb_array_elements(p_patch)
    LOOP
        IF jsonb_typeof(elem) <> 'object' THEN RETURN FALSE; END IF;

        op_value := elem->>'op';
        path_value := elem->>'path';
        IF op_value IS NULL OR path_value IS NULL THEN RETURN FALSE; END IF;

        IF op_value = 'test' THEN
            IF path_value !~ test_pattern THEN RETURN FALSE; END IF;
            IF NOT (elem ? 'value') THEN RETURN FALSE; END IF;
            value_field := elem->'value';
            IF jsonb_typeof(value_field) <> 'string' THEN RETURN FALSE; END IF;
            val_text := value_field #>> '{}';
            IF val_text IS NULL OR val_text !~ uuid_pat THEN
                RETURN FALSE;
            END IF;
            -- Record this budget index as pinned for the rest of the
            -- patch (so a subsequent replace on /spec/budgets/<i>/X
            -- can verify).
            idx_match := regexp_match(path_value, '^/spec/budgets/(' || idx_pat || ')/id$');
            IF idx_match IS NULL THEN RETURN FALSE; END IF;
            extracted_idx := idx_match[1]::INT;
            pinned_budget_indices := array_append(pinned_budget_indices, extracted_idx);
            CONTINUE;
        END IF;

        IF op_value <> 'replace' THEN
            RETURN FALSE;
        END IF;

        IF path_value !~ replace_pattern THEN RETURN FALSE; END IF;

        IF NOT (elem ? 'value') THEN RETURN FALSE; END IF;
        value_field := elem->'value';

        -- Same-index pinning check: if this is a budget replace, the
        -- index must already appear in pinned_budget_indices.
        IF path_value ~ budget_replace_idx_pattern THEN
            idx_match := regexp_match(path_value, '^/spec/budgets/(' || idx_pat || ')/');
            IF idx_match IS NULL THEN RETURN FALSE; END IF;
            extracted_idx := idx_match[1]::INT;
            IF NOT (extracted_idx = ANY(pinned_budget_indices)) THEN
                RETURN FALSE;
            END IF;
        END IF;

        leaf := split_part(path_value, '/', regexp_count(path_value, '/') + 1);

        IF leaf IN ('limit_amount_atomic', 'claim_amount_atomic_gt') THEN
            IF jsonb_typeof(value_field) <> 'string' THEN RETURN FALSE; END IF;
            val_text := value_field #>> '{}';
            IF val_text IS NULL OR val_text = '' OR length(val_text) > 38 THEN
                RETURN FALSE;
            END IF;
            IF val_text !~ '^[0-9]+$' THEN RETURN FALSE; END IF;

        ELSIF leaf = 'reservation_ttl_seconds' THEN
            IF jsonb_typeof(value_field) <> 'number' THEN RETURN FALSE; END IF;
            BEGIN
                val_int := (value_field #>> '{}')::BIGINT;
            EXCEPTION WHEN OTHERS THEN
                RETURN FALSE;
            END;
            IF val_int < 1 OR val_int > 86400 THEN RETURN FALSE; END IF;

        ELSIF leaf = 'decision' THEN
            IF jsonb_typeof(value_field) <> 'string' THEN RETURN FALSE; END IF;
            val_text := value_field #>> '{}';
            IF val_text IS NULL OR val_text NOT IN
                ('STOP', 'REQUIRE_APPROVAL', 'DEGRADE', 'CONTINUE', 'SKIP')
            THEN
                RETURN FALSE;
            END IF;

        ELSIF leaf = 'approver_role' THEN
            IF jsonb_typeof(value_field) <> 'string' THEN RETURN FALSE; END IF;
            val_text := value_field #>> '{}';
            IF val_text IS NULL OR val_text = '' OR length(val_text) > 64 THEN
                RETURN FALSE;
            END IF;
            IF val_text !~ '^[A-Za-z0-9_-]+$' THEN RETURN FALSE; END IF;

        ELSE
            RETURN FALSE;
        END IF;
    END LOOP;

    RETURN TRUE;
END;
$$;

COMMENT ON FUNCTION cost_advisor_validate_proposed_dsl_patch IS
    'CA-P3 + CA-P3.1: validates an RFC-6902 patch against the cost_advisor allowlist. 5 replace paths under /spec/ (limit_amount_atomic, reservation_ttl_seconds, claim_amount_atomic_gt, decision, approver_role) + 1 test path /spec/budgets/<i>/id with UUID value. Same-index pinning invariant: budget replace ops MUST be preceded by a test on /spec/budgets/<i>/id at the same <i> earlier in the patch (codex CA-P3.1 r1 P2). IMMUTABLE PARALLEL SAFE so it can live in a CHECK constraint.';
