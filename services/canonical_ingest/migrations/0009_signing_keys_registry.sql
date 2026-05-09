-- Phase 5 GA hardening S7: signing keys registry table.
--
-- Schema-first deliverable. The current verifier (S6/S8) reads keys
-- from a filesystem-mounted PEM directory + `keys.json` manifest.
-- This table is the production-shaped surface that a future
-- `DbKeyRegistryProvider` will consume; operators can publish new
-- keys via the registry instead of having to remount Secrets and
-- restart pods.
--
-- The schema captures the spec's rotation lifecycle:
--   1. Additive — new key is added with valid_from = future time.
--   2. Cutover — producers start signing with the new key after
--      valid_from passes; old key remains valid_until = retention
--      window.
--   3. Revoke — operator flips revoked = true on the old key after
--      retention overlap closes.
--
-- The verifier MUST evaluate validity against the SIGNED event time,
-- not ingest wallclock alone (per spec review standard). When
-- DbKeyRegistryProvider lands (S7-followup), it queries this table
-- with `WHERE event_time BETWEEN valid_from AND COALESCE(valid_until, 'infinity')`.

CREATE TABLE signing_keys (
    -- Same identifier the producer's CloudEvent.signing_key_id carries.
    -- Format: ed25519:<sha256(pubkey_bytes)[..16]> for local mode,
    --         arn:aws:kms:... for KMS mode.
    key_id              TEXT NOT NULL PRIMARY KEY,

    -- Algorithm: ed25519 | kms-ed25519. Future RSA / EC variants
    -- can extend the CHECK list.
    algorithm           TEXT NOT NULL CHECK (algorithm IN
                            ('ed25519', 'kms-ed25519')),

    -- Public key material:
    --   * For ed25519: 32-byte raw verifying key.
    --   * For kms-ed25519: empty BYTEA (the KMS arn IS the key id;
    --     verifier proxies to KMS for the verify call).
    public_key          BYTEA NOT NULL,

    -- Validity window. valid_from is required; valid_until is
    -- nullable (long-lived ops keys with no expiry). Verifier
    -- compares against the signed CloudEvent.time, not ingest now().
    valid_from          TIMESTAMPTZ NOT NULL,
    valid_until         TIMESTAMPTZ,

    -- Operator-driven revocation. Distinct from valid_until expiry.
    -- An operator who revokes a key MUST also write a `signing_key_revocations`
    -- audit row (see below) so the rotation timeline has a
    -- separate, immutable trace.
    revoked             BOOLEAN     NOT NULL DEFAULT FALSE,
    revoked_at          TIMESTAMPTZ,
    revoked_by          TEXT,
    revoked_reason      TEXT,

    -- Producer identity that requested the key. Useful for audit
    -- queries like "show me all keys signed by sidecar:wl-abc".
    producer_identity   TEXT,

    -- Operational metadata.
    created_at          TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    rotated_in_at       TIMESTAMPTZ,
    rotation_predecessor TEXT REFERENCES signing_keys(key_id),

    CHECK (NOT revoked OR revoked_at IS NOT NULL),
    CHECK (valid_until IS NULL OR valid_until > valid_from)
);

CREATE INDEX signing_keys_active_idx
    ON signing_keys (algorithm, valid_from)
    WHERE NOT revoked;

CREATE INDEX signing_keys_revoked_idx
    ON signing_keys (revoked_at DESC)
    WHERE revoked;

COMMENT ON TABLE signing_keys IS
    'S7: operator-managed key registry for producer signature verification. Verifier evaluates against signed CloudEvent.time, not ingest wallclock.';

-- Append-only revocation log. Distinct from the `revoked` flag on
-- signing_keys: this captures every revocation event (including
-- accidental flips that get reverted), which the boolean flag
-- cannot.
CREATE TABLE signing_key_revocations (
    revocation_id   UUID NOT NULL DEFAULT gen_random_uuid()
                    PRIMARY KEY,
    key_id          TEXT NOT NULL REFERENCES signing_keys(key_id),
    revoked_at      TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    revoked_by      TEXT NOT NULL,
    reason          TEXT NOT NULL
);

CREATE INDEX signing_key_revocations_key_idx
    ON signing_key_revocations (key_id, revoked_at DESC);

COMMENT ON TABLE signing_key_revocations IS
    'S7: append-only revocation event log. Operators write here AND flip signing_keys.revoked at the same time.';
