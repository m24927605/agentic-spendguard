-- POST_GA_08 / issue #146: belt-and-suspenders public read revoke for
-- tokenizer_t1_samples. Migration 0051 already restricted write
-- privileges and granted explicit ledger_reader_role access; this
-- forward migration makes SELECT revocation explicit for the parent and
-- existing monthly partitions.

REVOKE SELECT ON tokenizer_t1_samples FROM PUBLIC;
REVOKE SELECT ON tokenizer_t1_samples_2026_05 FROM PUBLIC;
REVOKE SELECT ON tokenizer_t1_samples_2026_06 FROM PUBLIC;
REVOKE SELECT ON tokenizer_t1_samples_2026_07 FROM PUBLIC;

COMMENT ON TABLE tokenizer_t1_samples IS
    'Tier 1 (provider count_tokens) shadow samples per tokenizer-service-spec-v1alpha1.md §4.4. Verification-only — NOT in audit chain. Retention: 90 days (cleanup job in SLICE-extra). Drift alerts cross into audit chain via signed tokenizer_drift_alert CloudEvents (not per-sample). R2 M8: PARTITION BY RANGE(sampled_at) with monthly partitions; DROP TABLE tokenizer_t1_samples_YYYYMM replaces the legacy DELETE retention path. POST_GA_08: PUBLIC has no SELECT; ledger_reader_role is the explicit read role.';

DO $$
BEGIN
    SET LOCAL search_path = pg_catalog, pg_temp;
    IF EXISTS (
        SELECT 1
          FROM information_schema.role_table_grants
         WHERE table_schema = 'public'
           AND table_name IN (
               'tokenizer_t1_samples',
               'tokenizer_t1_samples_2026_05',
               'tokenizer_t1_samples_2026_06',
               'tokenizer_t1_samples_2026_07'
           )
           AND grantee = 'PUBLIC'
           AND privilege_type = 'SELECT'
    ) THEN
        RAISE EXCEPTION 'PUBLIC still has SELECT on tokenizer_t1_samples or a current partition';
    END IF;
END $$;
