#!/bin/sh
# Build the demo contract + schema bundles in the layout the sidecar
# expects (per services/sidecar/src/bootstrap/bundles.rs):
#
#   /bundles/contract_bundle/<id>.tgz
#   /bundles/contract_bundle/<id>.tgz.sig
#   /bundles/contract_bundle/<id>.metadata.json
#   /bundles/schema_bundle/<id>.tgz
#
# Also writes a side-car env file `/bundles/runtime.env` with the
# computed sha256 of contract_bundle.tgz (the sidecar's
# CONTRACT_BUNDLE_HASH_HEX must match the bytes on disk) and the
# price_snapshot_hash (used by the demo adapter to populate
# PricingFreeze.price_snapshot_hash).
#
# All inputs come from env vars set in compose.yaml (CONTRACT_BUNDLE_ID,
# SCHEMA_BUNDLE_ID, PRICING_VERSION, etc.).

set -eu

OUT=/bundles
CONTRACT_DIR="$OUT/contract_bundle"
SCHEMA_DIR="$OUT/schema_bundle"
mkdir -p "$CONTRACT_DIR" "$SCHEMA_DIR"

# Phase 4 O3: pricing-seed-init writes /bundles/pricing.env with the
# real pricing_version + snapshot hash from the canonical DB. If
# present, source it so the freeze tuple matches what's actually in
# pricing_table. Falls back to compose env vars (legacy demo path).
if [ -f "$OUT/pricing.env" ]; then
    echo "[bundles] sourcing pricing-seed output from $OUT/pricing.env"
    set -a
    . "$OUT/pricing.env"
    set +a
fi

: "${CONTRACT_BUNDLE_ID:?required}"
: "${SCHEMA_BUNDLE_ID:?required}"
: "${PRICING_VERSION:?required}"
: "${FX_RATE_VERSION:?required}"
: "${UNIT_CONVERSION_VERSION:?required}"
: "${SIGNING_KEY_ID:?required}"

if [ -f "$OUT/runtime.env" ] && [ -f "$CONTRACT_DIR/$CONTRACT_BUNDLE_ID.tgz" ]; then
    # Phase 3 wedge: verify the existing bundle includes contract.yaml.
    # Prior Phase 2B bundles shipped contract.cel as a placeholder; an
    # upgrade-in-place needs to regenerate so the new sidecar parser
    # finds something to read.
    if tar -tzf "$CONTRACT_DIR/$CONTRACT_BUNDLE_ID.tgz" 2>/dev/null \
            | grep -q '^\./contract\.yaml$\|^contract\.yaml$'; then
        echo "[bundles] existing bundles detected (contract.yaml present), skipping regeneration"
        cat "$OUT/runtime.env"
        exit 0
    else
        echo "[bundles] existing bundle is pre-Phase-3 (no contract.yaml); regenerating"
    fi
fi

# 1. Build the demo contract bundle payload.
#    Phase 3 wedge: ship a real contract.yaml (POC subset of Contract
#    DSL §6 + §7) that the sidecar's hot-path evaluator parses at
#    startup. Existing modes (decision/invoice/release/ttl_sweep) all
#    use claim amounts well below the 1_000_000_000-atomic hard-cap, so
#    the wedge sits open-by-default for the happy path. DEMO_MODE=deny
#    sends a claim above the cap to exercise the STOP path.
echo "[bundles] writing demo contract source..."
WORK=/tmp/contract.work
rm -rf "$WORK" && mkdir -p "$WORK"

# Demo IDs:
#   contract.metadata.id     = 33333333-...
#   contract.spec.budgets[0] = SPENDGUARD_BUDGET_ID (44444444-...)
DEMO_BUDGET_ID="${DEMO_BUDGET_ID:-44444444-4444-4444-8444-444444444444}"
CONTRACT_LOGICAL_ID="${CONTRACT_LOGICAL_ID:-33333333-3333-4333-8333-333333333333}"

cat > "$WORK/contract.yaml" <<EOF
apiVersion: contract.spendguard.io/v1alpha1
kind: Contract
metadata:
  id: $CONTRACT_LOGICAL_ID
  name: demo-contract
spec:
  budgets:
    - id: $DEMO_BUDGET_ID
      limit_amount_atomic: "1000000000"
      currency: USD
      reservation_ttl_seconds: 600
      require_hard_cap: true
  rules:
    - id: hard-cap-deny
      when:
        budget_id: $DEMO_BUDGET_ID
        claim_amount_atomic_gt: "1000000000"
      then:
        decision: STOP
        reason_code: BUDGET_EXHAUSTED
EOF

