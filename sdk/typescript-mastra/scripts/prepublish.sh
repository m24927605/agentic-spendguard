#!/usr/bin/env bash
# COV_D38_01 — `prepublishOnly` script.
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
#      into the published tarball.
#   3. Enforce the implementation.md §2 bundle budget (size-budget.sh) on the
#      freshly built dist/index.js — a budget breach fails the publish.
#
# Copied from `sdk/typescript-langchain/scripts/prepublish.sh` (local copy —
# the cross-package relative path is brittle), with the size-budget gate
# wired in per implementation.md §2.
#
# The script is INTENDED to be idempotent — re-running it must produce the
# same dist tree.

set -euo pipefail

cd "$(dirname "$0")/.."

bash scripts/version-check.sh

# Build last so any version-check failure short-circuits before dist work.
pnpm run build

bash scripts/size-budget.sh
echo "prepublish complete."
