#!/usr/bin/env bash
# D29 SLICE 1 — npm pack size budget gate.
#
# Asserts the final published tarball stays below the 50 KB budget. The
# adapter is thin glue (wrapWithSpendGuard.ts + options.ts + ids.ts +
# extract.ts); the budget head-room covers the @inngest/agent-kit peer-import
# surface and a small amount of additive headroom.
#
# Bundle budget (design.md §2 / review-standards §9): 35 KB minified for
# `dist/index.js`. The npm-pack tarball budget here is 50 KB gzipped — same
# 50 KB ceiling as @spendguard/langchain so the cross-adapter SLO is
# uniform.
#
# Mirrors `sdk/typescript-langchain/scripts/size-budget.sh`.

set -euo pipefail

cd "$(dirname "$0")/.."

LIMIT_BYTES=$((50 * 1024))

TMP_JSON="$(mktemp -t spendguard-inngest-agent-kit-pack.XXXXXX.json)"
trap 'rm -f "$TMP_JSON"' EXIT

npm pack --dry-run --json >"$TMP_JSON"

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

echo "PASS: npm pack tarball ${SIZE_BYTES} bytes (${SIZE_KB} KB) <= ${LIMIT_KB} KB (${FILES_COUNT} files)"
