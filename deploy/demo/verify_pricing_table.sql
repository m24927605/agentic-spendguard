-- Phase 4 O3 verification — pricing_table seed populated from YAML.
-- Run against spendguard_canonical DB.

\echo
\echo === pricing_versions ===
SELECT
    pricing_version,
    encode(price_snapshot_hash, 'hex') AS price_snapshot_hash_hex,
    row_count,
    cut_by
  FROM pricing_versions
 ORDER BY cut_at DESC
 LIMIT 3;

\echo
\echo === pricing_table row count by provider ===
SELECT provider, COUNT(*)::int AS rows
  FROM pricing_table
 GROUP BY provider
 ORDER BY provider;

\echo
\echo === sample latest pricing for OpenAI gpt-4o-mini ===
SELECT model, token_kind, price_usd_per_million
  FROM pricing_table_latest
 WHERE provider = 'openai' AND model = 'gpt-4o-mini'
 ORDER BY token_kind;

\echo
\echo === ASSERTIONS ===
DO $$
DECLARE
    v_versions   INT;
    v_rows       INT;
    v_providers  INT;
    v_oai_rows   INT;
    v_anth_rows  INT;
BEGIN
    SELECT COUNT(*) INTO v_versions FROM pricing_versions;
    IF v_versions = 0 THEN
        RAISE EXCEPTION 'O3_GATE: pricing_versions empty (pricing-seed-init never ran)';
    END IF;

    SELECT COUNT(*) INTO v_rows FROM pricing_table;
    IF v_rows < 20 THEN
        RAISE EXCEPTION 'O3_GATE: pricing_table only has % rows (expected 20+)', v_rows;
    END IF;

    SELECT COUNT(DISTINCT provider) INTO v_providers FROM pricing_table;
    IF v_providers < 3 THEN
        RAISE EXCEPTION 'O3_GATE: pricing_table only covers % providers (expected 3+)', v_providers;
    END IF;

    SELECT COUNT(*) INTO v_oai_rows
      FROM pricing_table
     WHERE provider = 'openai' AND model = 'gpt-4o-mini';
    IF v_oai_rows < 2 THEN
        RAISE EXCEPTION 'O3_GATE: gpt-4o-mini missing token_kind rows (got %)', v_oai_rows;
    END IF;

    SELECT COUNT(*) INTO v_anth_rows
      FROM pricing_table
     WHERE provider = 'anthropic';
    IF v_anth_rows = 0 THEN
        RAISE EXCEPTION 'O3_GATE: pricing_table has no Anthropic rows';
    END IF;

    RAISE NOTICE 'O3 pricing seed PASS: versions=% rows=% providers=%',
        v_versions, v_rows, v_providers;
END
$$;
