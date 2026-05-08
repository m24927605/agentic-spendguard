# SpendGuard repo top-level Makefile.
#
# Most heavy lifting lives in subproject Makefiles; this file is the
# operator's entry point for the demo bring-up flow.

.PHONY: demo-up demo-down demo-logs demo-clean demo-build help

help:
	@echo "Targets:"
	@echo "  make demo-up      Bring up the full E2E demo + run the demo container"
	@echo "  make demo-down    Stop containers and remove named volumes"
	@echo "  make demo-logs    Tail logs from all demo services"
	@echo "  make demo-clean   demo-down + remove built images"
	@echo "  make demo-build   Build images without running"

demo-up demo-down demo-logs demo-clean demo-build:
	$(MAKE) -C deploy/demo $@
