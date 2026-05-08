-- Per-(region, shard) monotonic ingest offset (Trace §10.5).
-- Used to assign deterministic replay order independent of producer time.

CREATE TABLE ingest_shards (
    region_id        TEXT     NOT NULL,
    ingest_shard_id  TEXT     NOT NULL,
    last_offset      BIGINT   NOT NULL DEFAULT 0,
    created_at       TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (region_id, ingest_shard_id)
);

CREATE OR REPLACE FUNCTION next_ingest_offset(p_region TEXT, p_shard TEXT)
RETURNS BIGINT AS $$
DECLARE v_next BIGINT;
BEGIN
    INSERT INTO ingest_shards (region_id, ingest_shard_id)
    VALUES (p_region, p_shard)
    ON CONFLICT (region_id, ingest_shard_id) DO NOTHING;

    UPDATE ingest_shards
       SET last_offset = last_offset + 1
     WHERE region_id = p_region AND ingest_shard_id = p_shard
    RETURNING last_offset INTO v_next;
    RETURN v_next;
END;
$$ LANGUAGE plpgsql;
