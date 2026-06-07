#!/usr/bin/env bash
# D29 — `prepublishOnly` script. Mirrors
# `sdk/typescript-langchain/scripts/prepublish.sh`.

set -euo pipefail

cd "$(dirname "$0")/.."

bash scripts/version-check.sh

# Build last so any version-check failure short-circuits before dist work.
pnpm run build
echo "prepublish complete."