cat > "$WORK/manifest.json" <<EOF
{
  "name": "demo-contract",
  "version": "1.0.0",
  "schema_bundle_id": "$SCHEMA_BUNDLE_ID"
}
EOF

# Deterministic tar: sort + zero mtime / owner so re-runs produce the
# same sha256 (sidecar CONTRACT_BUNDLE_HASH_HEX pins exact bytes).
( cd "$WORK" && tar --sort=name --owner=0 --group=0 --mtime='UTC 1970-01-01' \
    -cf - . ) | gzip -n > "$CONTRACT_DIR/$CONTRACT_BUNDLE_ID.tgz"

CONTRACT_HASH=$(sha256sum "$CONTRACT_DIR/$CONTRACT_BUNDLE_ID.tgz" | awk '{print $1}')
echo "[bundles] contract bundle sha256: $CONTRACT_HASH"

# 2. Cosign-shaped placeholder signature. POC bundle loader only checks
#    file exists + non-empty (services/sidecar/src/bootstrap/bundles.rs
#    line 56-69). Phase 1 後段 verifies real cosign.
printf 'demo-cosign-placeholder' > "$CONTRACT_DIR/$CONTRACT_BUNDLE_ID.tgz.sig"

# 3. Bundle metadata (parsed by load_contract_bundle):
#    pricing_version + price_snapshot_hash + fx_rate_version +
#    unit_conversion_version + signing_key_id.
#
# Phase 4 O3: prefer the hash computed by pricing-seed-init from the
# real canonical DB pricing_table (sourced from /bundles/pricing.env
# above). Fall back to a synthetic hash for legacy compose runs that
# don't include pricing-seed-init.
if [ -n "${PRICE_SNAPSHOT_HASH_HEX:-}" ]; then
    PRICE_SNAPSHOT_HASH="$PRICE_SNAPSHOT_HASH_HEX"
    echo "[bundles] price_snapshot from pricing-seed: $PRICE_SNAPSHOT_HASH"
else
    PRICE_SNAPSHOT_INPUT="$PRICING_VERSION:$FX_RATE_VERSION:$UNIT_CONVERSION_VERSION:demo-prices"
    PRICE_SNAPSHOT_HASH=$(printf '%s' "$PRICE_SNAPSHOT_INPUT" | sha256sum | awk '{print $1}')
    echo "[bundles] price_snapshot computed (legacy fallback): $PRICE_SNAPSHOT_HASH"
fi

cat > "$CONTRACT_DIR/$CONTRACT_BUNDLE_ID.metadata.json" <<EOF
{
  "pricing_version":         "$PRICING_VERSION",
  "price_snapshot_hash":     "$PRICE_SNAPSHOT_HASH",
  "fx_rate_version":         "$FX_RATE_VERSION",
  "unit_conversion_version": "$UNIT_CONVERSION_VERSION",
  "signing_key_id":          "$SIGNING_KEY_ID"
}
EOF

# 4. Schema bundle — opaque tarball of canonical schema JSON.
SCHEMA_WORK=/tmp/schema.work
rm -rf "$SCHEMA_WORK" && mkdir -p "$SCHEMA_WORK"
cat > "$SCHEMA_WORK/canonical.schema.json" <<'EOF'
{
  "schema_version": "spendguard.v1alpha1",
  "comment":        "demo schema bundle — minimum viable for POC sidecar startup"
}
EOF
( cd "$SCHEMA_WORK" && tar --sort=name --owner=0 --group=0 --mtime='UTC 1970-01-01' \
    -cf - . ) | gzip -n > "$SCHEMA_DIR/$SCHEMA_BUNDLE_ID.tgz"
SCHEMA_HASH=$(sha256sum "$SCHEMA_DIR/$SCHEMA_BUNDLE_ID.tgz" | awk '{print $1}')

# 5. Side-car runtime.env consumed by sidecar + demo entrypoints.
cat > "$OUT/runtime.env" <<EOF
SPENDGUARD_SIDECAR_CONTRACT_BUNDLE_HASH_HEX=$CONTRACT_HASH
SPENDGUARD_SCHEMA_BUNDLE_HASH_HEX=$SCHEMA_HASH
SPENDGUARD_PRICE_SNAPSHOT_HASH_HEX=$PRICE_SNAPSHOT_HASH
SPENDGUARD_PRICING_VERSION=$PRICING_VERSION
SPENDGUARD_FX_RATE_VERSION=$FX_RATE_VERSION
SPENDGUARD_UNIT_CONVERSION_VERSION=$UNIT_CONVERSION_VERSION
EOF

echo "[bundles] generated:"
find "$OUT" -type f -exec ls -la {} \;
echo "[bundles] runtime.env:"
cat "$OUT/runtime.env"
