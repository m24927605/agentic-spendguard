#!/usr/bin/env bash
# D08 SLICE 6 — npm pack size budget gate.
#
# Asserts the final published tarball stays below the 60 KB budget. The
# adapter is thin glue (core.ts + withSpendGuard.ts + model.ts +
# signature.ts + usage.ts + runContext.ts + defaultEstimator.ts + the
# `./run-context` subpath alias); the budget head-room covers the
# `@openai/agents` peer-import surface and a small amount of additive
# headroom for optional fields landing in v0.2.x (windowInstanceId / unit
# / pricing / claimEstimator — see CHANGELOG.md Known limitations).
#
# Budget: 60 KB (gzipped tarball). `npm pack` emits the gzipped tarball
# size as `.size` in --json output. The D08 design (`implementation.md`
# §2 line "Size budget: ≤ 60 KB minified, ≤ 18 KB gzipped") allows 60 KB
# minified; this gate matches the 60 KB cap on the published tarball.
#
# Mirrors `sdk/typescript-vercel-ai/scripts/size-budget.sh` and
# `sdk/typescript-langchain/scripts/size-budget.sh` field-for-field.

set -euo pipefail

cd "$(dirname "$0")/.."

LIMIT_BYTES=$((60 * 1024))

# `npm pack --dry-run --json` writes the manifest to stdout. We capture it
# to a temp file rather than piping so jq failures surface a usable error
# rather than a broken pipe.
TMP_JSON="$(mktemp -t spendguard-openai-agents-pack.XXXXXX.json)"
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
