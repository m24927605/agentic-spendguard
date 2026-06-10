#!/usr/bin/env bash
# COV_D39_01 — version parity gate.
#
# Asserts `package.json#version` equals the `VERSION` constant in
# `src/version.ts`. Mirrors `sdk/typescript-langchain/scripts/version-check.sh`
# field-for-field (local copy — the cross-package script reference was judged
# brittle; see implementation.md §2 [VERIFY-AT-IMPL] resolution in the slice
# doc).

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
