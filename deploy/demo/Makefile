# SpendGuard E2E demo orchestration.
#
# Targets:
#   make demo-up      Build images + bring up the full topology and run the demo.
#   make demo-down    Stop containers and remove named volumes (full reset).
#   make demo-logs    Tail logs from all services.
#   make demo-clean   Remove built images.
#
# Demo modes: set DEMO_MODE before running.
#   DEMO_MODE=agent     (default) full Pydantic-AI Agent.run() with Mock LLM
#   DEMO_MODE=decision  handshake + RequestDecision + ConfirmPublishOutcome only

COMPOSE := docker compose -f compose.yaml
# Default to "decision" mode (Handshake + RequestDecision +
# ConfirmPublishOutcome only, no Pydantic-AI Agent.run()) — this is the
# smaller smoke-test surface and exposes wire-protocol issues without
# being subject to Pydantic-AI version drift. Override with DEMO_MODE=agent.
DEMO_MODE ?= decision

.PHONY: demo-up demo-down demo-logs demo-clean demo-build

demo-build:
	$(COMPOSE) build

demo-up:
	$(COMPOSE) up -d --build \
	    postgres pki-init bundles-init manifest-init endpoint-catalog \
	    ledger canonical-ingest sidecar webhook-receiver ttl-sweeper
	@echo "[demo] core services up; running demo container (DEMO_MODE=$(DEMO_MODE))..."
	$(COMPOSE) run --rm \
	    --env SPENDGUARD_DEMO_MODE=$(DEMO_MODE) \
	    demo
ifeq ($(DEMO_MODE),decision)
	@echo "[demo] verifying Phase 2B Step 8 (decision-mode commit + provider_report)..."
	@$(MAKE) demo-verify-step8
else ifeq ($(DEMO_MODE),invoice)
	@echo "[demo] verifying Phase 2B Step 9 (invoice-mode reserve+commit+provider_report+invoice_reconcile)..."
	@$(MAKE) demo-verify-step9
else ifeq ($(DEMO_MODE),release)
	@echo "[demo] verifying Phase 2B Step 7.5 (release-mode reserve + release)..."
	@$(MAKE) demo-verify-step7_5
else ifeq ($(DEMO_MODE),ttl_sweep)
	@echo "[demo] verifying TTL Sweeper (reserve+TTL=5s -> sweeper auto-release)..."
	@$(MAKE) demo-verify-ttl-sweep
else
	@echo "[demo] verifying Phase 2B Step 7 (agent-mode commit_estimated only)..."
	@$(MAKE) demo-verify-step7
endif

# Phase 2B Step 7 verification: 1 reserve + 1 commit_estimated;
# available=458 / reserved_hold=0 / committed_spend=42. agent-mode runs
# this (sidecar drives only CommitEstimated; ProviderReport is
# webhook-only per Stage 2 §0.2 D9).
demo-verify-step7:
	@$(COMPOSE) exec -T postgres psql -U spendguard -d spendguard_ledger -v ON_ERROR_STOP=1 < verify_step7.sql
	@echo "[demo] Step 7 verification done"

# Phase 2B Step 8 verification: 1 reserve + 1 commit_estimated +
# 1 provider_report; available=462 / reserved_hold=0 / committed_spend=38;
# commits.latest_state='provider_reported'; delta_to_reserved=-62.
# decision-mode runs the demo's webhook simulator after CommitEstimated.
demo-verify-step8:
	@$(COMPOSE) exec -T postgres psql -U spendguard -d spendguard_ledger -v ON_ERROR_STOP=1 < verify_step8.sql
	@echo "[demo] Step 8 verification done"

# Phase 2B Step 7.5 verification: 1 reserve + 1 release;
# available=500 (full refund), reserved_hold=0, committed_spend=0;
# reservations.current_state='released'; commits projection 0 rows.
demo-verify-step7_5:
	@$(COMPOSE) exec -T postgres psql -U spendguard -d spendguard_ledger -v ON_ERROR_STOP=1 < verify_step7_5.sql
	@echo "[demo] Step 7.5 verification done"

# Phase 2B Step 9 verification: full invoice chain (reserve + commit_estimated +
# provider_report + invoice_reconcile); available=460 / reserved_hold=0 /
# committed_spend=40; commits.latest_state='invoice_reconciled';
# delta_to_reserved=-60; dual-row audit (decision+outcome) with :decision /
# :outcome idempotency_key suffixes in global_keys.
demo-verify-step9:
	@$(COMPOSE) exec -T postgres psql -U spendguard -d spendguard_ledger -v ON_ERROR_STOP=1 < verify_step9.sql
	@echo "[demo] Step 9 verification done"

# TTL Sweeper verification: 1 reserve + 1 release(reason=TTL_EXPIRED);
# reservations.current_state='released'; available=500 (full refund);
# audit_outbox contains 1 audit.outcome with cloudevent_payload.data
# decoded JSON.reason='TTL_EXPIRED'.
demo-verify-ttl-sweep:
	@$(COMPOSE) exec -T postgres psql -U spendguard -d spendguard_ledger -v ON_ERROR_STOP=1 < verify_step_ttl_sweep.sql
	@echo "[demo] TTL Sweeper verification done"

.PHONY: demo-verify-step7 demo-verify-step8 demo-verify-step7_5 demo-verify-step9 demo-verify-ttl-sweep

demo-down:
	$(COMPOSE) down -v --remove-orphans

demo-logs:
	$(COMPOSE) logs -f --tail=200

demo-clean: demo-down
	docker image rm \
	    spendguard-demo-ledger \
	    spendguard-demo-canonical-ingest \
	    spendguard-demo-sidecar \
	    spendguard-demo-pki-init \
	    spendguard-demo-bundles-init \
	    spendguard-demo-manifest-init \
	    spendguard-demo-demo \
	    2>/dev/null || true
