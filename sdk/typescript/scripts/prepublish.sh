#!/usr/bin/env bash
# SLICE 10 (COV_S05_10) — `prepublishOnly` script.
#
# Runs automatically by `npm publish` (NOT by `pnpm install` — npm's
# `prepublishOnly` hook is a publish-time-only hook). The publish workflow
# also runs it explicitly so the gates exercised here also gate the
# size-budget step that runs AFTER prepublish.
#
# Tasks:
#   1. Copy the cross-language fixture corpus from `sdk/fixtures/cross-language/v1.json`
#      into the package tree at `fixtures/cross-language/v1.json`. The fixture
#      lives one level up from the npm package root (it's shared with the
#      Python SDK), but npm's `files` field cannot reach `../`. So we copy
#      it in at publish time.
#   2. Sanity-check `package.json#version` == `src/version.ts#VERSION`.
#      Drift here mints the wrong `sdk_version` in the wire handshake.
#   3. Build with tsup. tsup is `clean: true` so the dist tree is regenerated.
#
# The script is INTENDED to be idempotent — re-running it must produce the
# same fixture file + dist tree. The fixture copy is `cp -f` (force-overwrite)
# so a stale local copy doesn't shadow the canonical sdk/fixtures version.

set -euo pipefail

cd "$(dirname "$0")/.."

FIXTURE_SRC="../fixtures/cross-language/v1.json"
FIXTURE_DEST_DIR="fixtures/cross-language"
FIXTURE_DEST="$FIXTURE_DEST_DIR/v1.json"

if [ ! -f "$FIXTURE_SRC" ]; then
  echo "FAIL: source fixture missing: $FIXTURE_SRC" >&2
  echo "      Expected at sdk/fixtures/cross-language/v1.json (shipped in SLICE 9)." >&2
  exit 1
fi

mkdir -p "$FIXTURE_DEST_DIR"
cp -f "$FIXTURE_SRC" "$FIXTURE_DEST"
echo "Copied cross-language fixture: $FIXTURE_SRC -> $FIXTURE_DEST"

bash scripts/version-check.sh

# Build last so any version-check failure short-circuits before dist work.
pnpm run build
echo "prepublish complete."
