#!/usr/bin/env bash
# COV_D39_01 — `prepublishOnly` script.
#
# Runs automatically by `npm publish`. Local copy of the
# sdk/typescript-langchain prepublish pattern — the implementation.md §2
# skeleton referenced `../typescript-langchain/scripts/prepublish.sh`, but
# that script `cd`s into ITS OWN package directory and would build the wrong
# package; the pre-declared marker fallback ("copy the script locally") is
# used instead (recorded in the COV_D39_01 marker resolutions).
#
# Tasks:
#   1. Version parity: package.json#version == src/version.ts#VERSION.
#   2. Build with tsup (clean: true — dist regenerated from source).
#   3. Bundle budget gate (implementation.md §3 — breach fails the publish).

set -euo pipefail

cd "$(dirname "$0")/.."

bash scripts/version-check.sh

# Build before the size gate so the budget is measured on a fresh dist tree.
pnpm run build

bash scripts/size-budget.sh

echo "prepublish complete."
