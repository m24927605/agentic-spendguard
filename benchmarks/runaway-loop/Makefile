# Runaway-loop benchmark.
#
# Phase 1A (this Makefile): agentbudget vs agent-guard.
# Phase 1B: + SpendGuard runner against the demo sidecar.
#
# Sequential `docker compose run` per runner — using a single
# `up --abort-on-container-exit` would race-kill slower runners when
# the first one exits.

.PHONY: benchmark benchmark-clean benchmark-logs results

benchmark:
	docker compose -f compose.yml down -v
	docker compose -f compose.yml build
	docker compose -f compose.yml up -d mock-llm spendguard-shim
	docker compose -f compose.yml run --rm --no-deps agentbudget-runner
	docker compose -f compose.yml run --rm --no-deps agentguard-runner
	docker compose -f compose.yml run --rm --no-deps spendguard-runner
	docker compose -f compose.yml run --rm --no-deps analyzer
	docker compose -f compose.yml down

benchmark-clean:
	docker compose -f compose.yml down -v

benchmark-logs:
	docker compose -f compose.yml logs

results:
	@cat RESULTS.md 2>/dev/null || echo "No RESULTS.md yet — run 'make benchmark'"
