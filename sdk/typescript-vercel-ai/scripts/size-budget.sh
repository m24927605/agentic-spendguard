#!/usr/bin/env bash
# D06 SLICE 8 — npm pack size budget gate.
#
# Asserts the final published tarball stays below the 50 KB budget. The
# middleware is thin glue (middleware.ts + wrapper.ts + ids.ts + options.ts
# + mastra.ts alias); the budget head-room covers the `ai` peer-import
# surface and a small amount of headroom for additive optional fields
# landing in v0.2.x (windowInstanceId / unit / pricing / claimEstimator —
# see CHANGELOG.md Known limitations).
#
# Budget: 50 KB (gzipped tarball). `npm pack` emits the gzipped tarball
# size as `.size` in --json output. The D06 design (`design.md` §slice 6
# title) calls 40 KB the design intent; 50 KB gives margin for the
# additive options surface + the Mastra subpath alias entry without
# thrashing the gate.
#
# Mirrors `sdk/typescript-langchain/scripts/size-budget.sh` field-for-field.

set -euo pipefail

cd "$(dirname "$0")/.."

LIMIT_BYTES=$((50 * 1024))

# `npm pack --dry-run --json` writes the manifest to stdout. We capture it
# to a temp file rather than piping so jq failures surface a usable error
# rather than a broken pipe.
TMP_JSON="$(mktemp -t spendguard-vercel-ai-pack.XXXXXX.json)"
trap 'rm -f "$TMP_JSON"' EXIT

npm pack --dry-run --json >"$TMP_JSON"

# `.[0].size` is the gzipped tarball size in bytes per `npm pack --json`
# schema. The `unpackedSize` field is the post-extraction size and is NOT
# the gate (it includes the source maps which are themselves never
# published).
SIZE_BYTES="$(jq -r '.[0].size' "$TMP_JSON")"
FILES_COUNT="$(jq -r '.[0].entryCount' "$TMP_JSON")"

if [ -z "$SIZE_BYTES" ] || [ "$SIZE_BYTES" = "null" ]; then
  echo "FAIL: could not parse tarball size from npm pack output" >&2
  cat "$TMP_JSON" >&2
  exit 1
fi

LIMIT_KB=$((LIMIT_BYTES / 1024))
SIZE_KB=$((SIZE_BYTES / 1024))

if [ "$SIZE_BYTES" -gt "$LIMIT_BYTES" ]; then
  echo "FAIL: npm pack tarball ${SIZE_BYTES} bytes (${SIZE_KB} KB) > limit ${LIMIT_BYTES} bytes (${LIMIT_KB} KB)" >&2
  echo "      files in tarball: ${FILES_COUNT}" >&2
  echo "      run 'npm pack --dry-run' locally to inspect contents." >&2
  exit 1
fi

echo "PASS: npm pack tarball ${SIZE_BYTES} bytes (${SIZE_KB} KB) ≤ ${LIMIT_KB} KB (${FILES_COUNT} files)"
