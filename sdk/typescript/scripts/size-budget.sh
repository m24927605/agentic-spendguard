#!/usr/bin/env bash
# SLICE 10 (COV_S05_10) — npm pack size budget gate.
#
# Asserts the final published tarball stays below the budget so the substrate
# remains a tiny dependency for the four downstream adapter packages
# (`@spendguard/langchain`, `@spendguard/vercel-ai`, `@spendguard/openai-agents`,
# `@spendguard/inngest-agentkit`) that depend on `@spendguard/sdk`.
#
# Authoritative bundle-size budget (design.md §10) is on the MINIFIED
# `dist/index.js` (≤120KB unminified + ≤35KB gzipped). The tarball gate here
# is a coarser-grained backstop that catches accidental inclusion of large
# files (raw `_proto/` source, source maps, fixture corpora) in the published
# package. The two checks complement each other.
#
# Tarball composition at v0.1.0:
#   - `dist/**/*.{js,d.ts}`              ~445 KB (proto.d.ts dominates)
#   - `fixtures/cross-language/v1.json`  ~20 KB
#   - README.md + CHANGELOG.md + LICENSE_NOTICES.md ~15 KB combined
#   - package.json                       ~3 KB
#   Source maps are EXCLUDED from the pack (see package.json `files`).
#
# Budget: 250 KB (gzipped tarball). The 200 KB target from
# `docs/slices/COV_S05_10_d05_publish_pipeline.md` is the design intent;
# 250 KB gives a small headroom for the ≥124 KB `proto.d.ts` declaration
# block + future small additions without thrashing the gate on every PR.
# `npm pack` emits the gzipped tarball size as `.size` in --json output.

set -euo pipefail

cd "$(dirname "$0")/.."

LIMIT_BYTES=$((250 * 1024))

# `npm pack --dry-run --json` writes the manifest to stdout. We capture it to
# a temp file rather than piping so jq failures surface a usable error rather
# than a broken pipe.
TMP_JSON="$(mktemp -t spendguard-sdk-pack.XXXXXX.json)"
trap 'rm -f "$TMP_JSON"' EXIT

npm pack --dry-run --json >"$TMP_JSON"

# `.[0].size` is the gzipped tarball size in bytes per `npm pack --json`
# schema. The `unpackedSize` field is the post-extraction size and is NOT the
# gate (it includes the size maps which are themselves never published).
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
