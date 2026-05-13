#!/usr/bin/env bash
#
# Record a short asciinema cast of `make benchmark` for embedding in
# the README. Prebuilds images first so the recording is just the
# runner output (~10–15 seconds) instead of multi-minute Docker
# build time.
#
# Output: benchmarks/runaway-loop/cast/runaway-loop.cast
# View:   asciinema play cast/runaway-loop.cast
# To GIF: agg cast/runaway-loop.cast cast/runaway-loop.gif
#         (install: brew install agg)

set -euo pipefail

cd "$(dirname "$0")/.."

# 1. Prebuild + clean state (NOT recorded).
docker compose -f compose.yml down -v >/dev/null 2>&1 || true
docker compose -f compose.yml build >/dev/null
docker compose -f compose.yml up -d mock-llm spendguard-shim >/dev/null

# Give services a beat to become healthy.
sleep 2

# 2. Record only the runner-and-analyzer phase.
asciinema rec --overwrite --idle-time-limit=0.5 \
  --title "Agentic SpendGuard runaway-loop benchmark" \
  cast/runaway-loop.cast \
  --command 'bash -c "
echo \"\$ make benchmark\"
echo
docker compose -f compose.yml run --rm --no-deps agentbudget-runner 2>&1 | tail -15
echo
docker compose -f compose.yml run --rm --no-deps agentguard-runner 2>&1 | tail -15
echo
docker compose -f compose.yml run --rm --no-deps spendguard-runner 2>&1 | tail -15
echo
docker compose -f compose.yml run --rm --no-deps analyzer 2>&1 | grep -E \"^\\| \\\`\" || true
"'

# 3. Tear down (NOT recorded).
docker compose -f compose.yml down -v >/dev/null 2>&1 || true

echo
echo "Cast saved: $(pwd)/cast/runaway-loop.cast"
echo "Play:        asciinema play cast/runaway-loop.cast"
echo "Convert GIF: agg cast/runaway-loop.cast cast/runaway-loop.gif"
