#!/usr/bin/env bash
# fetch-pricing-snapshot.sh — pull a pricing snapshot from the Platform
# Pricing Authority DB at contract bundle build time (Stage 2 §9.4 cold path).
#
# Usage:
#   fetch-pricing-snapshot.sh <pricing_version> <fx_rate_version> <unit_conversion_version>
# Outputs the canonical JSON snapshot on stdout.
#
# The Platform Pricing DB DSN comes from $PLATFORM_PRICING_DSN. The query
# returns the pricing schema + fx rates + unit conversions matching the
# requested versions.

set -euo pipefail

if [[ $# -ne 3 ]]; then
  echo "usage: $0 <pricing_version> <fx_rate_version> <unit_conversion_version>" >&2
  exit 2
fi

PRICING_VERSION="$1"
FX_VERSION="$2"
UC_VERSION="$3"

if [[ -z "${PLATFORM_PRICING_DSN:-}" ]]; then
  echo "PLATFORM_PRICING_DSN env var required" >&2
  exit 2
fi

# Use psql for POC; production should use a typed CLI that emits canonical
# JSON (sort keys, no whitespace) via something like jq -S -c.
psql "${PLATFORM_PRICING_DSN}" \
  --no-psqlrc --pset=tuples_only --pset=format=unaligned \
  -v pv="${PRICING_VERSION}" \
  -v fxv="${FX_VERSION}" \
  -v ucv="${UC_VERSION}" <<'SQL' | jq -S -c '.'
SELECT jsonb_build_object(
  'pricing_version',          pv.pricing_version,
  'price_snapshot_hash',      encode(pv.price_snapshot_hash, 'hex'),
  'fx_rate_version',          fxv.fx_rate_version,
  'unit_conversion_version',  ucv.unit_conversion_version,
  'pricing_schema',           pv.schema,
  'fx_rates',                 fxv.rates,
  'unit_conversions',         ucv.conversions
)
FROM pricing_versions pv,
     fx_rate_versions fxv,
     unit_conversion_versions ucv
WHERE pv.pricing_version          = :'pv'
  AND fxv.fx_rate_version         = :'fxv'
  AND ucv.unit_conversion_version = :'ucv';
SQL
