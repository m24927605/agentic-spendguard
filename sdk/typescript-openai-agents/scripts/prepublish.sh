#!/usr/bin/env bash
# D08 SLICE 6 — `prepublishOnly` script.
#
# Runs automatically by `npm publish` (NOT by `pnpm install` — npm's
# `prepublishOnly` hook is a publish-time-only hook). The publish workflow
# also runs it explicitly so the gates exercised here also gate the
# size-budget step that runs AFTER prepublish.
#
# Tasks:
#   1. Sanity-check `package.json#version` == `src/version.ts#VERSION`.
#      Drift here mints the wrong version constant inside the published
#      bundle's `VERSION` export.
#   2. Build with tsup. tsup is `clean: true` so the dist tree is regenerated
#      from source — no stale artefact from a previous local build can sneak
#      into the published tarball. The build emits the root `dist/index.js`
#      AND the `dist/runContext.js` `./run-context` subpath alias entry,
#      both with matching `.d.ts` declarations.
#
# Mirrors `sdk/typescript-vercel-ai/scripts/prepublish.sh` and
# `sdk/typescript-langchain/scripts/prepublish.sh` field-for-field.
#
# The script is INTENDED to be idempotent — re-running it must produce the
# same dist tree.

set -euo pipefail

cd "$(dirname "$0")/.."

bash scripts/version-check.sh

# Build last so any version-check failure short-circuits before dist work.
pnpm run build
echo "prepublish complete."
