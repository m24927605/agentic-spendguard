#!/usr/bin/env bash
# D08 SLICE 6 — version parity gate.
#
# Asserts `package.json#version` equals the `VERSION` constant in
# `src/version.ts`. The CI publish workflow runs this BEFORE `npm publish`
# so a stale wire-reported `sdkVersion` is impossible.
#
# The two values MUST stay locked because:
#   - `package.json#version` is what npm publishes / what consumers see as
#     the installed version.
#   - `src/version.ts#VERSION` is the package's self-reported version
#     constant (consumers `as const`-pin against it; the demo example logs
#     it). Drift means consumers see one version installed and another at
#     runtime.
#
# Mirrors `sdk/typescript-vercel-ai/scripts/version-check.sh` and
# `sdk/typescript-langchain/scripts/version-check.sh` field-for-field.

set -euo pipefail

cd "$(dirname "$0")/.."

PKG_VERSION="$(node -e "console.log(JSON.parse(require('node:fs').readFileSync('package.json','utf8')).version)")"
SRC_VERSION="$(node -e "
  const fs = require('node:fs');
  const src = fs.readFileSync('src/version.ts','utf8');
  // Match \`export const VERSION = \"x.y.z\"\` — single double-quoted literal.
  const m = src.match(/export\s+const\s+VERSION\s*=\s*\"([^\"]+)\"/);
  if (!m) { console.error('version.ts: could not locate VERSION literal'); process.exit(2); }
  console.log(m[1]);
")"

if [ "$PKG_VERSION" != "$SRC_VERSION" ]; then
  echo "FAIL: package.json#version='$PKG_VERSION' != src/version.ts#VERSION='$SRC_VERSION'" >&2
  echo "      Update both to match before publishing." >&2
  exit 1
fi

echo "PASS: package.json#version == src/version.ts#VERSION == '$PKG_VERSION'"
