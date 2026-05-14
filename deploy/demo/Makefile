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
ifeq ($(DEMO_MODE),ttl_sweep)
	@echo "[demo] DEMO_MODE=ttl_sweep → exporting SIDECAR_TTL_SECONDS=5 so sidecar issues short-TTL reservations the sweeper can clean."
	@SIDECAR_TTL_SECONDS=5 $(COMPOSE) up -d --build \
	    postgres pki-init bundles-init canonical-seed-init manifest-init \
	    endpoint-catalog ledger canonical-ingest sidecar webhook-receiver \
	    ttl-sweeper outbox-forwarder
else
	$(COMPOSE) up -d --build \
	    postgres pki-init bundles-init canonical-seed-init manifest-init \
	    endpoint-catalog ledger canonical-ingest sidecar webhook-receiver \
	    ttl-sweeper outbox-forwarder
endif
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
else ifeq ($(DEMO_MODE),deny)
	@echo "[demo] verifying Phase 3 wedge (contract evaluator STOP path)..."
	@$(MAKE) demo-verify-deny
else ifneq (,$(findstring agent_real,$(DEMO_MODE)))
	@echo "[demo] DEMO_MODE=$(DEMO_MODE) — skipping demo-verify-step7."
	@echo "[demo] verify_step7.sql asserts hardcoded Mock LLM token counts (committed=42)"
	@echo "[demo] which never match real provider responses (gpt-4o-mini etc. return"
	@echo "[demo] variable token counts). See SPENDGUARD_VIRAL_PLAYBOOK.todo.md F2."
else
	@echo "[demo] verifying Phase 2B Step 7 (agent-mode commit_estimated only)..."
	@$(MAKE) demo-verify-step7
endif
	@echo "[demo] waiting 5s for outbox forwarder to drain audit chain..."
	@sleep 5
	@$(MAKE) demo-verify-outbox-closure

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

# Outbox forwarder closure (Phase 2B audit chain loop). After each
# demo mode, after a 5s drain wait:
#   - audit_outbox: at least 1 row pending_forward=FALSE (happy path)
#   - canonical_events (canonical DB): at least 1 row matches
demo-verify-outbox-closure:
	@$(COMPOSE) exec -T postgres psql -U spendguard -d spendguard_ledger -v ON_ERROR_STOP=1 < verify_outbox_closure.sql
	@echo "[demo] verifying canonical_events table received forwarded events..."
	@$(COMPOSE) exec -T postgres psql -U spendguard -d spendguard_canonical -v ON_ERROR_STOP=1 -c \
		"DO \$$\$$ DECLARE v_count INT; BEGIN \
		   SELECT COUNT(*) INTO v_count FROM canonical_events \
		    WHERE tenant_id = '00000000-0000-4000-8000-000000000001'; \
		   IF v_count = 0 THEN \
		     RAISE EXCEPTION 'OUTBOX_FORWARDER_GATE: canonical_events empty; CI never received any forwarded event'; \
		   END IF; \
		   RAISE NOTICE 'canonical_events count=%', v_count; \
		 END; \$$\$$;"
	@echo "[demo] outbox closure verification done"

# Phase 3 wedge DENY verification: contract evaluator STOP path emits
# 1 denied_decision ledger_transactions row (no entries) + 1
# audit_outbox row (event_type=spendguard.audit.decision); zero
# reservations; payload includes matched_rules.hard-cap-deny +
# reason_codes.BUDGET_EXHAUSTED for canonical_events forensics.
demo-verify-deny:
	@$(COMPOSE) exec -T postgres psql -U spendguard -d spendguard_ledger -v ON_ERROR_STOP=1 < verify_step_deny.sql
	@echo "[demo] Phase 3 DENY verification done"

# Phase 4 O3 pricing-table verification: pricing-seed-init loaded the
# real pricing_table from seed.yaml; pricing_versions has at least one
# row; coverage spans OpenAI + Anthropic + Azure + Bedrock + Gemini.
demo-verify-pricing:
	@$(COMPOSE) exec -T postgres psql -U spendguard -d spendguard_canonical -v ON_ERROR_STOP=1 < verify_pricing_table.sql
	@echo "[demo] Phase 4 O3 pricing verification done"

# Cost Advisor P0.6 view verification: 3 fixture scenarios assert
# reservations_with_ttl_status_v1 derives state + ttl_seconds correctly.
# Fixtures wrap in SAVEPOINT + ROLLBACK so no persistent state lands.
# Runs against ledger DB after the demo seed (script 30) is in place;
# pairs naturally with DEMO_MODE=ttl_sweep to also exercise the real
# release-with-reason path against the view.
demo-verify-p0_6:
	@$(COMPOSE) exec -T postgres psql -U spendguard -d spendguard_ledger -v ON_ERROR_STOP=1 < verify_p0_6_view.sql
	@echo "[demo] Cost Advisor P0.6 view verification done"

# Cost Advisor P1 rule fixture: synthesizes 10 reservations (7 TTL'd
# at 30s) on 2026-05-13, then runs idle_reservation_rate_v1's SQL
# inline and asserts the expected aggregate (total=10, ttl_expired=7,
# median_ttl=30s, idle_ratio=70%, waste=700_000 μUSD). Rolls back so
# no state lands.
demo-verify-p1:
	@$(COMPOSE) exec -T postgres psql -U spendguard -d spendguard_ledger -v ON_ERROR_STOP=1 < verify_p1_cost_advisor.sql
	@echo "[demo] Cost Advisor P1 rule fixture done"

# Cost Advisor P1.5 fixture: synthesizes canonical_events for run A
# (3× provider_5xx → failed_retry_burn_v1) + run B (7× successful
# calls in 60s → runaway_loop_v1). Asserts both rules fire with
# expected aggregates. Runs against spendguard_canonical (where
# canonical_events lives). Rolls back; no state lands.
demo-verify-p1_5:
	@$(COMPOSE) exec -T postgres psql -U spendguard -d spendguard_canonical -v ON_ERROR_STOP=1 < verify_p1_5_cost_advisor.sql
	@echo "[demo] Cost Advisor P1.5 rules fixture done"

.PHONY: demo-verify-step7 demo-verify-step8 demo-verify-step7_5 demo-verify-step9 demo-verify-ttl-sweep demo-verify-outbox-closure demo-verify-deny demo-verify-pricing demo-verify-p0_6 demo-verify-p1 demo-verify-p1_5

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
