#!/usr/bin/env bash
# COV_D38_01 — bundle budget gate (implementation.md §2; build FAILURE, not
# a warning).
#
#   dist/index.js minified  <= 40 KB
#   dist/index.js gzipped   <= 12 KB
#
# D04 parity — thin glue; `@mastra/core` and `@spendguard/sdk` are
# externalized peers and must never be inlined. Budget breach fails the
# build via `prepublishOnly`. Executed as a ship gate (A2.5) in COV_D38_06;
# this slice ships the script and the wiring.
#
# Copied from the `sdk/typescript-langchain/scripts/size-budget.sh` pattern
# (local copy — the cross-package relative path is brittle), with the gate
# moved to the minified/gzipped dist/index.js sizes the D38 budget is
# defined on (same shape as sdk/typescript-ag-ui/scripts/size-budget.sh).

set -euo pipefail

cd "$(dirname "$0")/.."

MIN_LIMIT_BYTES=$((40 * 1024))
GZ_LIMIT_BYTES=$((12 * 1024))

if [ ! -f dist/index.js ]; then
  echo "FAIL: dist/index.js missing — run 'pnpm run build' first" >&2
  exit 1
fi

MIN_BYTES="$(wc -c < dist/index.js | tr -d ' ')"
GZ_BYTES="$(gzip -c dist/index.js | wc -c | tr -d ' ')"

if [ "$MIN_BYTES" -gt "$MIN_LIMIT_BYTES" ]; then
  echo "FAIL: dist/index.js minified ${MIN_BYTES} bytes > limit ${MIN_LIMIT_BYTES} bytes (40 KB)" >&2
  exit 1
fi

if [ "$GZ_BYTES" -gt "$GZ_LIMIT_BYTES" ]; then
  echo "FAIL: dist/index.js gzipped ${GZ_BYTES} bytes > limit ${GZ_LIMIT_BYTES} bytes (12 KB)" >&2
  exit 1
fi

echo "PASS: dist/index.js minified ${MIN_BYTES} B <= ${MIN_LIMIT_BYTES} B; gzipped ${GZ_BYTES} B <= ${GZ_LIMIT_BYTES} B"
