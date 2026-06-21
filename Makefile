# SpendGuard repo top-level Makefile.
#
# Most heavy lifting lives in subproject Makefiles; this file is the
# operator's entry point for the demo bring-up flow.

.PHONY: try bench demo-up demo-down demo-logs demo-clean demo-build help \
        demo-verify-ag-ui-events demo-verify-mastra-processor \
        sdk-ts-proto sdk-ts-proto-check sdk-ts-dist sdk-ts-dist-check

help:
	@echo "Targets:"
	@echo "  make try               Try it in ~30s — runaway-loop benchmark (Docker only, no API key)"
	@echo "  make demo-up           Bring up the full E2E demo + run the demo container"
	@echo "  make demo-down         Stop containers and remove named volumes"
	@echo "  make demo-logs         Tail logs from all demo services"
	@echo "  make demo-clean        demo-down + remove built images"
	@echo "  make demo-build        Build images without running"
	@echo "  make demo-verify-ag-ui-events  Verify the AG-UI spend-event demo gate"
	@echo "  make sdk-ts-proto      Regenerate TS SDK proto stubs (sdk/typescript/src/_proto/)"
	@echo "  make sdk-ts-proto-check  CI determinism gate: fail if generated tree drifts"
	@echo "  make sdk-ts-dist       Rebuild every TS adapter dist/ from source (pnpm -r build)"
	@echo "  make sdk-ts-dist-check   CI determinism gate: fail if any committed adapter dist is stale"

demo-up demo-down demo-logs demo-clean demo-build demo-verify-ag-ui-events demo-verify-mastra-processor:
	$(MAKE) -C deploy/demo $@

# One-command "try it" (the README's front door): the runaway-loop benchmark.
# Docker only — no OpenAI key, no real spend (mock LLM). Builds the runners then
# shows SpendGuard's pre-call reservation stop a runaway agent loop head-to-head
# against agentbudget + agent-guard. `bench` is an alias.
try bench:
	$(MAKE) -C benchmarks/runaway-loop benchmark

# Mirrors `make -C sdk/python proto`. Delegates to pnpm so the codegen script
# is the single source of truth (see sdk/typescript/scripts/proto.ts).
# Requires `pnpm install` to have run inside sdk/typescript/.
sdk-ts-proto:
	pnpm --filter @spendguard/sdk run proto:gen

sdk-ts-proto-check:
	pnpm --filter @spendguard/sdk run proto:check

# Rebuild every committed-dist TS adapter from source. The demo runner images
# vendor these dist/ bundles verbatim, so they MUST track src/.
sdk-ts-dist:
	pnpm -r build

# Determinism gate: rebuild the committed-dist adapters and fail if any tracked
# dist/ drifts from a fresh build (i.e. someone edited src/ without rebuilding
# the bundle the demos + npm tarball ship). Mirrors sdk-ts-proto-check.
sdk-ts-dist-check:
	bash scripts/verify-ts-dist-fresh.sh
