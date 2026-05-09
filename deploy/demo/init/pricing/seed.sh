#!/bin/sh
# Phase 4 O3 — load deploy/demo/init/pricing/seed.yaml into the canonical
# DB's pricing_table. Idempotent on re-run (DELETE + INSERT under one
# pricing_version). Run after canonical_ingest migrations + before
# bundles-init so generate.sh can compute price_snapshot_hash from the
# real table contents.
#
# Outputs to stdout the (pricing_version, price_snapshot_hash_hex,
# row_count) tuple so callers can sanity-check.

set -eu

PRICING_YAML="${PRICING_YAML:-/seed.yaml}"
PRICING_DB="${PRICING_DB:-spendguard_canonical}"

if [ ! -f "$PRICING_YAML" ]; then
    echo "[pricing-seed] FATAL: $PRICING_YAML not found" >&2
    exit 1
fi

# Postgres race: docker compose marks postgres "healthy" via the
# health-check command, but the TCP socket can lag behind. Retry
# connect a few times before giving up so a fresh `compose up` doesn't
# fail on the first run.
PG_RETRIES="${PG_RETRIES:-30}"
PG_RETRY_DELAY="${PG_RETRY_DELAY:-1}"
i=0
until psql -h "${POSTGRES_HOST:-postgres}" \
           -U "${POSTGRES_USER:-spendguard}" \
           -d "${PRICING_DB:-spendguard_canonical}" \
           -c 'SELECT 1' >/dev/null 2>&1; do
    i=$((i + 1))
    if [ "$i" -ge "$PG_RETRIES" ]; then
        echo "[pricing-seed] FATAL: postgres unreachable after $PG_RETRIES tries" >&2
        exit 3
    fi
    echo "[pricing-seed] postgres not ready (attempt $i); sleeping $PG_RETRY_DELAY"
    sleep "$PG_RETRY_DELAY"
done

# Use python3 because the alpine image already has it; pure bash YAML
# parsing is fragile. Build a TSV that psql COPY can ingest.
TSV_FILE=$(mktemp /tmp/pricing.XXXXXX.tsv)
trap 'rm -f "$TSV_FILE"' EXIT

