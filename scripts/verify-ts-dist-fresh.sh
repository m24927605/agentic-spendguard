#!/usr/bin/env bash
#
# TS adapter dist determinism gate.
#
# The committed `dist/` bundles for the TS adapters are vendored verbatim by
# the demo runner images (deploy/demo/*/docker-compose.yaml mount the workspace
# read-only and resolve `@spendguard/*` via `file:` against the pre-built dist).
# So a `dist/` that lags its `src/` ships STALE code into every demo and into
# the published npm tarball — e.g. a fail-closed `statusCode === 403` fix that
# lived in source but never reached the built bundle, silently fail-OPENing on a
# DENY (caught in the 2026-06-20 regression sweep for vercel-ai + n8n).
#
# This gate rebuilds every committed-dist package from source and fails if any
# tracked `dist/` file drifts. It mirrors the proto-codegen determinism gate
# (`pnpm --filter @spendguard/sdk run proto:check`). Builds are deterministic
# (verified cold==warm), so a clean tree must produce zero diff.
#
# Run locally with `make sdk-ts-dist-check`. Wired into CI in
# .github/workflows/sdk-ts-ci.yml.
#
# Packages WITHOUT committed dist (core @spendguard/sdk, flowise, openclaw) are
# intentionally excluded: they build dist fresh and gitignore it, so there is
# nothing to drift.
#
# integrations/botpress is ALSO excluded even though it commits dist: its build
# imports the Botpress-generated `.botpress` types module, which only exists
# after `bp` CLI codegen in a Botpress workspace. A clean `pnpm install` checkout
# (CI) cannot resolve it, so botpress dist is built locally and not gate-checked.
# The seven tsup adapters below build purely from src/ + the workspace and ARE
# reproducible cross-platform (pinned esbuild via the frozen lockfile).

set -euo pipefail

cd "$(dirname "$0")/.."

# Committed-dist tsup adapters + their workspace deps (the trailing `...` pulls
# in @spendguard/sdk so adapters build against a fresh core). docs/site-v2,
# flowise, openclaw (build-fresh) and botpress (Botpress codegen) are not built.
echo "[verify-ts-dist] rebuilding committed-dist tsup adapters from source..."
pnpm \
  --filter "@spendguard/langchain..." \
  --filter "@spendguard/vercel-ai..." \
  --filter "@spendguard/openai-agents..." \
  --filter "@spendguard/inngest-agent-kit..." \
  --filter "n8n-nodes-spendguard..." \
  --filter "@spendguard/ag-ui..." \
  --filter "@spendguard/mastra..." \
  build

DIST_PATHS=(
  sdk/typescript-langchain/dist
  sdk/typescript-vercel-ai/dist
  sdk/typescript-openai-agents/dist
  sdk/typescript-inngest-agent-kit/dist
  sdk/typescript-n8n/dist
  sdk/typescript-ag-ui/dist
  sdk/typescript-mastra/dist
)

if git diff --quiet -- "${DIST_PATHS[@]}"; then
  echo "[verify-ts-dist] OK — every committed TS adapter dist matches a fresh build."
  exit 0
fi

echo "" >&2
echo "[verify-ts-dist] ERROR: committed TS adapter dist is STALE vs source." >&2
echo "  A source change was made without rebuilding the committed bundle, so the" >&2
echo "  demos (which vendor dist/) and the npm tarball would ship old code." >&2
echo "  Fix: run 'pnpm -r build' (or 'make sdk-ts-dist') and commit the dist/ changes." >&2
echo "  Drifted files:" >&2
git --no-pager diff --name-only -- "${DIST_PATHS[@]}" | sed 's/^/    /' >&2
exit 1
