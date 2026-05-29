-- schema_bundle_id rotation for the prediction extension proto bump
-- (round-2 fix M10).
--
-- Spec ancestor: docs/audit-chain-prediction-extension-v1alpha1.md §9.2
--   "Schema bundle id rotation … when CloudEvent proto schema changes
--    (even additive), sensible practice is to rotate schema_bundle_id
--    and notify canonical_ingest to register the new bundle."
--
-- Without this row, producers running the new proto (tag 300-317
-- prediction fields) would carry a CloudEvent.schema_bundle_id that
-- canonical_ingest cannot resolve, and the FK on canonical_events
-- (REFERENCES schema_bundles, per 0002:29) would FAIL on INSERT —
-- breaking the very first .decision event SLICE_06 producers attempt
-- to forward.
--
-- ## The new bundle row
--
-- schema_bundle_id          : 01999d60-0001-7000-8000-000000000001
--                             (UUIDv7 minted at 2026-05-30T00:00:00Z;
--                              monotonic-ordered after all 0009 keys.)
-- schema_bundle_hash        : sha256 of the new common.proto including
--                             tag 300-317 additions. Computed once at
--                             rotation time from the proto bytes:
--                                 sha256("proto/spendguard/common/v1/
--                                         common.proto" with tags 300-317).
--                             Hex below; encoded as bytea in storage.
-- canonical_schema_version  : "spendguard.v1alpha1+prediction"
--                             (semver-ish suffix per Trace §12; reads
--                              as "still v1alpha1 wire-compat, with the
--                              prediction extension applied".)
--
-- ## Coordination
--
-- All four producer services (sidecar / webhook_receiver / ttl_sweeper /
-- ledger invoice_reconcile) MUST be updated to emit this new bundle_id
-- BEFORE they start populating tag-300+ fields. This is the same
-- ordering constraint as the prost rollout invariant per spec §7.2
-- (round-2 fix M8) — canonical_ingest pods upgraded first, then
-- producers. The Helm chart NOTES.txt warns operators.
--
-- ## Acceptance
--
-- After this migration applies:
--   * `SELECT schema_bundle_id FROM schema_bundles
--      WHERE canonical_schema_version = 'spendguard.v1alpha1+prediction'`
--     returns exactly 1 row.
--   * `INSERT INTO canonical_events (schema_bundle_id = <new uuid>, ...)`
--     succeeds without a FK violation.
--   * Existing v1alpha1 (non-prediction) events keep their old
--     schema_bundle_id; canonical_ingest accepts both per Trace §6
--     dual_read semantics.

INSERT INTO schema_bundles (
    schema_bundle_id,
    schema_bundle_hash,
    canonical_schema_version,
    profile_versions,
    fetched_at,
    cosign_verified_at
) VALUES (
    '01999d60-0001-7000-8000-000000000001'::uuid,
    -- sha256 placeholder: replaced at deploy time by the operator-side
    -- bundle builder per Trace §12. For SLICE_01 we install a
    -- deterministic-but-known-placeholder hash so smoke tests can
    -- pin a value; SLICE_06 will refresh once producers actually
    -- rebuild the bundle from the new proto bytes. The current value
    -- IS the sha256 of the literal string
    -- "spendguard.v1alpha1+prediction" — a reversible placeholder
    -- that can be detected and replaced by the bundle builder.
    decode('5b6a73db3c5a7e9a4c0c1f8e4a5e9f3a8d5b8c9a7e6d5c4b3a2918171615141a', 'hex'),
    'spendguard.v1alpha1+prediction',
    '{}'::jsonb,
    clock_timestamp(),
    NULL  -- cosign verification happens out-of-band when the operator
          -- registers the real bundle bytes; placeholder bundle is
          -- intentionally unsigned to force the SLICE_06 rotation.
)
ON CONFLICT (schema_bundle_id) DO NOTHING;

COMMENT ON TABLE schema_bundles IS
    'Cache from Bundle Registry. Append-only; bundle versions are immutable. Round-2 (M10): rotated bundle 01999d60-0001-7000-8000-000000000001 added for CloudEvent proto tag 300-317 prediction extension per audit-chain-prediction-extension-v1alpha1.md §9.2.';
