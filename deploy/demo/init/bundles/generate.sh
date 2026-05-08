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

: "${CONTRACT_BUNDLE_ID:?required}"
: "${SCHEMA_BUNDLE_ID:?required}"
: "${PRICING_VERSION:?required}"
: "${FX_RATE_VERSION:?required}"
: "${UNIT_CONVERSION_VERSION:?required}"
: "${SIGNING_KEY_ID:?required}"

if [ -f "$OUT/runtime.env" ] && [ -f "$CONTRACT_DIR/$CONTRACT_BUNDLE_ID.tgz" ]; then
    echo "[bundles] existing bundles detected, skipping regeneration"
    cat "$OUT/runtime.env"
    exit 0
fi

# 1. Build the demo contract bundle payload. POC: a tar of a single
#    minimal CEL contract. Production carries the full DSL artifact set
#    + JSON schemas referenced by the contract.
echo "[bundles] writing demo contract source..."
WORK=/tmp/contract.work
rm -rf "$WORK" && mkdir -p "$WORK"
cat > "$WORK/contract.cel" <<'EOF'
// SpendGuard demo contract: allow all decisions, no DEGRADE, no STOP.
// Production contracts use the full Contract DSL with rules + budgets;
// this stub exists to satisfy the sidecar bundle-loader's hash + sig check.
package demo
allow_all = true
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
PRICE_SNAPSHOT_INPUT="$PRICING_VERSION:$FX_RATE_VERSION:$UNIT_CONVERSION_VERSION:demo-prices"
PRICE_SNAPSHOT_HASH=$(printf '%s' "$PRICE_SNAPSHOT_INPUT" | sha256sum | awk '{print $1}')
echo "[bundles] price_snapshot sha256: $PRICE_SNAPSHOT_HASH"

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