PRICING_VERSION=$(python3 -c '
import sys, yaml
with open(sys.argv[1]) as f:
    d = yaml.safe_load(f)
print(d["pricing_version"])
' "$PRICING_YAML")

CUT_BY=$(python3 -c '
import sys, yaml
with open(sys.argv[1]) as f:
    d = yaml.safe_load(f)
print(d.get("cut_by", "manual:unknown"))
' "$PRICING_YAML")

SOURCES=$(python3 -c '
import sys, yaml
with open(sys.argv[1]) as f:
    d = yaml.safe_load(f)
print("{" + ",".join(d.get("sources_used", ["manual"])) + "}")
' "$PRICING_YAML")

# Render YAML rows to two TSVs:
#   $TSV_FILE         — canonical content for hashing (no timestamps)
#   ${TSV_FILE}.full  — full row including fetched_at + source for COPY
# This split keeps price_snapshot_hash a function of *content only* so a
# re-run under the same pricing_version produces a byte-identical hash
# (otherwise wallclock drift would break idempotency).
TSV_CONTENT="$TSV_FILE"
TSV_FULL="${TSV_FILE}.full"

python3 - "$PRICING_YAML" "$PRICING_VERSION" "$TSV_CONTENT" "$TSV_FULL" <<'PYEOF'
import sys, yaml, datetime
yaml_path, pv, tsv_content, tsv_full = sys.argv[1:5]
with open(yaml_path) as f:
    d = yaml.safe_load(f)
now = datetime.datetime.now(datetime.timezone.utc).strftime("%Y-%m-%d %H:%M:%S+00")
sources = d.get("sources_used", ["manual"])
default_source = sources[0] if sources else "manual"

content_rows = []
full_rows = []
for p in d["prices"]:
    src = p.get("source", default_source)
    # Canonical content (deterministic across runs):
    content_rows.append("\t".join([
        pv, p["provider"], p["model"], p["token_kind"],
        str(p["price_usd_per_million"]), src,
    ]))
    # Full row for psql COPY (includes wallclock fetched_at):
    full_rows.append("\t".join([
        pv, p["provider"], p["model"], p["token_kind"],
        str(p["price_usd_per_million"]), now, src,
    ]))

with open(tsv_content, "w") as f:
    f.write("\n".join(sorted(content_rows)) + "\n")
with open(tsv_full, "w") as f:
    f.write("\n".join(full_rows) + "\n")
PYEOF

trap 'rm -f "$TSV_CONTENT" "$TSV_FULL"' EXIT

ROW_COUNT=$(wc -l <"$TSV_FULL" | tr -d ' ')

# Hash content-only (already sorted); wallclock-free → same input YAML
# always produces same hash.
PRICE_SNAPSHOT_HASH=$(sha256sum "$TSV_CONTENT" | awk '{print $1}')

# Idempotency: if pricing_versions row already exists for this version,
# require the hash to match (otherwise refuse — content drift under same
# version is a misconfiguration).
EXISTING_HASH=$(psql -h "${POSTGRES_HOST:-postgres}" -U "${POSTGRES_USER:-spendguard}" -d "$PRICING_DB" -tAc "
SELECT encode(price_snapshot_hash, 'hex')
  FROM pricing_versions
 WHERE pricing_version = '$PRICING_VERSION';
" 2>/dev/null || true)

if [ -n "$EXISTING_HASH" ]; then
    if [ "$EXISTING_HASH" != "$PRICE_SNAPSHOT_HASH" ]; then
        echo "[pricing-seed] FATAL: pricing_version $PRICING_VERSION already exists with different hash" >&2
        echo "  existing: $EXISTING_HASH" >&2
        echo "  computed: $PRICE_SNAPSHOT_HASH" >&2
        exit 2
    fi
    echo "[pricing-seed] pricing_version=$PRICING_VERSION already loaded (hash matches), skipping"
    echo "PRICING_VERSION=$PRICING_VERSION"
    echo "PRICE_SNAPSHOT_HASH=$PRICE_SNAPSHOT_HASH"
    echo "ROW_COUNT=$ROW_COUNT"
    exit 0
fi

# Atomic load: pricing_versions + pricing_table together.
psql -h "${POSTGRES_HOST:-postgres}" -U "${POSTGRES_USER:-spendguard}" -d "$PRICING_DB" -v ON_ERROR_STOP=1 <<SQL
BEGIN;

INSERT INTO pricing_versions
    (pricing_version, price_snapshot_hash, row_count, cut_at, cut_by, sources_used)
VALUES
    ('$PRICING_VERSION', decode('$PRICE_SNAPSHOT_HASH', 'hex'), $ROW_COUNT,
     clock_timestamp(), '$CUT_BY', '$SOURCES');

\\copy pricing_table (pricing_version, provider, model, token_kind, price_usd_per_million, fetched_at, source) FROM '$TSV_FULL' WITH (FORMAT text, DELIMITER E'\\t');

COMMIT;
SQL

echo "[pricing-seed] loaded pricing_version=$PRICING_VERSION rows=$ROW_COUNT hash=$PRICE_SNAPSHOT_HASH"

# Mirror the pricing snapshot into the ledger DB's pricing_snapshots
# table. The ledger SP validates `(pricing_version, price_snapshot_hash,
# fx_rate_version, unit_conversion_version)` against this table on every
# ReserveSet — without a row, ReserveSet fails with PRICING_VERSION_UNKNOWN.
LEDGER_DB="${LEDGER_DB:-spendguard_ledger}"
FX_RATE_VERSION="${FX_RATE_VERSION:-demo-fx-v1}"
UNIT_CONVERSION_VERSION="${UNIT_CONVERSION_VERSION:-demo-units-v1}"
SIGNING_KEY_ID="${SIGNING_KEY_ID:-demo-key-1}"

psql -h "${POSTGRES_HOST:-postgres}" -U "${POSTGRES_USER:-spendguard}" -d "$LEDGER_DB" -v ON_ERROR_STOP=1 <<SQL
INSERT INTO pricing_snapshots (
    pricing_version, price_snapshot_hash, fx_rate_version,
    unit_conversion_version, schema_json, signature, signing_key_id,
    deployed_by
) VALUES (
    '$PRICING_VERSION',
    decode('$PRICE_SNAPSHOT_HASH', 'hex'),
    '$FX_RATE_VERSION',
    '$UNIT_CONVERSION_VERSION',
    '{"source": "pricing-seed-init", "row_count": $ROW_COUNT}'::JSONB,
    decode('00', 'hex'),
    '$SIGNING_KEY_ID',
    'pricing-seed-init'
) ON CONFLICT DO NOTHING;
SQL
echo "[pricing-seed] mirrored pricing_snapshot to ledger DB"

# Hand off to bundles-init via shared volume. generate.sh sources this
# file (when present) before computing PRICE_SNAPSHOT_HASH itself.
PRICING_OUT="${PRICING_OUT:-/bundles/pricing.env}"
mkdir -p "$(dirname "$PRICING_OUT")"
{
    printf 'PRICING_VERSION=%s\n' "$PRICING_VERSION"
    printf 'PRICE_SNAPSHOT_HASH_HEX=%s\n' "$PRICE_SNAPSHOT_HASH"
    printf 'PRICING_ROW_COUNT=%s\n' "$ROW_COUNT"
} >"$PRICING_OUT"
echo "[pricing-seed] wrote $PRICING_OUT"
