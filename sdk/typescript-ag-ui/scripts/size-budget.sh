#!/usr/bin/env bash
# COV_D39_01 — bundle budget gate (implementation.md §3; build FAILURE, not
# a warning).
#
#   dist/index.js minified  <= 8 KB
#   dist/index.js gzipped   <= 3 KB
#   npm pack tarball        <= 25 KB
#
# Mirrors the sdk/typescript-langchain/scripts/size-budget.sh pattern with
# the D39 caps: this package is five builders, one serializer, and one
# string helper — tiny by design.

set -euo pipefail

cd "$(dirname "$0")/.."

MIN_LIMIT_BYTES=$((8 * 1024))
GZ_LIMIT_BYTES=$((3 * 1024))
TAR_LIMIT_BYTES=$((25 * 1024))

if [ ! -f dist/index.js ]; then
  echo "FAIL: dist/index.js missing — run 'pnpm run build' first" >&2
  exit 1
fi

MIN_BYTES="$(wc -c < dist/index.js | tr -d ' ')"
GZ_BYTES="$(gzip -c dist/index.js | wc -c | tr -d ' ')"

if [ "$MIN_BYTES" -gt "$MIN_LIMIT_BYTES" ]; then
  echo "FAIL: dist/index.js minified ${MIN_BYTES} bytes > limit ${MIN_LIMIT_BYTES} bytes (8 KB)" >&2
  exit 1
fi

if [ "$GZ_BYTES" -gt "$GZ_LIMIT_BYTES" ]; then
  echo "FAIL: dist/index.js gzipped ${GZ_BYTES} bytes > limit ${GZ_LIMIT_BYTES} bytes (3 KB)" >&2
  exit 1
fi

# `npm pack --dry-run --json` emits the gzipped tarball size as `.size`.
# Captured to a temp file so jq failures surface a usable error rather than
# a broken pipe.
TMP_JSON="$(mktemp -t spendguard-ag-ui-pack.XXXXXX.json)"
trap 'rm -f "$TMP_JSON"' EXIT

npm pack --dry-run --json >"$TMP_JSON"

TAR_BYTES="$(jq -r '.[0].size' "$TMP_JSON")"
FILES_COUNT="$(jq -r '.[0].entryCount' "$TMP_JSON")"

if [ -z "$TAR_BYTES" ] || [ "$TAR_BYTES" = "null" ]; then
  echo "FAIL: could not parse tarball size from npm pack output" >&2
  cat "$TMP_JSON" >&2
  exit 1
fi

if [ "$TAR_BYTES" -gt "$TAR_LIMIT_BYTES" ]; then
  echo "FAIL: npm pack tarball ${TAR_BYTES} bytes > limit ${TAR_LIMIT_BYTES} bytes (25 KB)" >&2
  echo "      files in tarball: ${FILES_COUNT}" >&2
  exit 1
fi

echo "PASS: dist/index.js minified ${MIN_BYTES} B <= ${MIN_LIMIT_BYTES} B; gzipped ${GZ_BYTES} B <= ${GZ_LIMIT_BYTES} B; tarball ${TAR_BYTES} B <= ${TAR_LIMIT_BYTES} B (${FILES_COUNT} files)"
