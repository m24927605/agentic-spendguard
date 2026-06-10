# SpendGuard repo top-level Makefile.
#
# Most heavy lifting lives in subproject Makefiles; this file is the
# operator's entry point for the demo bring-up flow.

.PHONY: demo-up demo-down demo-logs demo-clean demo-build help \
        demo-verify-ag-ui-events \
        sdk-ts-proto sdk-ts-proto-check

help:
	@echo "Targets:"
	@echo "  make demo-up           Bring up the full E2E demo + run the demo container"
	@echo "  make demo-down         Stop containers and remove named volumes"
	@echo "  make demo-logs         Tail logs from all demo services"
	@echo "  make demo-clean        demo-down + remove built images"
	@echo "  make demo-build        Build images without running"
	@echo "  make sdk-ts-proto      Regenerate TS SDK proto stubs (sdk/typescript/src/_proto/)"
	@echo "  make sdk-ts-proto-check  CI determinism gate: fail if generated tree drifts"

demo-up demo-down demo-logs demo-clean demo-build demo-verify-ag-ui-events:
	$(MAKE) -C deploy/demo $@

# Mirrors `make -C sdk/python proto`. Delegates to pnpm so the codegen script
# is the single source of truth (see sdk/typescript/scripts/proto.ts).
# Requires `pnpm install` to have run inside sdk/typescript/.
sdk-ts-proto:
	pnpm --filter @spendguard/sdk run proto:gen

sdk-ts-proto-check:
	pnpm --filter @spendguard/sdk run proto:check
