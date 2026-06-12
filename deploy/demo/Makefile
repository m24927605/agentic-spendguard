# SpendGuard E2E demo orchestration.
#
# Targets:
#   make demo-up      Build images + bring up the full topology and run the demo.
#   make demo-down    Stop containers and remove named volumes (full reset).
#   make demo-logs    Tail logs from all services.
#   make demo-clean   Remove built images.
#
# Demo modes: set DEMO_MODE before running.
#   DEMO_MODE=agent         (default) full Pydantic-AI Agent.run() with Mock LLM
#   DEMO_MODE=decision      handshake + RequestDecision + ConfirmPublishOutcome only
#   DEMO_MODE=ag_ui_events  COV_D39: spendguard.* AG-UI display events over SSE
#                           (real sidecar run; verify with demo-verify-ag-ui-events)
#   DEMO_MODE=openclaw_base_url
#                           COV_D40A: OpenClaw custom provider baseUrl fixture
#                           through SpendGuard egress proxy + counting stub
#   DEMO_MODE=openclaw_provider_plugin
#                           COV_D40B: OpenClaw in-process provider plugin
#                           wrapper + local counting stub
#   DEMO_MODE=session_reservation
#                           COV_D41S: session-scoped reserve/stream/release
#                           substrate demo + strict audit ingest gate

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
else ifeq ($(DEMO_MODE),proxy)
	@echo "[demo] DEMO_MODE=proxy → postgres + ledger + canonical-ingest + sidecar"
	@echo "[demo]                  + egress-proxy (auto-instrument HTTP proxy)."
	@echo "[demo] Validates the 1-env-var launch claim:"
	@echo "[demo]   OPENAI_BASE_URL=http://localhost:9000/v1"
	@echo "[demo]   + 4 SPENDGUARD_PROXY_DEFAULT_* env vars"
	@echo "[demo]   No code change to the user's openai-python."
	$(COMPOSE) up -d --build \
	    postgres pki-init bundles-init canonical-seed-init manifest-init \
	    endpoint-catalog ledger canonical-ingest sidecar egress-proxy
else ifeq ($(DEMO_MODE),agent_real_openai_agents_proxy)
	@echo "[demo] DEMO_MODE=agent_real_openai_agents_proxy →"
	@echo "[demo]   Same stack as proxy mode, but the demo container runs the"
	@echo "[demo]   ACTUAL openai-agents SDK (no SpendGuard wrapper, no SDK"
	@echo "[demo]   adapter) against OPENAI_BASE_URL=egress-proxy. Verifies the"
	@echo "[demo]   1-env-var launch claim against the SDK HN readers will run."
	$(COMPOSE) up -d --build \
	    postgres pki-init bundles-init canonical-seed-init manifest-init \
	    endpoint-catalog ledger canonical-ingest sidecar egress-proxy
else ifeq ($(DEMO_MODE),anythingllm_real)
	@echo "[demo] DEMO_MODE=anythingllm_real → D33 SLICE 1 (AnythingLLM drop-in recipe)."
	@echo "[demo]   Base stack: proxy mode + AnythingLLM (mintplexlabs/anythingllm:1.8.4)."
	@echo "[demo]   Smoke runner configures the Generic OpenAI provider via"
	@echo "[demo]     /api/v1/system/update-env, sends one Workspace chat through"
	@echo "[demo]     SpendGuard, then asserts reserve + commit_estimated in the"
	@echo "[demo]     ledger audit chain. Real OpenAI gpt-4o-mini round-trip."
	@echo "[demo]   Pattern 2 (base-URL redirect) — no AnythingLLM code change."
	@test -n "$$OPENAI_API_KEY" || (echo "[demo] FATAL: OPENAI_API_KEY required for DEMO_MODE=anythingllm_real" >&2; exit 8)
	$(COMPOSE) --profile anythingllm_real up -d --build \
	    postgres pki-init bundles-init canonical-seed-init manifest-init \
	    endpoint-catalog ledger canonical-ingest sidecar egress-proxy anythingllm
else ifeq ($(DEMO_MODE),lobechat_real)
	@echo "[demo] DEMO_MODE=lobechat_real → D34 SLICE 1 (LobeChat drop-in recipe)."
	@echo "[demo]   Base stack: proxy mode + LobeChat (lobehub/lobe-chat:1.40.0)."
	@echo "[demo]   LobeChat boots with OPENAI_PROXY_URL=http://egress-proxy:9000/v1"
	@echo "[demo]     in the compose env block (no admin API call — the env var"
	@echo "[demo]     does the work at container boot). Smoke runner sends one chat"
	@echo "[demo]     through LobeChat /api/chat/openai, asserts reserve +"
	@echo "[demo]     commit_estimated in the ledger audit chain. Real OpenAI"
	@echo "[demo]     gpt-4o-mini round-trip."
	@echo "[demo]   Pattern 2 (base-URL redirect) — no LobeChat code change."
	@test -n "$$OPENAI_API_KEY" || (echo "[demo] FATAL: OPENAI_API_KEY required for DEMO_MODE=lobechat_real" >&2; exit 8)
	$(COMPOSE) --profile lobechat_real up -d --build \
	    postgres pki-init bundles-init canonical-seed-init manifest-init \
	    endpoint-catalog ledger canonical-ingest sidecar egress-proxy lobechat
else ifeq ($(DEMO_MODE),cost_advisor)
	@echo "[demo] DEMO_MODE=cost_advisor → postgres + ledger + canonical-ingest + sidecar"
	@echo "[demo]                          + bundle-registry + dashboard."
	@echo "[demo] cost_advisor reads ledger directly. bundle-registry LISTENs on NOTIFY"
	@echo "[demo] and applies the approved patch. dashboard exposes the operator-facing"
	@echo "[demo] approval UI; the demo driver POSTs to /api/approvals/:id/resolve."
	@echo "[demo] sidecar runs so CA-P3.7 step 6 can assert the runtime.env watcher"
	@echo "[demo] hot-reloaded the rotated bundle via GET /contract."
	$(COMPOSE) up -d --build \
	    postgres pki-init bundles-init canonical-seed-init manifest-init \
	    endpoint-catalog ledger canonical-ingest sidecar bundle-registry dashboard
else ifeq ($(DEMO_MODE),approval)
	@echo "[demo] DEMO_MODE=approval → postgres + ledger + canonical-ingest + sidecar"
	@echo "[demo]                       + control-plane + outbox-forwarder."
	@echo "[demo] Closes Round-2 #9 part 2: exercises the REQUIRE_APPROVAL →"
	@echo "[demo]   POST /v1/approvals/:id/resolve → ResumeAfterApproval round-trip."
	@echo "[demo] control-plane provides the operator REST surface; outbox-forwarder"
	@echo "[demo] drains the audit chain so verify_step_approval.sql sees the"
	@echo "[demo] approval + resume rows."
	$(COMPOSE) up -d --build \
	    postgres pki-init bundles-init canonical-seed-init manifest-init \
	    endpoint-catalog ledger canonical-ingest sidecar webhook-receiver \
	    control-plane outbox-forwarder
else ifeq ($(DEMO_MODE),multi_provider_usd)
	@echo "[demo] DEMO_MODE=multi_provider_usd → postgres + ledger + canonical-ingest + sidecar + egress-proxy."
	@echo "[demo] SLICE_11 multi-provider routing: the proxy's routing table"
	@echo "[demo]   covers OpenAI / Anthropic / Bedrock / Vertex / Azure OpenAI."
	@echo "[demo] The routing-table tests in services/egress_proxy/tests/multi_provider.rs"
	@echo "[demo] exercise the deterministic path resolution + per-provider"
	@echo "[demo] usage extraction; real provider calls require operator-supplied"
	@echo "[demo] keys (out-of-scope for this demo target)."
	$(COMPOSE) up -d --build \
	    postgres pki-init bundles-init canonical-seed-init manifest-init \
	    endpoint-catalog ledger canonical-ingest sidecar egress-proxy
else ifeq ($(DEMO_MODE),m1_benchmark_runaway_loop)
	@echo "[demo] DEMO_MODE=m1_benchmark_runaway_loop → sidecar wired to run_cost_projector."
	@SPENDGUARD_SIDECAR_RUN_COST_PROJECTOR_URL=http://run-cost-projector:50055 \
	  SPENDGUARD_SIDECAR_ALLOW_UNTRUSTED_BUDGET_METADATA=true \
	  $(COMPOSE) up -d --build \
	    postgres pki-init bundles-init canonical-seed-init manifest-init \
	    endpoint-catalog ledger canonical-ingest run-cost-projector sidecar webhook-receiver \
	    outbox-forwarder
else ifeq ($(DEMO_MODE),plugin_c_synthetic)
	@echo "[demo] DEMO_MODE=plugin_c_synthetic → host Strategy C circuit-breaker regression."
else ifeq ($(DEMO_MODE),envoy_extproc)
	@echo "[demo] DEMO_MODE=envoy_extproc → COV_07 final D01 slice."
	@echo "[demo]   Base stack: postgres + ledger + canonical-ingest + sidecar"
	@echo "[demo]               + outbox-forwarder."
	@echo "[demo]   Overlay (deploy/demo/envoy_extproc/docker-compose.yaml):"
	@echo "[demo]               + counting-stub + envoy-extproc (uds-dev) +"
	@echo "[demo]               envoy-gateway (envoyproxy/envoy:v1.31-latest)."
	@echo "[demo]   Driver issues 3 calls (ALLOW + DENY + STREAM) through the"
	@echo "[demo]   ExtProc filter chain and asserts INV-2 strict-order."
	$(COMPOSE) -f envoy_extproc/docker-compose.yaml up -d --build \
	    postgres pki-init bundles-init canonical-seed-init manifest-init \
	    endpoint-catalog ledger canonical-ingest sidecar webhook-receiver \
	    outbox-forwarder counting-stub envoy-extproc envoy-gateway
else ifeq ($(DEMO_MODE),kong_gateway_real)
	@echo "[demo] DEMO_MODE=kong_gateway_real → D09 SLICE 7 (Kong AI Gateway plugin)."
	@echo "[demo]   Base stack: postgres + ledger + canonical-ingest + sidecar"
	@echo "[demo]               + outbox-forwarder."
	@echo "[demo]   Overlay (deploy/demo/kong_gateway/docker-compose.yaml):"
	@echo "[demo]               + counting-stub (mock OpenAI provider) +"
	@echo "[demo]               kong-companion (SpendGuard HTTP companion) +"
	@echo "[demo]               kong-gateway (kong/kong-gateway:3.7)."
	@echo "[demo]   Driver issues 3 calls (ALLOW + DENY + STREAM) through Kong"
	@echo "[demo]   and asserts INV-2 strict-order. The bypass plugin stands"
	@echo "[demo]   in for the Go .so (see deploy/demo/kong_gateway/README.md)."
	$(COMPOSE) -f kong_gateway/docker-compose.yaml up -d --build \
	    postgres pki-init bundles-init canonical-seed-init manifest-init \
	    endpoint-catalog ledger canonical-ingest sidecar webhook-receiver \
	    outbox-forwarder counting-stub kong-companion kong-gateway
else ifeq ($(DEMO_MODE),langchain_ts)
	@echo "[demo] DEMO_MODE=langchain_ts → COV_D04 SLICE 5 (LangChain.js callback handler)."
	@echo "[demo]   Base stack: postgres + ledger + canonical-ingest + sidecar"
	@echo "[demo]               + outbox-forwarder."
	@echo "[demo]   Overlay (deploy/demo/langchain_ts/docker-compose.yaml):"
	@echo "[demo]               + counting-stub (mock OpenAI provider) +"
	@echo "[demo]               langchain-runner (Node 20 running"
	@echo "[demo]               examples/langchain-ts/index.mjs)."
	@echo "[demo]   Driver issues 3 LangChain.js ChatOpenAI calls"
	@echo "[demo]   (ALLOW + DENY + STREAM) and asserts INV-2 strict-order."
	$(COMPOSE) -f langchain_ts/docker-compose.yaml up -d --build \
	    postgres pki-init bundles-init canonical-seed-init manifest-init \
	    endpoint-catalog ledger canonical-ingest sidecar webhook-receiver \
	    outbox-forwarder counting-stub
else ifeq ($(DEMO_MODE),ag_ui_events)
	@echo "[demo] DEMO_MODE=ag_ui_events → COV_D39 SLICE 3 (AG-UI spend-event family, display-only)."
	@echo "[demo]   Base stack: postgres + ledger + canonical-ingest + sidecar"
	@echo "[demo]               + outbox-forwarder."
	@echo "[demo]   Overlay (deploy/demo/ag_ui_events/docker-compose.yaml):"
	@echo "[demo]               + counting-stub (mock OpenAI provider) +"
	@echo "[demo]               ag-ui-runner (Node 20 running"
	@echo "[demo]               examples/ag-ui-events/index.mjs, then serving"
	@echo "[demo]               the recorded SSE frames on :8077)."
	@echo "[demo]   Runner drives a REAL sidecar run (snapshot + ALLOW"
	@echo "[demo]   reserve/commit + DENY) and renders each decision as a"
	@echo "[demo]   spendguard.* AG-UI CUSTOM event. Display-only: the deny is"
	@echo "[demo]   enforced by the sidecar pre-dispatch; AG-UI just reports it."
	$(COMPOSE) -f ag_ui_events/docker-compose.yaml up -d --build \
	    postgres pki-init bundles-init canonical-seed-init manifest-init \
	    endpoint-catalog ledger canonical-ingest sidecar webhook-receiver \
	    outbox-forwarder counting-stub ag-ui-runner
else ifeq ($(DEMO_MODE),openclaw_base_url)
	@echo "[demo] DEMO_MODE=openclaw_base_url → COV_D40A SLICE 1 (OpenClaw base-URL recipe)."
	@echo "[demo]   Base stack: postgres + ledger + canonical-ingest + sidecar"
	@echo "[demo]               + egress-proxy + outbox-forwarder."
	@echo "[demo]   Overlay (deploy/demo/openclaw_base_url/docker-compose.yaml):"
	@echo "[demo]               + counting-stub (mock OpenAI provider) +"
	@echo "[demo]               openclaw-runner (Node 22 validating the"
	@echo "[demo]               committed OpenClaw custom-provider config)."
	@echo "[demo]   Driver issues 3 OpenAI-compatible calls through the"
	@echo "[demo]   configured OpenClaw provider baseUrl (ALLOW + DENY +"
	@echo "[demo]   STREAM), asserts the DENY leaves /_count unchanged,"
	@echo "[demo]   and uses a local upstream stub — no live OpenAI key."
	$(COMPOSE) -f openclaw_base_url/docker-compose.yaml up -d --build \
	    postgres pki-init bundles-init canonical-seed-init manifest-init \
	    endpoint-catalog ledger canonical-ingest sidecar webhook-receiver \
	    outbox-forwarder counting-stub egress-proxy
else ifeq ($(DEMO_MODE),openclaw_provider_plugin)
	@echo "[demo] DEMO_MODE=openclaw_provider_plugin → COV_D40B SLICE 5 (OpenClaw provider plugin adapter)."
	@echo "[demo]   Base stack: postgres + ledger + canonical-ingest + sidecar"
	@echo "[demo]               + outbox-forwarder."
	@echo "[demo]   Overlay (deploy/demo/openclaw_provider_plugin/docker-compose.yaml):"
	@echo "[demo]               + counting-stub (mock OpenAI provider) +"
	@echo "[demo]               openclaw-provider-plugin-runner (Node 22.19"
	@echo "[demo]               loading @spendguard/openclaw-provider-plugin)."
	@echo "[demo]   Driver issues 4 in-process wrapper calls (ALLOW + DENY +"
	@echo "[demo]   STREAM + PROVIDER_ERROR), asserts DENY leaves /_count"
	@echo "[demo]   unchanged, and uses a local upstream stub — no live OpenAI key."
	$(COMPOSE) -f openclaw_provider_plugin/docker-compose.yaml up -d --build \
	    postgres pki-init bundles-init canonical-seed-init manifest-init \
	    endpoint-catalog ledger canonical-ingest sidecar webhook-receiver \
	    outbox-forwarder counting-stub
else ifeq ($(DEMO_MODE),session_reservation)
	@echo "[demo] DEMO_MODE=session_reservation → COV_D41S substrate demo."
	@echo "[demo]   Minimal stack: postgres + pki + bundles + ledger +"
	@echo "[demo]               canonical-ingest + outbox-forwarder."
	@echo "[demo]   Direct substrate gate; no sidecar or ttl-sweeper build."
	$(COMPOSE) up -d --build \
	    postgres pki-init bundles-init canonical-seed-init manifest-init \
	    ledger canonical-ingest outbox-forwarder
else ifeq ($(DEMO_MODE),vercel_ai_mastra)
	@echo "[demo] DEMO_MODE=vercel_ai_mastra → COV_D06 SLICE 7 (Vercel AI SDK middleware, covers Mastra)."
	@echo "[demo]   Base stack: postgres + ledger + canonical-ingest + sidecar"
	@echo "[demo]               + outbox-forwarder."
	@echo "[demo]   Overlay (deploy/demo/vercel_ai_mastra/docker-compose.yaml):"
	@echo "[demo]               + counting-stub (mock OpenAI provider) +"
	@echo "[demo]               vercel-ai-mastra-runner (Node 20 running"
	@echo "[demo]               examples/vercel-ai-mastra/index.mjs)."
	@echo "[demo]   Driver issues 3 Vercel AI SDK generateText/streamText calls"
	@echo "[demo]   (ALLOW + DENY + STREAM) through wrapLanguageModel + the"
	@echo "[demo]   SpendGuard middleware, asserts INV-2 strict-order, AND"
	@echo "[demo]   asserts the @spendguard/vercel-ai/mastra subpath alias"
	@echo "[demo]   is a strict-equality function-reference of the root export."
	$(COMPOSE) -f vercel_ai_mastra/docker-compose.yaml up -d --build \
	    postgres pki-init bundles-init canonical-seed-init manifest-init \
	    endpoint-catalog ledger canonical-ingest sidecar webhook-receiver \
	    outbox-forwarder counting-stub
else ifeq ($(DEMO_MODE),mastra_processor)
	@echo "[demo] DEMO_MODE=mastra_processor → COV_D38 SLICE 5 (@spendguard/mastra dedicated Processor)."
	@echo "[demo]   Base stack: postgres + ledger + canonical-ingest + sidecar"
	@echo "[demo]               + outbox-forwarder."
	@echo "[demo]   Overlay (deploy/demo/mastra_processor/docker-compose.yaml):"
	@echo "[demo]               + counting-stub (mock OpenAI provider) +"
	@echo "[demo]               mastra-processor-runner (Node 22.13 running"
	@echo "[demo]               examples/mastra-processor/index.mjs — Mastra"
	@echo "[demo]               needs Node >= 22.13; first runner off node:20.10)."
	@echo "[demo]   Driver issues 3 real @mastra/core Agent calls"
	@echo "[demo]   (ALLOW + DENY + STREAM) through SpendGuardProcessor"
	@echo "[demo]   mounted via inputProcessors/outputProcessors, asserts"
	@echo "[demo]   INV-2 strict-order AND /_count UNCHANGED across the DENY"
	@echo "[demo]   step (zero provider HTTP — live fail-closed proof)."
	$(COMPOSE) -f mastra_processor/docker-compose.yaml up -d --build \
	    postgres pki-init bundles-init canonical-seed-init manifest-init \
	    endpoint-catalog ledger canonical-ingest sidecar webhook-receiver \
	    outbox-forwarder counting-stub
else ifeq ($(DEMO_MODE),openai_agents_ts)
	@echo "[demo] DEMO_MODE=openai_agents_ts → COV_D08 SLICE 5 (OpenAI Agents SDK TS adapter)."
	@echo "[demo]   Base stack: postgres + ledger + canonical-ingest + sidecar"
	@echo "[demo]               + outbox-forwarder."
	@echo "[demo]   Overlay (deploy/demo/openai_agents_ts/docker-compose.yaml):"
	@echo "[demo]               + counting-stub (mock OpenAI provider) +"
	@echo "[demo]               openai-agents-runner (Node 20 running"
	@echo "[demo]               examples/openai-agents-ts-composite/demo.mjs --real)."
	@echo "[demo]   Driver issues 3 Agent + Runner.run calls"
	@echo "[demo]   (ALLOW + DENY + STREAM) through withSpendGuard(model),"
	@echo "[demo]   asserts INV-2 strict-order, AND asserts inner.callCount"
	@echo "[demo]   is 0 after the DENY step (review-standards 1.6)."
	$(COMPOSE) -f openai_agents_ts/docker-compose.yaml up -d --build \
	    postgres pki-init bundles-init canonical-seed-init manifest-init \
	    endpoint-catalog ledger canonical-ingest sidecar webhook-receiver \
	    outbox-forwarder counting-stub
else ifeq ($(DEMO_MODE),dify_plugin_real)
	@echo "[demo] DEMO_MODE=dify_plugin_real → COV_D10 SLICE 7 (Dify Model Provider Plugin)."
	@echo "[demo]   Base stack: postgres + ledger + canonical-ingest + sidecar"
	@echo "[demo]               + outbox-forwarder."
	@echo "[demo]   Overlay (deploy/demo/dify_plugin/docker-compose.yaml):"
	@echo "[demo]               + counting-stub (mock OpenAI provider) +"
	@echo "[demo]               dify-plugin-runner (Python 3.12 running the"
	@echo "[demo]               SpendGuardLLM adapter directly via the dify-"
	@echo "[demo]               plugin SDK boundary)."
	@echo "[demo]   Driver issues 3 SpendGuardLLM._invoke calls (ALLOW + DENY +"
	@echo "[demo]   STREAM) and asserts INV-1 (DENY skips upstream) + INV-2"
	@echo "[demo]   (strict-order) + INV-5 (real usage commits)."
	@echo "[demo]   Demo scope: focused on plugin SDK boundary — Dify Workspace"
	@echo "[demo]   chat-messages routing is upstream of the plugin and out of"
	@echo "[demo]   scope for the D10 plugin value proposition (see deviation"
	@echo "[demo]   note in deploy/demo/dify_plugin/docker-compose.yaml)."
	$(COMPOSE) -f dify_plugin/docker-compose.yaml up -d --build \
	    postgres pki-init bundles-init canonical-seed-init manifest-init \
	    endpoint-catalog ledger canonical-ingest sidecar webhook-receiver \
	    outbox-forwarder counting-stub
else ifeq ($(DEMO_MODE),botpress_real)
	@echo "[demo] DEMO_MODE=botpress_real → D32 SLICE 5 (Botpress Integration SDK)."
	@echo "[demo]   Base stack: postgres + ledger + canonical-ingest + sidecar"
	@echo "[demo]               + outbox-forwarder."
	@echo "[demo]   Overlay (deploy/demo/botpress_real/docker-compose.yaml):"
	@echo "[demo]               + counting-stub (mock OpenAI provider) +"
	@echo "[demo]               botpress-integration-runner (Node 20 exercising"
	@echo "[demo]               the @spendguard/botpress-integration hook"
	@echo "[demo]               lifecycle directly via the D09 HTTP companion)."
	@echo "[demo]   Driver issues 3 calls (ALLOW + DENY + STREAM) through the"
	@echo "[demo]   integration's reserve/commit/release path and asserts"
	@echo "[demo]   INV-1 (DENY skips upstream) + INV-5 (real usage commits)."
	@echo "[demo]   Demo scope: focused on Botpress integration SDK boundary —"
	@echo "[demo]   full Botpress v12 self-host (~800 MB image, ~30s boot) is"
	@echo "[demo]   exercised in CI via testcontainers-node (see deviation"
	@echo "[demo]   note in deploy/demo/botpress_real/docker-compose.yaml)."
	$(COMPOSE) -f botpress_real/docker-compose.yaml up -d --build \
	    postgres pki-init bundles-init canonical-seed-init manifest-init \
	    endpoint-catalog ledger canonical-ingest sidecar webhook-receiver \
	    outbox-forwarder counting-stub
else ifeq ($(DEMO_MODE),n8n_real)
	@echo "[demo] DEMO_MODE=n8n_real → D37 SLICE 5 (n8n community node)."
	@echo "[demo]   Base stack: postgres + ledger + canonical-ingest + sidecar"
	@echo "[demo]               + outbox-forwarder."
	@echo "[demo]   Overlay (deploy/demo/n8n_real/docker-compose.yaml):"
	@echo "[demo]               + counting-stub (mock OpenAI provider) +"
	@echo "[demo]               n8n-integration-runner (Node 20 exercising"
	@echo "[demo]               the SpendGuardChatModel sub-node's"
	@echo "[demo]               reserve/commit/release lifecycle via the"
	@echo "[demo]               sidecar HTTP companion)."
	@echo "[demo]   Driver issues 3 calls (ALLOW + DENY + STREAM) and asserts"
	@echo "[demo]   INV-1 (DENY skips upstream) + INV-5 (real usage commits)."
	@echo "[demo]   Demo scope: focused on the integration's reserve/commit"
	@echo "[demo]   wire shape — full n8n v1.50 self-host (~600 MB image,"
	@echo "[demo]   ~30s boot) is exercised in CI via testcontainers-node."
	$(COMPOSE) -f n8n_real/docker-compose.yaml up -d --build \
	    postgres pki-init bundles-init canonical-seed-init manifest-init \
	    endpoint-catalog ledger canonical-ingest sidecar webhook-receiver \
	    outbox-forwarder counting-stub
else ifeq ($(DEMO_MODE),flowise_real)
	@echo "[demo] DEMO_MODE=flowise_real → D35 SLICE 5 (@spendguard/flowise-nodes)."
	@echo "[demo]   Base stack: postgres + ledger + canonical-ingest + sidecar"
	@echo "[demo]               + outbox-forwarder."
	@echo "[demo]   Overlay (deploy/demo/flowise_real/docker-compose.yaml):"
	@echo "[demo]               + counting-stub (mock OpenAI provider) +"
	@echo "[demo]               flowise-integration-runner (Node 20 exercising"
	@echo "[demo]               the @spendguard/flowise-nodes wrapper's"
	@echo "[demo]               reserve/commit/release lifecycle directly via"
	@echo "[demo]               the D09 HTTP companion)."
	@echo "[demo]   Driver issues 3 calls (ALLOW + DENY + STREAM) through the"
	@echo "[demo]   wrapper's reserve/commit/release path and asserts"
	@echo "[demo]   INV-1 (DENY skips upstream) + INV-5 (real usage commits)."
	@echo "[demo]   Demo scope: focused on the wrapper's sidecar boundary —"
	@echo "[demo]   full Flowise 2.x runtime (~1.2 GB image, UI-driven chatflow"
	@echo "[demo]   config) is exercised in the integrations/flowise/tests/e2e"
	@echo "[demo]   testcontainers suite (see deviation note in"
	@echo "[demo]   deploy/demo/flowise_real/docker-compose.yaml)."
	$(COMPOSE) -f flowise_real/docker-compose.yaml up -d --build \
	    postgres pki-init bundles-init canonical-seed-init manifest-init \
	    endpoint-catalog ledger canonical-ingest sidecar webhook-receiver \
	    outbox-forwarder counting-stub
else ifeq ($(DEMO_MODE),langflow_real)
	@echo "[demo] DEMO_MODE=langflow_real → D36 SLICE 4 (spendguard-langflow-component)."
	@echo "[demo]   Base stack: postgres + ledger + canonical-ingest + sidecar"
	@echo "[demo]               + outbox-forwarder."
	@echo "[demo]   Overlay (deploy/demo/langflow_real/docker-compose.yaml):"
	@echo "[demo]               + counting-stub (mock OpenAI provider) +"
	@echo "[demo]               langflow-component-runner (Python 3.12 exercising"
	@echo "[demo]               the SpendGuardChatModelWrapper's build_model_sync()"
	@echo "[demo]               + ainvoke lifecycle directly against the sidecar UDS)."
	@echo "[demo]   Driver issues 3 calls (ALLOW + DENY + STREAM) through the"
	@echo "[demo]   wrapper's reserve/commit/release path and asserts INV-1"
	@echo "[demo]   (DENY skips upstream) + INV-5 (real usage commits)."
	@echo "[demo]   Demo scope: focused on the SpendGuard component's SDK boundary"
	@echo "[demo]   — full Langflow 1.8 UI runtime (~1.2 GB image) is exercised"
	@echo "[demo]   in CI via plugins/langflow tests + the metadata YAML schema"
	@echo "[demo]   check (see deviation note in"
	@echo "[demo]   deploy/demo/langflow_real/docker-compose.yaml)."
	$(COMPOSE) -f langflow_real/docker-compose.yaml up -d --build \
	    postgres pki-init bundles-init canonical-seed-init manifest-init \
	    endpoint-catalog ledger canonical-ingest sidecar webhook-receiver \
	    outbox-forwarder counting-stub
else ifeq ($(DEMO_MODE),coze_studio_real)
	@echo "[demo] DEMO_MODE=coze_studio_real → COV_D31 SLICE 3 (Coze Studio Model Provider)."
	@echo "[demo]   Base stack: postgres + ledger + canonical-ingest + sidecar"
	@echo "[demo]               + outbox-forwarder."
	@echo "[demo]   Overlay (deploy/demo/coze_studio/compose.override.yaml):"
	@echo "[demo]               + counting-stub (mock OpenAI provider) +"
	@echo "[demo]               coze-postgres + coze-redis (Coze metadata —"
	@echo "[demo]               separate DB per INV-8) +"
	@echo "[demo]               coze-seed (idempotent ledger seed) +"
	@echo "[demo]               coze-runner (Python 3.12 demo driver)."
	@echo "[demo]   Driver issues 3 companion-endpoint calls (ALLOW + DENY +"
	@echo "[demo]   STREAM) and asserts INV-1 (DENY skips upstream) + INV-2"
	@echo "[demo]   (strict-order) + INV-5 (end-of-stream commit)."
	@echo "[demo]   Coze Studio container is profiled-off by default (~1.5GB"
	@echo "[demo]   image); the companion contract is the proof surface."
	@echo "[demo]   Set COMPOSE_PROFILES=coze to bring up the full Coze UI."
	@test -n "$$OPENAI_API_KEY" || (echo "[demo] FATAL: OPENAI_API_KEY required for DEMO_MODE=coze_studio_real" >&2; exit 8)
	$(COMPOSE) -f coze_studio/compose.override.yaml up -d --build \
	    postgres pki-init bundles-init canonical-seed-init manifest-init \
	    endpoint-catalog ledger canonical-ingest sidecar webhook-receiver \
	    outbox-forwarder counting-stub \
	    coze-postgres coze-redis coze-seed
else ifeq ($(DEMO_MODE),cursor_mitm_fixture)
	@echo "[demo] DEMO_MODE=cursor_mitm_fixture → D17 SLICE 9 (Cursor IDE MITM codec — EXPERIMENTAL, SOW only)."
	@echo "[demo]   Base stack: postgres + ledger + canonical-ingest + sidecar"
	@echo "[demo]               + outbox-forwarder."
	@echo "[demo]   Overlay (deploy/demo/cursor_mitm_fixture/docker-compose.yaml):"
	@echo "[demo]               + counting-stub (mock OpenAI provider) +"
	@echo "[demo]               cursor-mitm-fixture-runner (Rust 1.83 running"
	@echo "[demo]               services/cursor_codec example cursor_mitm_fixture_demo)."
	@echo "[demo]   Driver replays 4 synthetic .cursor-rpc fixtures (multi-turn,"
	@echo "[demo]   tool calls, error response, long stream) through the codec"
	@echo "[demo]   pipeline + POSTs each translated body to the counting stub."
	@echo "[demo]   Asserts 4 reserves + 3 commits + 1 upstream error + W5 byte-"
	@echo "[demo]   for-byte preservation across the corpus."
	@echo "[demo]   LEGAL POSTURE: This is NOT a real Cursor binary demo. Codec"
	@echo "[demo]   is EXPERIMENTAL, gated to SOW customers. See"
	@echo "[demo]   services/cursor_codec/SOW.md + design.md §1."
	$(COMPOSE) -f cursor_mitm_fixture/docker-compose.yaml up -d --build \
	    postgres pki-init bundles-init canonical-seed-init manifest-init \
	    endpoint-catalog ledger canonical-ingest sidecar webhook-receiver \
	    outbox-forwarder counting-stub
else ifeq ($(DEMO_MODE),windsurf_mitm_fixture)
	@echo "[demo] DEMO_MODE=windsurf_mitm_fixture → D18 SLICE 82 (Windsurf IDE / Codeium MITM codec — EXPERIMENTAL, SOW only)."
	@echo "[demo]   Base stack: postgres + ledger + canonical-ingest + sidecar"
	@echo "[demo]               + outbox-forwarder."
	@echo "[demo]   Overlay (deploy/demo/windsurf_mitm_fixture/docker-compose.yaml):"
	@echo "[demo]               + counting-stub (mock OpenAI provider) +"
	@echo "[demo]               windsurf-mitm-fixture-runner (Rust 1.87 running"
	@echo "[demo]               services/windsurf_codec example windsurf_mitm_fixture_demo)."
	@echo "[demo]   Driver replays 6 synthetic .windsurf-rpc fixtures (simple,"
	@echo "[demo]   tool calls, long streaming, error response, unknown wire"
	@echo "[demo]   version, truncated body) through the codec pipeline + POSTs"
	@echo "[demo]   each translated body to the counting stub."
	@echo "[demo]   Asserts 4 reserves + 3 commits + 1 upstream error + 1"
	@echo "[demo]   unsupported wire version + 1 decoder_skipped + byte-for-"
	@echo "[demo]   byte preservation across the corpus."
	@echo "[demo]   LEGAL POSTURE: This is NOT a real Windsurf binary demo."
	@echo "[demo]   Codec is EXPERIMENTAL, gated to SOW customers. See"
	@echo "[demo]   services/windsurf_codec/SOW.md + D18 design.md §1."
	$(COMPOSE) -f windsurf_mitm_fixture/docker-compose.yaml up -d --build \
	    postgres pki-init bundles-init canonical-seed-init manifest-init \
	    endpoint-catalog ledger canonical-ingest sidecar webhook-receiver \
	    outbox-forwarder counting-stub
else ifeq ($(DEMO_MODE),litellm_sdk_real)
	@echo "[demo] DEMO_MODE=litellm_sdk_real → COV_D12 SLICE 7 (LiteLLM SDK monkey-patch shim)."
	@echo "[demo]   Base stack: postgres + ledger + canonical-ingest + sidecar"
	@echo "[demo]               + outbox-forwarder."
	@echo "[demo]   Overlay (deploy/demo/litellm_sdk_real/docker-compose.yaml):"
	@echo "[demo]               + counting-stub (mock OpenAI provider) +"
	@echo "[demo]               litellm-sdk-shim-runner (Python 3.12 calling"
	@echo "[demo]               real litellm.acompletion after install_shim)."
	@echo "[demo]   Driver issues 3 calls: ALLOW + STREAM + TRANSITIVE/CrewAI"
	@echo "[demo]   (CrewAI step is a clean SKIP if crewai unavailable)."
	@echo "[demo]   D12 closes LiteLLM Issue #8842 — direct SDK callers + 7"
	@echo "[demo]   transitive frameworks (CrewAI / DSPy / SmolAgents / Strands /"
	@echo "[demo]   BeeAI / AutoGen / Atomic Agents) gain a SpendGuard pre-call"
	@echo "[demo]   gate with NO framework-side changes."
	$(COMPOSE) -f litellm_sdk_real/docker-compose.yaml up -d --build \
	    postgres pki-init bundles-init canonical-seed-init manifest-init \
	    endpoint-catalog ledger canonical-ingest sidecar webhook-receiver \
	    outbox-forwarder counting-stub
else ifeq ($(DEMO_MODE),litellm_sdk_deny)
	@echo "[demo] DEMO_MODE=litellm_sdk_deny → COV_D12 SLICE 7 (LiteLLM SDK shim DENY variant)."
	@echo "[demo]   Same topology as litellm_sdk_real but the runner exercises"
	@echo "[demo]   3 fail-closed sub-steps: ALLOW positive control + DENY"
	@echo "[demo]   budget-exhausted + DENY sidecar-unreachable. Stub counter"
	@echo "[demo]   must remain UNCHANGED on each DENY (INV-1)."
	$(COMPOSE) -f litellm_sdk_deny/docker-compose.yaml up -d --build \
	    postgres pki-init bundles-init canonical-seed-init manifest-init \
	    endpoint-catalog ledger canonical-ingest sidecar webhook-receiver \
	    outbox-forwarder counting-stub
else ifeq ($(DEMO_MODE),inngest_agent_kit)
	@echo "[demo] DEMO_MODE=inngest_agent_kit → COV_D29 SLICE 5 (Inngest AgentKit retry-dedup)."
	@echo "[demo]   Base stack: postgres + ledger + canonical-ingest + sidecar"
	@echo "[demo]               + outbox-forwarder."
	@echo "[demo]   Overlay (deploy/demo/inngest_agent_kit/docker-compose.yaml):"
	@echo "[demo]               + counting-stub (mock OpenAI provider) +"
	@echo "[demo]               inngest-agent-kit-runner (Node 20 running"
	@echo "[demo]               examples/inngest-agent-kit/index.mjs --real)."
	@echo "[demo]   Driver issues 3 calls through wrapWithSpendGuard(step.ai):"
	@echo "[demo]   ALLOW + DENY + RETRY_DEDUP (replaces D04/D06/D08's STREAM"
	@echo "[demo]   because step.ai.infer is non-streaming, design.md §3"
	@echo "[demo]   non-goal). RETRY_DEDUP replays the same (runId, step.id,"
	@echo "[demo]   idempotencyKey) 3× and asserts the SpendGuard reserve"
	@echo "[demo]   fires EXACTLY ONCE across attempts — the D29 headline."
	$(COMPOSE) -f inngest_agent_kit/docker-compose.yaml up -d --build \
	    postgres pki-init bundles-init canonical-seed-init manifest-init \
	    endpoint-catalog ledger canonical-ingest sidecar webhook-receiver \
	    outbox-forwarder counting-stub
else ifeq ($(DEMO_MODE),maf_dotnet_real)
	@echo "[demo] DEMO_MODE=maf_dotnet_real → COV_D07 SLICE 8 (Microsoft Agent Framework .NET)."
	@echo "[demo]   Base stack: postgres + ledger + canonical-ingest + sidecar"
	@echo "[demo]               + outbox-forwarder."
	@echo "[demo]   Overlay (deploy/demo/maf_dotnet/docker-compose.yaml):"
	@echo "[demo]               + counting-stub (mock OpenAI provider) +"
	@echo "[demo]               maf-dotnet-runner (.NET 8 console app"
	@echo "[demo]               running examples/maf-dotnet/Program.cs --real)."
	@echo "[demo]   Driver issues 3 calls through IChatClient.UseSpendGuard(sp):"
	@echo "[demo]   ALLOW + DENY + ALLOW2 (replaces D04/D06/D08's STREAM"
	@echo "[demo]   because per-chunk gating is v0.1.x non-goal, design.md §3)."
	$(COMPOSE) -f maf_dotnet/docker-compose.yaml up -d --build \
	    postgres pki-init bundles-init canonical-seed-init manifest-init \
	    endpoint-catalog ledger canonical-ingest sidecar webhook-receiver \
	    outbox-forwarder counting-stub
else ifeq ($(DEMO_MODE),maf_python_real)
	@echo "[demo] DEMO_MODE=maf_python_real → COV_D07 SLICE 8 (Microsoft Agent Framework Python)."
	@echo "[demo]   Base stack: postgres + ledger + canonical-ingest + sidecar"
	@echo "[demo]               + outbox-forwarder."
	@echo "[demo]   Overlay (deploy/demo/maf_python/docker-compose.yaml):"
	@echo "[demo]               + counting-stub (mock OpenAI provider) +"
	@echo "[demo]               maf-python-runner (Python 3.12 running"
	@echo "[demo]               examples/maf-python/run.py --real)."
	@echo "[demo]   Driver issues 3 calls through SpendGuardMiddleware.process(...):"
	@echo "[demo]   ALLOW + DENY + ALLOW2 (replaces D04/D06/D08's STREAM"
	@echo "[demo]   because per-chunk gating is v0.1.x non-goal, design.md §3)."
	$(COMPOSE) -f maf_python/docker-compose.yaml up -d --build \
	    postgres pki-init bundles-init canonical-seed-init manifest-init \
	    endpoint-catalog ledger canonical-ingest sidecar webhook-receiver \
	    outbox-forwarder counting-stub
else ifeq ($(DEMO_MODE),maf_python_with_agt)
	@echo "[demo] DEMO_MODE=maf_python_with_agt → COV_D07 SLICE 8 composite (MAF + AGT)."
	@echo "[demo]   Smoke overlay for design.md ADR-006 coexistence story:"
	@echo "[demo]   single SpendGuardClient shared between SpendGuardMiddleware"
	@echo "[demo]   (MAF chat-client hook) and spendguard.integrations.agt."
	@echo "[demo]   Reuses the same 3-step matrix as maf_python_real."
	$(COMPOSE) -f maf_python_with_agt/docker-compose.yaml up -d --build \
	    postgres pki-init bundles-init canonical-seed-init manifest-init \
	    endpoint-catalog ledger canonical-ingest sidecar webhook-receiver \
	    outbox-forwarder counting-stub
else ifeq ($(DEMO_MODE),agent_real_adk)
	@echo "[demo] DEMO_MODE=agent_real_adk → COV_D19 SLICE 5 (Google ADK Python adapter)."
	@echo "[demo]   Base stack: postgres + ledger + canonical-ingest + sidecar"
	@echo "[demo]               + outbox-forwarder."
	@echo "[demo]   Overlay (deploy/demo/agent_real_adk/docker-compose.yaml):"
	@echo "[demo]               + counting-stub (mock Gemini provider) +"
	@echo "[demo]               adk-runner (Python 3.12 + google-adk +"
	@echo "[demo]               spendguard-sdk[adk])."
	@echo "[demo]   Driver issues 1 LlmAgent call through"
	@echo "[demo]   SpendGuardAdkCallback's before/after slots:"
	@echo "[demo]   ALLOW + DENY (replaces D04/D06/D08's STREAM because"
	@echo "[demo]   ADK per-chunk gating is v0.1.x non-goal, design.md §3.2)."
	$(COMPOSE) -f agent_real_adk/docker-compose.yaml up -d --build \
	    postgres pki-init bundles-init canonical-seed-init manifest-init \
	    endpoint-catalog ledger canonical-ingest sidecar webhook-receiver \
	    outbox-forwarder counting-stub
else ifeq ($(DEMO_MODE),agent_real_adk_stub)
	@echo "[demo] DEMO_MODE=agent_real_adk_stub → COV_D19 no-API-key variant."
	@echo "[demo]   Reuses the agent_real_adk overlay so the runner has"
	@echo "[demo]   google-adk installed (required by the integration's"
	@echo "[demo]   import-time guard), but the driver exercises the"
	@echo "[demo]   callback against duck-typed LlmRequest / LlmResponse"
	@echo "[demo]   (no Gemini API key required). CI gate G08."
	$(COMPOSE) -f agent_real_adk/docker-compose.yaml up -d --build \
	    postgres pki-init bundles-init canonical-seed-init manifest-init \
	    endpoint-catalog ledger canonical-ingest sidecar webhook-receiver \
	    outbox-forwarder counting-stub
else ifeq ($(DEMO_MODE),agent_real_strands)
	@echo "[demo] DEMO_MODE=agent_real_strands → COV_D20 SLICE 5 (AWS Strands Python adapter)."
	@echo "[demo]   Base stack: postgres + ledger + canonical-ingest + sidecar"
	@echo "[demo]               + outbox-forwarder."
	@echo "[demo]   Overlay (deploy/demo/agent_real_strands/docker-compose.yaml):"
	@echo "[demo]               + counting-stub (mock Bedrock + OpenAI provider)"
	@echo "[demo]               + strands-runner (Python 3.12 + strands-agents +"
	@echo "[demo]               spendguard-sdk[strands])."
	@echo "[demo]   Driver issues 1 Agent.invoke_async() through"
	@echo "[demo]   SpendGuardStrandsHookProvider's before/after invocation slots:"
	@echo "[demo]   ALLOW + multi-backend coverage matrix proof."
	$(COMPOSE) -f agent_real_strands/docker-compose.yaml up -d --build \
	    postgres pki-init bundles-init canonical-seed-init manifest-init \
	    endpoint-catalog ledger canonical-ingest sidecar webhook-receiver \
	    outbox-forwarder counting-stub
else ifeq ($(DEMO_MODE),agent_real_strands_deny)
	@echo "[demo] DEMO_MODE=agent_real_strands_deny → COV_D20 SLICE 5 DENY variant."
	@echo "[demo]   Same base + overlay as agent_real_strands; the runner"
	@echo "[demo]   exercises one ALLOW turn followed by one DENY turn and"
	@echo "[demo]   asserts counting-stub recorded zero new hits on DENY"
	@echo "[demo]   (INV-2 zero-provider-HTTP-on-DENY proof)."
	$(COMPOSE) -f agent_real_strands_deny/docker-compose.yaml up -d --build \
	    postgres pki-init bundles-init canonical-seed-init manifest-init \
	    endpoint-catalog ledger canonical-ingest sidecar webhook-receiver \
	    outbox-forwarder counting-stub
else ifeq ($(DEMO_MODE),agent_real_dspy)
	@echo "[demo] DEMO_MODE=agent_real_dspy → COV_D21 SLICE 3 (DSPy Python adapter)."
	@echo "[demo]   Base stack: postgres + ledger + canonical-ingest + sidecar"
	@echo "[demo]               + outbox-forwarder."
	@echo "[demo]   Overlay (deploy/demo/agent_real_dspy/docker-compose.yaml):"
	@echo "[demo]               + counting-stub (mock OpenAI provider on"
	@echo "[demo]               /v1/chat/completions) + dspy-runner (Python 3.12"
	@echo "[demo]               + dspy-ai + spendguard-sdk[dspy])."
	@echo "[demo]   Driver issues 3 substeps through dspy.configure(callbacks=[cb]):"
	@echo "[demo]     1 ALLOW       — dspy.ChainOfThought ALLOW path"
	@echo "[demo]     2 DENY        — DecisionDenied raised BEFORE LM HTTP"
	@echo "[demo]     3 CUSTOM-LM   — direct-path (D12-bypass) coverage proof"
	$(COMPOSE) -f agent_real_dspy/docker-compose.yaml up -d --build \
	    postgres pki-init bundles-init canonical-seed-init manifest-init \
	    endpoint-catalog ledger canonical-ingest sidecar webhook-receiver \
	    outbox-forwarder counting-stub
else ifeq ($(DEMO_MODE),agent_real_agno)
	@echo "[demo] DEMO_MODE=agent_real_agno → COV_D22 SLICE 4 (Agno Python adapter)."
	@echo "[demo]   Base stack: postgres + ledger + canonical-ingest + sidecar"
	@echo "[demo]               + outbox-forwarder."
	@echo "[demo]   Overlay (deploy/demo/agent_real_agno/docker-compose.yaml):"
	@echo "[demo]               + counting-stub (mock OpenAI provider on"
	@echo "[demo]               /v1/chat/completions) + agno-runner (Python 3.12"
	@echo "[demo]               + agno>=2.0,<3 + spendguard-sdk[agno])."
	@echo "[demo]   Driver issues 1 Agent.arun() through SpendGuard"
	@echo "[demo]   pre_hooks=[pre()] / post_hooks=[post()]:"
	@echo "[demo]     ALLOW path proves PRE-before-vendor-SDK + POST-after."
	$(COMPOSE) -f agent_real_agno/docker-compose.yaml up -d --build \
	    postgres pki-init bundles-init canonical-seed-init manifest-init \
	    endpoint-catalog ledger canonical-ingest sidecar webhook-receiver \
	    outbox-forwarder counting-stub
else ifeq ($(DEMO_MODE),agent_real_beeai)
	@echo "[demo] DEMO_MODE=agent_real_beeai → COV_D23 SLICE 4 (BeeAI Framework Python adapter)."
	@echo "[demo]   Base stack: postgres + ledger + canonical-ingest + sidecar"
	@echo "[demo]               + outbox-forwarder."
	@echo "[demo]   Overlay (deploy/demo/agent_real_beeai/docker-compose.yaml):"
	@echo "[demo]               + counting-stub (mock OpenAI provider on"
	@echo "[demo]               /v1/chat/completions) + beeai-runner (Python 3.12"
	@echo "[demo]               + beeai-framework>=0.1.81,<0.2 + spendguard-sdk[beeai])."
	@echo "[demo]   Driver issues 1 BaseAgent.run() through SpendGuard"
	@echo "[demo]   subscribe_spendguard(agent, client, ...):"
	@echo "[demo]     ALLOW path proves *.start reserve BEFORE provider HTTP +"
	@echo "[demo]     *.success commit after."
	$(COMPOSE) -f agent_real_beeai/docker-compose.yaml up -d --build \
	    postgres pki-init bundles-init canonical-seed-init manifest-init \
	    endpoint-catalog ledger canonical-ingest sidecar webhook-receiver \
	    outbox-forwarder counting-stub
else ifeq ($(DEMO_MODE),agent_real_autogen)
	@echo "[demo] DEMO_MODE=agent_real_autogen → COV_D24 SLICE 5 (AutoGen 0.4+ / AG2 adapter)."
	@echo "[demo]   Base stack: postgres + ledger + canonical-ingest + sidecar"
	@echo "[demo]               + outbox-forwarder."
	@echo "[demo]   Overlay (deploy/demo/agent_real_autogen/docker-compose.yaml):"
	@echo "[demo]               + counting-stub (mock OpenAI provider on"
	@echo "[demo]               /v1/chat/completions) + autogen-runner (Python 3.12"
	@echo "[demo]               + autogen-core>=0.4,<1.0 + autogen-agentchat"
	@echo "[demo]               + autogen-ext[openai] + spendguard-sdk[autogen])."
	@echo "[demo]   Driver issues 1 AssistantAgent.on_messages() through"
	@echo "[demo]   SpendGuardChatCompletionClient(inner=OpenAIChatCompletionClient(...)):"
	@echo "[demo]     ALLOW path proves PRE-before-vendor-SDK + POST-after."
	$(COMPOSE) -f agent_real_autogen/docker-compose.yaml up -d --build \
	    postgres pki-init bundles-init canonical-seed-init manifest-init \
	    endpoint-catalog ledger canonical-ingest sidecar webhook-receiver \
	    outbox-forwarder counting-stub
else ifeq ($(DEMO_MODE),agent_real_ag2)
	@echo "[demo] DEMO_MODE=agent_real_ag2 → COV_D24 SLICE 5 (AG2 lineage of D24)."
	@echo "[demo]   Same overlay as agent_real_autogen but the driver"
	@echo "[demo]   requests the ag2 lineage (falls back to autogen if"
	@echo "[demo]   ag2 isn't installed). One wrapper covers both lineages."
	$(COMPOSE) -f agent_real_autogen/docker-compose.yaml up -d --build \
	    postgres pki-init bundles-init canonical-seed-init manifest-init \
	    endpoint-catalog ledger canonical-ingest sidecar webhook-receiver \
	    outbox-forwarder counting-stub
else ifeq ($(DEMO_MODE),agent_real_smolagents)
	@echo "[demo] DEMO_MODE=agent_real_smolagents → COV_D25 SLICE 5 (SmolAgents Python adapter)."
	@echo "[demo]   Base stack: postgres + ledger + canonical-ingest + sidecar"
	@echo "[demo]               + outbox-forwarder."
	@echo "[demo]   Overlay (deploy/demo/agent_real_smolagents/docker-compose.yaml):"
	@echo "[demo]               + counting-stub (mock OpenAI provider on"
	@echo "[demo]               /v1/chat/completions) + smolagents-runner (Python 3.12"
	@echo "[demo]               + smolagents>=1.5,<2 + spendguard-sdk[smolagents])."
	@echo "[demo]   Driver issues 1 SpendGuardSmolModel.generate() through"
	@echo "[demo]   inner=OpenAIServerModel(...):"
	@echo "[demo]     ALLOW path proves PRE-before-vendor-HTTP + POST-after."
	$(COMPOSE) -f agent_real_smolagents/docker-compose.yaml up -d --build \
	    postgres pki-init bundles-init canonical-seed-init manifest-init \
	    endpoint-catalog ledger canonical-ingest sidecar webhook-receiver \
	    outbox-forwarder counting-stub
else ifeq ($(DEMO_MODE),agent_real_letta)
	@echo "[demo] DEMO_MODE=agent_real_letta → COV_D26 SLICE 5 (Letta library-mode adapter)."
	@echo "[demo]   Base stack: postgres + ledger + canonical-ingest + sidecar"
	@echo "[demo]               + outbox-forwarder."
	@echo "[demo]   Overlay (deploy/demo/agent_real_letta/docker-compose.yaml):"
	@echo "[demo]               + counting-stub (mock OpenAI provider on"
	@echo "[demo]               /v1/chat/completions) + letta-runner (Python 3.12"
	@echo "[demo]               + letta>=0.8,<1.0 + spendguard-sdk[letta])."
	@echo "[demo]   Driver issues 1 Agent.step() through"
	@echo "[demo]   SpendGuardLettaClient(inner=OpenAIClient(...)):"
	@echo "[demo]     ALLOW path proves PRE-before-provider-HTTP + POST-after."
	@echo "[demo]   NOTE: ~70%% of Letta deployments are server-mode REST — use"
	@echo "[demo]         D02 + D03 egress proxy for those. D26 covers"
	@echo "[demo]         library-mode (in-process Agent.step()) only."
	$(COMPOSE) -f agent_real_letta/docker-compose.yaml up -d --build \
	    postgres pki-init bundles-init canonical-seed-init manifest-init \
	    endpoint-catalog ledger canonical-ingest sidecar webhook-receiver \
	    outbox-forwarder counting-stub
else ifeq ($(DEMO_MODE),agent_real_llamaindex)
	@echo "[demo] DEMO_MODE=agent_real_llamaindex → COV_D27 SLICE 5 (LlamaIndex callback-manager adapter)."
	@echo "[demo]   Base stack: postgres + ledger + canonical-ingest + sidecar"
	@echo "[demo]               + outbox-forwarder."
	@echo "[demo]   Overlay (deploy/demo/agent_real_llamaindex/docker-compose.yaml):"
	@echo "[demo]               + counting-stub (mock OpenAI on /v1/chat/completions)"
	@echo "[demo]               + llamaindex-runner (Python 3.12 +"
	@echo "[demo]               llama-index-core>=0.12 + llama-index-llms-openai"
	@echo "[demo]               + spendguard-sdk[llamaindex])."
	@echo "[demo]   Driver issues 1 VectorStoreIndex.from_documents -> query through"
	@echo "[demo]   Settings.callback_manager = CallbackManager([SpendGuardLlamaIndexHandler]):"
	@echo "[demo]     ALLOW path proves PRE-before-provider-HTTP + POST-after."
	@echo "[demo]   Two-path coverage matrix: llama-index-llms-litellm is covered"
	@echo "[demo]         transitively by D12; this demo covers the direct provider"
	@echo "[demo]         path (D27)."
	$(COMPOSE) -f agent_real_llamaindex/docker-compose.yaml up -d --build \
	    postgres pki-init bundles-init canonical-seed-init manifest-init \
	    endpoint-catalog ledger canonical-ingest sidecar webhook-receiver \
	    outbox-forwarder counting-stub
else ifeq ($(DEMO_MODE),agent_real_llamaindex_stub)
	@echo "[demo] DEMO_MODE=agent_real_llamaindex_stub → COV_D27 no-API-key variant."
	@echo "[demo]   Reuses the agent_real_llamaindex overlay so the runner has"
	@echo "[demo]   llama-index-core installed (required by the integration's"
	@echo "[demo]   import-time guard), but the driver exercises the handler"
	@echo "[demo]   against MockLLM from llama_index.core.llms (no provider"
	@echo "[demo]   sub-package required, no API key needed). CI gate G09."
	$(COMPOSE) -f agent_real_llamaindex/docker-compose.yaml up -d --build \
	    postgres pki-init bundles-init canonical-seed-init manifest-init \
	    endpoint-catalog ledger canonical-ingest sidecar webhook-receiver \
	    outbox-forwarder counting-stub
else ifeq ($(DEMO_MODE),agent_real_atomic_agents)
	@echo "[demo] DEMO_MODE=agent_real_atomic_agents → COV_D28 SLICE 4 (Atomic Agents + Instructor adapter)."
	@echo "[demo]   Base stack: postgres + ledger + canonical-ingest + sidecar"
	@echo "[demo]               + outbox-forwarder."
	@echo "[demo]   Overlay (deploy/demo/agent_real_atomic_agents/docker-compose.yaml):"
	@echo "[demo]               + counting-stub (mock OpenAI provider serving"
	@echo "[demo]               Instructor TOOLS-mode function-tool calls whose"
	@echo "[demo]               'arguments' JSON deserializes into Answer Pydantic"
	@echo "[demo]               schema) + atomic-agents-runner (Python 3.12 +"
	@echo "[demo]               instructor>=1.5,<2.0 + atomic-agents>=2.0,<3 +"
	@echo "[demo]               openai + pydantic + spendguard-sdk[atomic-agents])."
	@echo "[demo]   Driver wraps Instructor via wrap_instructor_client(...) →"
	@echo "[demo]   SpendGuardInstructorProxy. Per DEVIATION-C the gate intercepts"
	@echo "[demo]   the raw provider create method (per-attempt boundary inside"
	@echo "[demo]   Instructor's retry loop), proving PRE-before-vendor-HTTP +"
	@echo "[demo]   POST-after on every Instructor retry attempt."
	$(COMPOSE) -f agent_real_atomic_agents/docker-compose.yaml up -d --build \
	    postgres pki-init bundles-init canonical-seed-init manifest-init \
	    endpoint-catalog ledger canonical-ingest sidecar webhook-receiver \
	    outbox-forwarder counting-stub
else ifeq ($(DEMO_MODE),subscription_meter)
	@echo "[demo] DEMO_MODE=subscription_meter → D13 (Subscription-Tier Meter)."
	@echo "[demo]   Base stack: postgres + ledger + canonical-ingest + sidecar"
	@echo "[demo]               + outbox-forwarder."
	@echo "[demo]   Overlay (deploy/demo/subscription_meter/docker-compose.yaml):"
	@echo "[demo]               + counting-stub (mock vendor — Anthropic shape) +"
	@echo "[demo]               subscription-meter-runner (Python 3.12 driver that"
	@echo "[demo]               POSTs three requests through the sidecar gRPC"
	@echo "[demo]               adapter with reservation_source=SUBSCRIPTION_METER)."
	@echo "[demo]   Driver issues:"
	@echo "[demo]     1. ClaudeCodePro PASS  — under all thresholds."
	@echo "[demo]     2. ClaudeCodePro SOFT  — over alert_at, CONTINUE + alert."
	@echo "[demo]     3. ClaudeCodePro HARD  — over hard_cap, STOP + synthetic 429."
	@echo "[demo]   Asserts: audit_outbox row with reservation_source='subscription_meter'"
	@echo "[demo]            for each step + ZERO ledger_entries rows for the meter tenant"
	@echo "[demo]            + a hard-cap STOP decision on step 3."
	$(COMPOSE) -f subscription_meter/docker-compose.yaml up -d --build \
	    postgres pki-init bundles-init canonical-seed-init manifest-init \
	    endpoint-catalog ledger canonical-ingest sidecar webhook-receiver \
	    outbox-forwarder counting-stub
else ifeq ($(DEMO_MODE),approval_hot_reload)
	@echo "[demo] DEMO_MODE=approval_hot_reload → same stack as DEMO_MODE=approval."
	@echo "[demo]   Exercises issue #59 BUNDLE_HOT_RELOADED error path end-to-end:"
	@echo "[demo]   REQUIRE_APPROVAL @ B0 → rotate bundle to B1 → resolve →"
	@echo "[demo]   resume() raises ApprovalBundleHotReloadedError."
	@echo "[demo]   The demo container mounts bundles-data :rw to perform the"
	@echo "[demo]   rotation; sidecar's runtime.env watcher picks up the new"
	@echo "[demo]   hash within ~500ms (CA-P3.7 hot-reload poll interval)."
	$(COMPOSE) up -d --build \
	    postgres pki-init bundles-init canonical-seed-init manifest-init \
	    endpoint-catalog ledger canonical-ingest sidecar webhook-receiver \
	    control-plane outbox-forwarder
else
	$(COMPOSE) up -d --build \
	    postgres pki-init bundles-init canonical-seed-init manifest-init \
	    endpoint-catalog ledger canonical-ingest sidecar webhook-receiver \
	    ttl-sweeper outbox-forwarder
endif
ifeq ($(DEMO_MODE),proxy)
	@echo "[demo] running egress-proxy smoke test..."
	@test -n "$$OPENAI_API_KEY" || (echo "[demo] FATAL: OPENAI_API_KEY required for DEMO_MODE=proxy" >&2; exit 8)
	$(COMPOSE) run --rm egress-proxy-smoke
else ifeq ($(DEMO_MODE),plugin_c_synthetic)
	@echo "[demo] verifying Strategy C breaker-open skip path..."
	@cd ../.. && cargo test --manifest-path services/output_predictor/Cargo.toml \
	    breaker_open_skips_predict_without_recording_extra_failure -- --nocapture
	@echo "[demo] verifying HARDEN_08 per-tenant plugin client SVID path..."
	@cd ../.. && cargo test --manifest-path services/output_predictor/Cargo.toml \
	    --test plugin_svid_mtls -- --nocapture
	@cd ../.. && python3 -m pytest contrib/output_predictor_template/conformance_test.py \
	    -q -k 'client_svid'
	@echo "[demo] plugin_c_synthetic PASS — Strategy C breaker open skips plugin Predict"
else ifeq ($(DEMO_MODE),agent_real_openai_agents_proxy)
	@echo "[demo] running openai-agents SDK demo against the proxy..."
	@echo "[demo]   the demo container has OPENAI_API_KEY from the host env"
	@echo "[demo]   and OPENAI_BASE_URL pointing at the proxy."
	$(COMPOSE) build demo
	$(COMPOSE) run --rm --no-deps \
	    --env SPENDGUARD_DEMO_MODE=agent_real_openai_agents_proxy \
	    --env OPENAI_API_KEY=$$OPENAI_API_KEY \
	    --env SPENDGUARD_PROXY_BASE_URL=http://egress-proxy:9000/v1 \
	    demo
	@echo "[demo] PASS — 1-env-var launch claim verified against real openai-agents SDK"
else ifeq ($(DEMO_MODE),anythingllm_real)
	@echo "[demo] running AnythingLLM Generic OpenAI provider smoke (D33)..."
	@echo "[demo]   - configures AnythingLLM via /api/v1/system/update-env"
	@echo "[demo]   - sends one Workspace chat through the SpendGuard egress proxy"
	@echo "[demo]   - asserts reserve + commit_estimated in the ledger audit chain"
	$(COMPOSE) --profile anythingllm_real run --rm anythingllm-smoke
	@echo "[demo] PASS — AnythingLLM drop-in via Generic OpenAI provider verified"
else ifeq ($(DEMO_MODE),lobechat_real)
	@echo "[demo] running LobeChat OPENAI_PROXY_URL smoke (D34)..."
	@echo "[demo]   - LobeChat reads OPENAI_PROXY_URL at container boot (no admin call)"
	@echo "[demo]   - sends one chat through /api/chat/openai to the SpendGuard egress proxy"
	@echo "[demo]   - asserts reserve + commit_estimated in the ledger audit chain"
	$(COMPOSE) --profile lobechat_real run --rm lobechat-smoke
	@echo "[demo] PASS — LobeChat drop-in via OPENAI_PROXY_URL verified"
else ifeq ($(DEMO_MODE),cost_advisor)
	@echo "[demo] running cost_advisor demo container (full closed loop)..."
	$(COMPOSE) run --rm cost-advisor-demo
	@echo "[demo] cost_advisor closed loop PASS — skipping outbox-forwarder check"
	@echo "[demo]   (the cost_advisor write path doesn't produce forwardable audit"
	@echo "[demo]    events; fixture rows in audit_outbox are marked"
	@echo "[demo]    pending_forward=FALSE so the forwarder skips them)"
else ifeq ($(DEMO_MODE),langchain_ts)
	@echo "[demo] running langchain-runner (Node 20 + LangChain.js ChatOpenAI)..."
	$(COMPOSE) -f langchain_ts/docker-compose.yaml run --rm langchain-runner
	@echo "[demo] verifying COV_D04 SLICE 5 (langchain_ts ALLOW + DENY + STREAM)..."
	@$(MAKE) demo-verify-langchain-ts
	@echo "[demo] waiting 5s for outbox forwarder to drain audit chain..."
	@sleep 5
	@$(MAKE) demo-verify-outbox-closure
else ifeq ($(DEMO_MODE),ag_ui_events)
	@echo "[demo] waiting for ag-ui-runner health (snapshot + ALLOW + DENY steps, then SSE replay server)..."
	@for i in $$(seq 1 90); do \
	  st=$$(docker inspect -f '{{.State.Health.Status}}' spendguard-ag-ui-runner 2>/dev/null || echo starting); \
	  if [ "$$st" = "healthy" ]; then break; fi; \
	  if [ "$$st" = "unhealthy" ]; then \
	    echo "[demo] FATAL: ag-ui-runner unhealthy — runner logs follow:" >&2; \
	    docker logs spendguard-ag-ui-runner >&2 || true; exit 9; \
	  fi; \
	  sleep 2; \
	done; \
	st=$$(docker inspect -f '{{.State.Health.Status}}' spendguard-ag-ui-runner 2>/dev/null || echo missing); \
	if [ "$$st" != "healthy" ]; then \
	  echo "[demo] FATAL: ag-ui-runner did not become healthy (status=$$st)" >&2; \
	  docker logs spendguard-ag-ui-runner >&2 || true; exit 9; \
	fi
	@echo "[demo] verifying COV_D39 SLICE 3 (ag_ui_events 4-frame SSE + ledger join + SQL gates)..."
	@$(MAKE) demo-verify-ag-ui-events
else ifeq ($(DEMO_MODE),openclaw_base_url)
	@echo "[demo] running openclaw-runner (Node 22 + committed OpenClaw config fixture)..."
	$(COMPOSE) -f openclaw_base_url/docker-compose.yaml run --rm openclaw-runner
	@echo "[demo] verifying COV_D40A SLICE 1 (openclaw_base_url ALLOW + DENY + STREAM)..."
	@$(MAKE) demo-verify-openclaw-base-url
else ifeq ($(DEMO_MODE),openclaw_provider_plugin)
	@echo "[demo] building @spendguard/openclaw-provider-plugin before container staging..."
	@pnpm --dir ../../integrations/openclaw-provider-plugin build
	@echo "[demo] running openclaw-provider-plugin-runner (Node 22.19 + real sidecar UDS)..."
	$(COMPOSE) -f openclaw_provider_plugin/docker-compose.yaml run --rm openclaw-provider-plugin-runner
	@echo "[demo] verifying COV_D40B SLICE 5 (openclaw_provider_plugin ALLOW + DENY + STREAM + PROVIDER_ERROR)..."
	@$(MAKE) demo-verify-openclaw-provider-plugin
else ifeq ($(DEMO_MODE),session_reservation)
	@echo "[demo] running D41 session reservation substrate driver (signed audit decision/outcome pairs)..."
	@python3 session_reservation/driver.py
	@echo "[demo] waiting 8s for outbox forwarder to drain session audit rows..."
	@sleep 8
	@$(MAKE) demo-verify-session-reservation
else ifeq ($(DEMO_MODE),vercel_ai_mastra)
	@echo "[demo] running vercel-ai-mastra-runner (Node 20 + Vercel AI SDK + Mastra alias)..."
	$(COMPOSE) -f vercel_ai_mastra/docker-compose.yaml run --rm vercel-ai-mastra-runner
	@echo "[demo] verifying COV_D06 SLICE 7 (vercel_ai_mastra ALLOW + DENY + STREAM)..."
	@$(MAKE) demo-verify-vercel-ai-mastra
	@echo "[demo] waiting 5s for outbox forwarder to drain audit chain..."
	@sleep 5
	@$(MAKE) demo-verify-outbox-closure
else ifeq ($(DEMO_MODE),mastra_processor)
	@echo "[demo] running mastra-processor-runner (Node 22.13 + @mastra/core Agent + SpendGuardProcessor)..."
	$(COMPOSE) -f mastra_processor/docker-compose.yaml run --rm mastra-processor-runner
	@echo "[demo] verifying COV_D38 SLICE 5 (mastra_processor ALLOW + DENY + STREAM)..."
	@$(MAKE) demo-verify-mastra-processor
else ifeq ($(DEMO_MODE),openai_agents_ts)
	@echo "[demo] running openai-agents-runner (Node 20 + @openai/agents Agent + Runner.run)..."
	$(COMPOSE) -f openai_agents_ts/docker-compose.yaml run --rm openai-agents-runner
	@echo "[demo] verifying COV_D08 SLICE 5 (openai_agents_ts ALLOW + DENY + STREAM)..."
	@$(MAKE) demo-verify-openai-agents-ts
	@echo "[demo] waiting 5s for outbox forwarder to drain audit chain..."
	@sleep 5
	@$(MAKE) demo-verify-outbox-closure
else ifeq ($(DEMO_MODE),dify_plugin_real)
	@echo "[demo] running dify-plugin-runner (Python 3.12 + SpendGuardLLM._invoke)..."
	$(COMPOSE) -f dify_plugin/docker-compose.yaml run --rm dify-plugin-runner
	@echo "[demo] verifying COV_D10 SLICE 7 (dify_plugin_real ALLOW + DENY + STREAM)..."
	@$(MAKE) demo-verify-dify-plugin-real
	@echo "[demo] waiting 5s for outbox forwarder to drain audit chain..."
	@sleep 5
	@$(MAKE) demo-verify-outbox-closure
else ifeq ($(DEMO_MODE),botpress_real)
	@echo "[demo] running botpress-integration-runner (Node 20 + @spendguard/botpress-integration)..."
	$(COMPOSE) -f botpress_real/docker-compose.yaml run --rm botpress-integration-runner
	@echo "[demo] verifying D32 SLICE 5 (botpress_real ALLOW + DENY + STREAM)..."
	@$(MAKE) demo-verify-botpress-real
	@echo "[demo] waiting 5s for outbox forwarder to drain audit chain..."
	@sleep 5
	@$(MAKE) demo-verify-outbox-closure
else ifeq ($(DEMO_MODE),n8n_real)
	@echo "[demo] running n8n-integration-runner (Node 20 + n8n-nodes-spendguard)..."
	$(COMPOSE) -f n8n_real/docker-compose.yaml run --rm n8n-integration-runner
	@echo "[demo] verifying D37 SLICE 5 (n8n_real ALLOW + DENY + STREAM)..."
	@$(MAKE) demo-verify-n8n-real
	@echo "[demo] waiting 5s for outbox forwarder to drain audit chain..."
	@sleep 5
	@$(MAKE) demo-verify-outbox-closure
else ifeq ($(DEMO_MODE),flowise_real)
	@echo "[demo] running flowise-integration-runner (Node 20 + @spendguard/flowise-nodes)..."
	$(COMPOSE) -f flowise_real/docker-compose.yaml run --rm flowise-integration-runner
	@echo "[demo] verifying D35 SLICE 5 (flowise_real ALLOW + DENY + STREAM)..."
	@$(MAKE) demo-verify-flowise-real
	@echo "[demo] waiting 5s for outbox forwarder to drain audit chain..."
	@sleep 5
	@$(MAKE) demo-verify-outbox-closure
else ifeq ($(DEMO_MODE),langflow_real)
	@echo "[demo] running langflow-component-runner (Python 3.12 + spendguard-langflow-component)..."
	$(COMPOSE) -f langflow_real/docker-compose.yaml run --rm langflow-component-runner
	@echo "[demo] verifying D36 SLICE 4 (langflow_real ALLOW + DENY + STREAM)..."
	@$(MAKE) demo-verify-langflow-real
	@echo "[demo] waiting 5s for outbox forwarder to drain audit chain..."
	@sleep 5
	@$(MAKE) demo-verify-outbox-closure
else ifeq ($(DEMO_MODE),coze_studio_real)
	@echo "[demo] running coze-runner (Python 3.12 + companion endpoint driver)..."
	$(COMPOSE) -f coze_studio/compose.override.yaml run --rm coze-runner
	@echo "[demo] verifying COV_D31 SLICE 3 (coze_studio_real ALLOW + DENY + STREAM)..."
	@$(MAKE) demo-verify-coze-studio-real
	@echo "[demo] waiting 5s for outbox forwarder to drain audit chain..."
	@sleep 5
	@$(MAKE) demo-verify-outbox-closure
else ifeq ($(DEMO_MODE),cursor_mitm_fixture)
	@echo "[demo] running cursor-mitm-fixture-runner (Rust 1.83 — codec replay + counting stub)..."
	$(COMPOSE) -f cursor_mitm_fixture/docker-compose.yaml run --rm cursor-mitm-fixture-runner
	@echo "[demo] verifying D17 SLICE 9 (cursor_mitm_fixture — legal posture + audit chain hygiene)..."
	@$(MAKE) demo-verify-cursor-mitm-fixture
	@echo "[demo] waiting 5s for outbox forwarder to drain audit chain..."
	@sleep 5
	@$(MAKE) demo-verify-outbox-closure
else ifeq ($(DEMO_MODE),windsurf_mitm_fixture)
	@echo "[demo] running windsurf-mitm-fixture-runner (Rust 1.87 — codec replay + counting stub)..."
	$(COMPOSE) -f windsurf_mitm_fixture/docker-compose.yaml run --rm windsurf-mitm-fixture-runner
	@echo "[demo] verifying D18 SLICE 82 (windsurf_mitm_fixture — legal posture + audit chain hygiene)..."
	@$(MAKE) demo-verify-windsurf-mitm-fixture
	@echo "[demo] waiting 5s for outbox forwarder to drain audit chain..."
	@sleep 5
	@$(MAKE) demo-verify-outbox-closure
else ifeq ($(DEMO_MODE),litellm_sdk_real)
	@echo "[demo] running litellm-sdk-shim-runner (Python 3.12 + litellm + install_shim)..."
	$(COMPOSE) -f litellm_sdk_real/docker-compose.yaml run --rm litellm-sdk-shim-runner
	@echo "[demo] verifying COV_D12 SLICE 7 (litellm_sdk_real ALLOW + STREAM + TRANSITIVE)..."
	@$(MAKE) demo-verify-litellm-sdk-real
	@echo "[demo] waiting 5s for outbox forwarder to drain audit chain..."
	@sleep 5
	@$(MAKE) demo-verify-outbox-closure
else ifeq ($(DEMO_MODE),litellm_sdk_deny)
	@echo "[demo] running litellm-sdk-shim-runner DENY variant..."
	$(COMPOSE) -f litellm_sdk_deny/docker-compose.yaml run --rm litellm-sdk-shim-runner
	@echo "[demo] verifying COV_D12 SLICE 7 (litellm_sdk_deny 3 fail-closed sub-steps)..."
	@$(MAKE) demo-verify-litellm-sdk-deny
	@echo "[demo] waiting 5s for outbox forwarder to drain audit chain..."
	@sleep 5
	@$(MAKE) demo-verify-outbox-closure
else ifeq ($(DEMO_MODE),inngest_agent_kit)
	@echo "[demo] running inngest-agent-kit-runner (Node 20 + wrapWithSpendGuard(step.ai))..."
	$(COMPOSE) -f inngest_agent_kit/docker-compose.yaml run --rm inngest-agent-kit-runner
	@echo "[demo] verifying COV_D29 SLICE 5 (inngest_agent_kit ALLOW + DENY + RETRY_DEDUP)..."
	@$(MAKE) demo-verify-inngest-agent-kit
	@echo "[demo] waiting 5s for outbox forwarder to drain audit chain..."
	@sleep 5
	@$(MAKE) demo-verify-outbox-closure
else ifeq ($(DEMO_MODE),maf_dotnet_real)
	@echo "[demo] running maf-dotnet-runner (.NET 8 + IChatClient.UseSpendGuard(sp))..."
	$(COMPOSE) -f maf_dotnet/docker-compose.yaml run --rm maf-dotnet-runner
	@echo "[demo] verifying COV_D07 SLICE 8 (maf_dotnet_real ALLOW + DENY + ALLOW2)..."
	@$(MAKE) demo-verify-maf-dotnet
	@echo "[demo] waiting 5s for outbox forwarder to drain audit chain..."
	@sleep 5
	@$(MAKE) demo-verify-outbox-closure
else ifeq ($(DEMO_MODE),maf_python_real)
	@echo "[demo] running maf-python-runner (Python 3.12 + SpendGuardMiddleware)..."
	$(COMPOSE) -f maf_python/docker-compose.yaml run --rm maf-python-runner
	@echo "[demo] verifying COV_D07 SLICE 8 (maf_python_real ALLOW + DENY + ALLOW2)..."
	@$(MAKE) demo-verify-maf-python
	@echo "[demo] waiting 5s for outbox forwarder to drain audit chain..."
	@sleep 5
	@$(MAKE) demo-verify-outbox-closure
else ifeq ($(DEMO_MODE),maf_python_with_agt)
	@echo "[demo] running maf-python-with-agt-runner (composite of MAF + AGT)..."
	$(COMPOSE) -f maf_python_with_agt/docker-compose.yaml run --rm maf-python-with-agt-runner
	@echo "[demo] verifying COV_D07 SLICE 8 composite (maf_python_with_agt ALLOW + DENY + ALLOW2)..."
	@$(MAKE) demo-verify-maf-python
	@echo "[demo] waiting 5s for outbox forwarder to drain audit chain..."
	@sleep 5
	@$(MAKE) demo-verify-outbox-closure
else ifeq ($(DEMO_MODE),agent_real_adk)
	@echo "[demo] running adk-runner (Python 3.12 + google-adk + SpendGuardAdkCallback)..."
	$(COMPOSE) -f agent_real_adk/docker-compose.yaml run --rm adk-runner
	@echo "[demo] verifying COV_D19 SLICE 5 (agent_real_adk ALLOW + DENY)..."
	@$(MAKE) demo-verify-agent-real-adk
	@echo "[demo] waiting 5s for outbox forwarder to drain audit chain..."
	@sleep 5
	@$(MAKE) demo-verify-outbox-closure
else ifeq ($(DEMO_MODE),agent_real_strands)
	@echo "[demo] running strands-runner (Python 3.12 + strands-agents + SpendGuardStrandsHookProvider)..."
	$(COMPOSE) -f agent_real_strands/docker-compose.yaml run --rm strands-runner
	@echo "[demo] verifying COV_D20 SLICE 5 (agent_real_strands ALLOW)..."
	@$(MAKE) demo-verify-agent-real-strands
	@echo "[demo] waiting 5s for outbox forwarder to drain audit chain..."
	@sleep 5
	@$(MAKE) demo-verify-outbox-closure
else ifeq ($(DEMO_MODE),agent_real_strands_deny)
	@echo "[demo] running strands-runner-deny (ALLOW + DENY proof)..."
	$(COMPOSE) -f agent_real_strands_deny/docker-compose.yaml run --rm strands-runner-deny
	@echo "[demo] verifying COV_D20 SLICE 5 (agent_real_strands_deny ALLOW + DENY)..."
	@$(MAKE) demo-verify-agent-real-strands
	@echo "[demo] waiting 5s for outbox forwarder to drain audit chain..."
	@sleep 5
	@$(MAKE) demo-verify-outbox-closure
else ifeq ($(DEMO_MODE),agent_real_dspy)
	@echo "[demo] running dspy-runner (Python 3.12 + dspy-ai + SpendGuardDSPyCallback)..."
	$(COMPOSE) -f agent_real_dspy/docker-compose.yaml run --rm dspy-runner
	@echo "[demo] verifying COV_D21 SLICE 3 (agent_real_dspy ALLOW + DENY + CUSTOM-LM)..."
	@$(MAKE) demo-verify-agent-real-dspy
	@echo "[demo] waiting 5s for outbox forwarder to drain audit chain..."
	@sleep 5
	@$(MAKE) demo-verify-outbox-closure
else ifeq ($(DEMO_MODE),agent_real_agno)
	@echo "[demo] running agno-runner (Python 3.12 + agno>=2.0,<3 + SpendGuardAgno{Pre,Post}Hook)..."
	$(COMPOSE) -f agent_real_agno/docker-compose.yaml run --rm agno-runner
	@echo "[demo] verifying COV_D22 SLICE 4 (agent_real_agno ALLOW)..."
	@$(MAKE) demo-verify-agent-real-agno
	@echo "[demo] waiting 5s for outbox forwarder to drain audit chain..."
	@sleep 5
	@$(MAKE) demo-verify-outbox-closure
else ifeq ($(DEMO_MODE),agent_real_beeai)
	@echo "[demo] running beeai-runner (Python 3.12 + beeai-framework>=0.1.81,<0.2 + subscribe_spendguard)..."
	$(COMPOSE) -f agent_real_beeai/docker-compose.yaml run --rm beeai-runner
	@echo "[demo] verifying COV_D23 SLICE 4 (agent_real_beeai ALLOW)..."
	@$(MAKE) demo-verify-agent-real-beeai
	@echo "[demo] waiting 5s for outbox forwarder to drain audit chain..."
	@sleep 5
	@$(MAKE) demo-verify-outbox-closure
else ifeq ($(DEMO_MODE),agent_real_autogen)
	@echo "[demo] running autogen-runner (Python 3.12 + autogen-agentchat + SpendGuardChatCompletionClient)..."
	$(COMPOSE) -f agent_real_autogen/docker-compose.yaml run --rm autogen-runner
	@echo "[demo] verifying COV_D24 SLICE 5 (agent_real_autogen ALLOW)..."
	@$(MAKE) demo-verify-agent-real-autogen
	@echo "[demo] waiting 5s for outbox forwarder to drain audit chain..."
	@sleep 5
	@$(MAKE) demo-verify-outbox-closure
else ifeq ($(DEMO_MODE),agent_real_ag2)
	@echo "[demo] running autogen-runner with SPENDGUARD_DEMO_MODE=agent_real_ag2 (AG2 lineage)..."
	@SPENDGUARD_DEMO_MODE=agent_real_ag2 \
	  $(COMPOSE) -f agent_real_autogen/docker-compose.yaml run --rm \
	    -e SPENDGUARD_DEMO_MODE=agent_real_ag2 autogen-runner
	@echo "[demo] verifying COV_D24 SLICE 5 (agent_real_ag2 ALLOW — shared verifier)..."
	@$(MAKE) demo-verify-agent-real-autogen
	@echo "[demo] waiting 5s for outbox forwarder to drain audit chain..."
	@sleep 5
	@$(MAKE) demo-verify-outbox-closure
else ifeq ($(DEMO_MODE),agent_real_smolagents)
	@echo "[demo] running smolagents-runner (Python 3.12 + smolagents>=1.5,<2 + SpendGuardSmolModel)..."
	$(COMPOSE) -f agent_real_smolagents/docker-compose.yaml run --rm smolagents-runner
	@echo "[demo] verifying COV_D25 SLICE 5 (agent_real_smolagents ALLOW)..."
	@$(MAKE) demo-verify-agent-real-smolagents
	@echo "[demo] waiting 5s for outbox forwarder to drain audit chain..."
	@sleep 5
	@$(MAKE) demo-verify-outbox-closure
else ifeq ($(DEMO_MODE),agent_real_letta)
	@echo "[demo] running letta-runner (Python 3.12 + letta>=0.8,<1.0 + SpendGuardLettaClient)..."
	$(COMPOSE) -f agent_real_letta/docker-compose.yaml run --rm letta-runner
	@echo "[demo] verifying COV_D26 SLICE 5 (agent_real_letta ALLOW)..."
	@$(MAKE) demo-verify-agent-real-letta
	@echo "[demo] waiting 5s for outbox forwarder to drain audit chain..."
	@sleep 5
	@$(MAKE) demo-verify-outbox-closure
else ifeq ($(DEMO_MODE),agent_real_llamaindex)
	@echo "[demo] running llamaindex-runner (Python 3.12 + llama-index-core + llama-index-llms-openai + SpendGuardLlamaIndexHandler)..."
	$(COMPOSE) -f agent_real_llamaindex/docker-compose.yaml run --rm llamaindex-runner
	@echo "[demo] verifying COV_D27 SLICE 5 (agent_real_llamaindex ALLOW)..."
	@$(MAKE) demo-verify-agent-real-llamaindex
	@echo "[demo] waiting 5s for outbox forwarder to drain audit chain..."
	@sleep 5
	@$(MAKE) demo-verify-outbox-closure
else ifeq ($(DEMO_MODE),agent_real_llamaindex_stub)
	@echo "[demo] running llamaindex-runner in stub mode (no API key, MockLLM)..."
	@SPENDGUARD_DEMO_MODE=agent_real_llamaindex_stub \
	  $(COMPOSE) -f agent_real_llamaindex/docker-compose.yaml run --rm \
	    -e SPENDGUARD_DEMO_MODE=agent_real_llamaindex_stub llamaindex-runner
	@echo "[demo] verifying COV_D27 SLICE 5 (agent_real_llamaindex_stub PRE + POST)..."
	@$(MAKE) demo-verify-agent-real-llamaindex
else ifeq ($(DEMO_MODE),agent_real_atomic_agents)
	@echo "[demo] running atomic-agents-runner (Python 3.12 + instructor + atomic-agents + SpendGuardInstructorProxy)..."
	$(COMPOSE) -f agent_real_atomic_agents/docker-compose.yaml run --rm atomic-agents-runner
	@echo "[demo] verifying COV_D28 SLICE 4 (agent_real_atomic_agents ALLOW)..."
	@$(MAKE) demo-verify-agent-real-atomic-agents
	@echo "[demo] waiting 5s for outbox forwarder to drain audit chain..."
	@sleep 5
	@$(MAKE) demo-verify-outbox-closure
else ifeq ($(DEMO_MODE),agent_real_adk_stub)
	@echo "[demo] running adk-runner in stub mode (no GOOGLE_API_KEY)..."
	@SPENDGUARD_DEMO_MODE=agent_real_adk_stub \
	  $(COMPOSE) -f agent_real_adk/docker-compose.yaml run --rm \
	    -e SPENDGUARD_DEMO_MODE=agent_real_adk_stub adk-runner
	@echo "[demo] verifying COV_D19 SLICE 5 (agent_real_adk_stub PRE + POST)..."
	@$(MAKE) demo-verify-agent-real-adk
else ifeq ($(DEMO_MODE),subscription_meter)
	@echo "[demo] running subscription-meter-runner (Python 3.12 + sidecar gRPC client)..."
	$(COMPOSE) -f subscription_meter/docker-compose.yaml run --rm subscription-meter-runner
	@echo "[demo] verifying D13 subscription_meter (3 meter steps + no ledger writes)..."
	@$(MAKE) demo-verify-subscription-meter
else
	@echo "[demo] core services up; running demo container (DEMO_MODE=$(DEMO_MODE))..."
	$(COMPOSE) build demo
	$(COMPOSE) run --rm --no-deps \
	    --env SPENDGUARD_DEMO_MODE=$(DEMO_MODE) \
	    demo
ifeq ($(DEMO_MODE),decision)
	@echo "[demo] verifying Phase 2B Step 8 (decision-mode commit + provider_report)..."
	@$(MAKE) demo-verify-step8
else ifeq ($(DEMO_MODE),default)
	@echo "[demo] verifying Phase 2B Step 8 (default decision-mode commit + provider_report)..."
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
else ifeq ($(DEMO_MODE),approval)
	@echo "[demo] verifying Round-2 #9 part 2 (REQUIRE_APPROVAL → resolve → resume)..."
	@$(MAKE) demo-verify-approval
else ifeq ($(DEMO_MODE),approval_hot_reload)
	@echo "[demo] verifying issue #59 hot-reload regression (approval NOT bundled)..."
	@$(MAKE) demo-verify-approval-hot-reload
else ifeq ($(DEMO_MODE),multi_provider_usd)
	@echo "[demo] verifying SLICE_11 multi-provider routing table..."
	@cd ../.. && cargo test --manifest-path services/egress_proxy/Cargo.toml \
	    multi_provider_demo_routing_works_for_all_five -- --nocapture
	@echo "[demo] multi_provider_usd PASS — five provider routes verified"
else ifeq ($(DEMO_MODE),m1_benchmark_runaway_loop)
	@echo "[demo] verifying RUN_BUDGET_PROJECTION_EXCEEDED landed in canonical_events..."
	@sleep 5
	@$(MAKE) demo-verify-m1-runaway-loop
else ifeq ($(DEMO_MODE),litellm_real)
	@echo "[demo] verifying Slice 6 (litellm_real ALLOW + DENY)..."
	@$(MAKE) demo-verify-litellm-real
else ifeq ($(DEMO_MODE),litellm_deny)
	@echo "[demo] verifying Slice 7 (litellm_deny 3 fail-closed sub-steps)..."
	@$(MAKE) demo-verify-litellm-deny
else ifeq ($(DEMO_MODE),litellm_direct)
	@echo "[demo] verifying Slice A3 (litellm_direct ALLOW + DENY)..."
	@$(MAKE) demo-verify-litellm-direct
else ifeq ($(DEMO_MODE),litellm_guardrail)
	@echo "[demo] verifying COV_D11 SLICE 6 (litellm_guardrail ALLOW + DENY + STREAM)..."
	@$(MAKE) demo-verify-litellm-guardrail
else ifeq ($(DEMO_MODE),envoy_extproc)
	@echo "[demo] verifying COV_07 (envoy_extproc ALLOW + DENY + STREAM)..."
	@$(MAKE) demo-verify-envoy-extproc
else ifeq ($(DEMO_MODE),kong_gateway_real)
	@echo "[demo] verifying D09 SLICE 7 (kong_gateway_real ALLOW + DENY + STREAM)..."
	@$(MAKE) demo-verify-kong-gateway-real
else ifeq ($(DEMO_MODE),anythingllm_real)
	@echo "[demo] DEMO_MODE=anythingllm_real — smoke runner already asserted reserve + commit."
	@echo "[demo] Skipping demo-verify-step7 (Mock LLM token-count assertions do not apply to a real OpenAI round-trip)."
else ifeq ($(DEMO_MODE),lobechat_real)
	@echo "[demo] DEMO_MODE=lobechat_real — smoke runner already asserted reserve + commit."
	@echo "[demo] Skipping demo-verify-step7 (Mock LLM token-count assertions do not apply to a real OpenAI round-trip)."
else ifneq (,$(findstring agent_real,$(DEMO_MODE)))
	@echo "[demo] DEMO_MODE=$(DEMO_MODE) — skipping demo-verify-step7."
	@echo "[demo] verify_step7.sql asserts hardcoded Mock LLM token counts (committed=42)"
	@echo "[demo] which never match real provider responses (gpt-4o-mini etc. return"
	@echo "[demo] variable token counts). See SPENDGUARD_VIRAL_PLAYBOOK.todo.md F2."
else
	@echo "[demo] verifying Phase 2B Step 7 (agent-mode commit_estimated only)..."
	@$(MAKE) demo-verify-step7
endif
ifneq ($(DEMO_MODE),multi_provider_usd)
	@echo "[demo] waiting 5s for outbox forwarder to drain audit chain..."
	@sleep 5
	@$(MAKE) demo-verify-outbox-closure
else
	@echo "[demo] multi_provider_usd offline routing gate emits no audit rows without provider keys; skipping outbox closure."
endif
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

demo-verify-session-reservation:
	@$(COMPOSE) exec -T postgres psql -U spendguard -d spendguard_ledger -v ON_ERROR_STOP=1 < verify_step_session_reservation.sql
	@echo "[demo] verifying canonical_events carried D41 session audit pairs..."
	@SESSION_ID="$$( $(COMPOSE) exec -T postgres psql -U spendguard -d spendguard_ledger -tA -v ON_ERROR_STOP=1 -c "SELECT session_reservation_id FROM session_reservations WHERE tenant_id = '00000000-0000-4000-8000-000000000001' AND session_id = 'd41-session-reservation-demo'" )"; \
	DENIED_SESSION_ID="$$( $(COMPOSE) exec -T postgres psql -U spendguard -d spendguard_ledger -tA -v ON_ERROR_STOP=1 -c "SELECT session_reservation_id FROM session_reservations WHERE tenant_id = '00000000-0000-4000-8000-000000000001' AND session_id = 'd41-session-reservation-deny-demo'" )"; \
	EXPIRED_SESSION_ID="$$( $(COMPOSE) exec -T postgres psql -U spendguard -d spendguard_ledger -tA -v ON_ERROR_STOP=1 -c "SELECT session_reservation_id FROM session_reservations WHERE tenant_id = '00000000-0000-4000-8000-000000000001' AND session_id = 'd41-session-reservation-expire-demo'" )"; \
	$(COMPOSE) exec -T postgres psql -U spendguard -d spendguard_canonical -v ON_ERROR_STOP=1 -v session_id="$$SESSION_ID" -v denied_session_id="$$DENIED_SESSION_ID" -v expired_session_id="$$EXPIRED_SESSION_ID" < verify_step_session_reservation_canonical.sql
	@echo "[demo] session_reservation verification done"

# Phase 3 wedge DENY verification: contract evaluator STOP path emits
# 1 denied_decision ledger_transactions row (no entries) + 1
# audit_outbox row (event_type=spendguard.audit.decision); zero
# reservations; payload includes matched_rules.hard-cap-deny +
# reason_codes.BUDGET_EXHAUSTED for canonical_events forensics.
demo-verify-deny:
	@$(COMPOSE) exec -T postgres psql -U spendguard -d spendguard_ledger -v ON_ERROR_STOP=1 < verify_step_deny.sql
	@echo "[demo] Phase 3 DENY verification done"

demo-verify-m1-runaway-loop:
	@$(COMPOSE) exec -T postgres psql -U spendguard -d spendguard_canonical -v ON_ERROR_STOP=1 -c \
		"DO \$$\$$ DECLARE v_count INT; BEGIN \
		   SELECT COUNT(*) INTO v_count FROM canonical_events \
		    WHERE tenant_id = '00000000-0000-4000-8000-000000000001' \
		      AND event_type = 'spendguard.audit.decision' \
		      AND cost_advisor_safe_decode_payload(payload_json)::text LIKE '%RUN_BUDGET_PROJECTION_EXCEEDED%'; \
		   IF v_count = 0 THEN \
		     RAISE EXCEPTION 'HARDEN_02_GATE: RUN_BUDGET_PROJECTION_EXCEEDED not found in canonical_events'; \
		   END IF; \
		   RAISE NOTICE 'HARDEN_02 RUNAWAY LOOP OK: matching canonical_events=%', v_count; \
		 END; \$$\$$;"
	@echo "[demo] M1 runaway-loop verification done"

# Round-2 #9 part 2 closed-loop verification: full REQUIRE_APPROVAL
# round-trip — contract evaluator emits REQUIRE_APPROVAL, demo runner
# POSTs to control-plane /v1/approvals/:id/resolve, sidecar
# ResumeAfterApproval calls Ledger.ReserveSet + MarkApprovalBundled.
# Asserts approval_requests.state='approved' AND
# bundled_ledger_transaction_id IS NOT NULL.
demo-verify-approval:
	@$(COMPOSE) exec -T postgres psql -U spendguard -d spendguard_ledger -v ON_ERROR_STOP=1 < verify_step_approval.sql
	@echo "[demo] Round-2 #9 part 2 approval verification done"

# Issue #59 slice 4 (#68): hot-reload regression. Asserts the approval
# was resolved=approved but NEVER bundled (bundled_ledger_transaction_id
# IS NULL) because resume failed with BUNDLE_HOT_RELOADED.
demo-verify-approval-hot-reload:
	@$(COMPOSE) exec -T postgres psql -U spendguard -d spendguard_ledger -v ON_ERROR_STOP=1 < verify_step_approval_hot_reload.sql
	@echo "[demo] issue #59 hot-reload regression verification done"

# Slice 6 (litellm_real) verification: ALLOW step produces a
# reserve + commit_estimated; DENY step produces a denied_decision.
# The .sql file holds ONLY ledger-DB assertions; canonical_events
# assertions are cross-DB and live in a second `psql -d
# spendguard_canonical` block here (R2 P0-2 fix). The 5s drain
# before the canonical-DB query gives the outbox forwarder a tick
# to catch up (R2 P1-1 fix).
demo-verify-litellm-deny:
	@$(COMPOSE) exec -T postgres psql -U spendguard -d spendguard_ledger -v ON_ERROR_STOP=1 < verify_step_litellm_deny.sql
	@echo "[demo] Slice 7 litellm_deny verification done"

demo-verify-litellm-direct:
	@$(COMPOSE) exec -T postgres psql -U spendguard -d spendguard_ledger -v ON_ERROR_STOP=1 < verify_step_litellm_direct.sql
	@echo "[demo] Slice A3 litellm_direct verification done"

# COV_D11 SLICE 6 (litellm_guardrail) verification. Mirrors the
# litellm_real layout: ledger-DB assertions in verify_step_litellm_guardrail.sql,
# then a 5s outbox drain wait, then a cross-DB canonical_events check
# under spendguard_canonical that asserts the spendguard sub-object's
# integration='litellm' enrichment landed for both ALLOW + DENY rows.
demo-verify-litellm-guardrail:
	@$(COMPOSE) exec -T postgres psql -U spendguard -d spendguard_ledger -v ON_ERROR_STOP=1 < verify_step_litellm_guardrail.sql
	@echo "[demo] waiting 5s for outbox forwarder to land canonical_events rows..."
	@sleep 5
	@$(COMPOSE) exec -T postgres psql -U spendguard -d spendguard_canonical -v ON_ERROR_STOP=1 -c \
		"DO \$$\$$ DECLARE v_decision INT; v_outcome INT; v_with_spendguard INT; v_integration TEXT; BEGIN \
		   SELECT COUNT(*) INTO v_decision FROM canonical_events \
		    WHERE tenant_id = '00000000-0000-4000-8000-000000000001' \
		      AND event_type = 'spendguard.audit.decision'; \
		   SELECT COUNT(*) INTO v_outcome FROM canonical_events \
		    WHERE tenant_id = '00000000-0000-4000-8000-000000000001' \
		      AND event_type = 'spendguard.audit.outcome'; \
		   IF v_decision < 2 THEN \
		     RAISE EXCEPTION 'D11_GUARDRAIL_GATE: canonical_events.decision >= 2 expected (ALLOW + DENY), got %', v_decision; \
		   END IF; \
		   IF v_outcome < 1 THEN \
		     RAISE EXCEPTION 'D11_GUARDRAIL_GATE: canonical_events.outcome >= 1 expected (ALLOW commit), got %', v_outcome; \
		   END IF; \
		   SELECT COUNT(*) INTO v_with_spendguard FROM canonical_events \
		    WHERE tenant_id = '00000000-0000-4000-8000-000000000001' \
		      AND event_type = 'spendguard.audit.decision' \
		      AND cost_advisor_safe_decode_payload(payload_json)->'spendguard' IS NOT NULL; \
		   IF v_with_spendguard < 2 THEN \
		     RAISE EXCEPTION 'D11_GUARDRAIL_GATE: canonical_events with spendguard sub-object >= 2 expected (ALLOW + DENY, both carry integration=litellm), got %', v_with_spendguard; \
		   END IF; \
		   SELECT (cost_advisor_safe_decode_payload(payload_json)->'spendguard'->>'integration') INTO v_integration FROM canonical_events \
		    WHERE tenant_id = '00000000-0000-4000-8000-000000000001' \
		      AND event_type = 'spendguard.audit.decision' \
		      AND cost_advisor_safe_decode_payload(payload_json)->'spendguard' IS NOT NULL \
		    LIMIT 1; \
		   IF v_integration <> 'litellm' THEN \
		     RAISE EXCEPTION 'D11_GUARDRAIL_GATE: spendguard.integration expected litellm, got %', v_integration; \
		   END IF; \
		   RAISE NOTICE 'D11_GUARDRAIL CANONICAL OK: decision=% outcome=% with_spendguard=% integration=%', v_decision, v_outcome, v_with_spendguard, v_integration; \
		 END; \$$\$$;"
	@echo "[demo] COV_D11 SLICE 6 litellm_guardrail verification done"

# COV_07 (D01 envoy_extproc demo) verification. Mirrors the
# litellm_guardrail layout: ledger-DB assertions in
# verify_step_envoy_extproc.sql + INV-2 strict-order DO block + a
# cross-DB canonical_events check under spendguard_canonical that
# asserts at least 2 decision rows + 1 outcome row landed after the
# outbox forwarder drained.
demo-verify-envoy-extproc:
	@$(COMPOSE) exec -T postgres psql -U spendguard -d spendguard_ledger -v ON_ERROR_STOP=1 < verify_step_envoy_extproc.sql
	@echo "[demo] waiting 5s for outbox forwarder to land canonical_events rows..."
	@sleep 5
	@$(COMPOSE) exec -T postgres psql -U spendguard -d spendguard_canonical -v ON_ERROR_STOP=1 -c \
		"DO \$$\$$ DECLARE v_decision INT; v_outcome INT; BEGIN \
		   SELECT COUNT(*) INTO v_decision FROM canonical_events \
		    WHERE tenant_id = '00000000-0000-4000-8000-000000000001' \
		      AND event_type = 'spendguard.audit.decision'; \
		   SELECT COUNT(*) INTO v_outcome FROM canonical_events \
		    WHERE tenant_id = '00000000-0000-4000-8000-000000000001' \
		      AND event_type = 'spendguard.audit.outcome'; \
		   IF v_decision < 2 THEN \
		     RAISE EXCEPTION 'COV_07_GATE: canonical_events.decision >= 2 expected (ALLOW + DENY), got %', v_decision; \
		   END IF; \
		   IF v_outcome < 1 THEN \
		     RAISE EXCEPTION 'COV_07_GATE: canonical_events.outcome >= 1 expected (ALLOW commit), got %', v_outcome; \
		   END IF; \
		   RAISE NOTICE 'COV_07 CANONICAL OK: decision=% outcome=%', v_decision, v_outcome; \
		 END; \$$\$$;"
	@echo "[demo] COV_07 envoy_extproc verification done"

# D09 SLICE 7 (kong_gateway_real demo) verification. Mirrors the
# envoy_extproc layout: ledger-DB assertions in
# verify_step_kong_gateway_real.sql + INV-2 strict-order DO block.
demo-verify-kong-gateway-real:
	@$(COMPOSE) exec -T postgres psql -U spendguard -d spendguard_ledger -v ON_ERROR_STOP=1 < verify_step_kong_gateway_real.sql
	@echo "[demo] D09 SLICE 7 kong_gateway_real verification done"

# D10 SLICE 7 (dify_plugin_real demo) verification. Mirrors the
# kong_gateway_real layout: ledger-DB assertions in
# verify_step_dify_plugin_real.sql + INV-2 strict-order DO block + a
# cross-DB canonical_events check under spendguard_canonical that
# asserts at least 2 decision rows + 1 outcome row landed after the
# outbox forwarder drained.
demo-verify-dify-plugin-real:
	@$(COMPOSE) exec -T postgres psql -U spendguard -d spendguard_ledger -v ON_ERROR_STOP=1 < verify_step_dify_plugin_real.sql
	@echo "[demo] waiting 5s for outbox forwarder to land canonical_events rows..."
	@sleep 5
	@$(COMPOSE) exec -T postgres psql -U spendguard -d spendguard_canonical -v ON_ERROR_STOP=1 -c \
		"DO \$$\$$ DECLARE v_decision INT; v_outcome INT; BEGIN \
		   SELECT COUNT(*) INTO v_decision FROM canonical_events \
		    WHERE tenant_id = '00000000-0000-4000-8000-000000000001' \
		      AND event_type = 'spendguard.audit.decision'; \
		   SELECT COUNT(*) INTO v_outcome FROM canonical_events \
		    WHERE tenant_id = '00000000-0000-4000-8000-000000000001' \
		      AND event_type = 'spendguard.audit.outcome'; \
		   IF v_decision < 2 THEN \
		     RAISE EXCEPTION 'COV_D10_GATE: canonical_events.decision >= 2 expected (ALLOW + DENY), got %', v_decision; \
		   END IF; \
		   IF v_outcome < 1 THEN \
		     RAISE EXCEPTION 'COV_D10_GATE: canonical_events.outcome >= 1 expected (ALLOW commit), got %', v_outcome; \
		   END IF; \
		   RAISE NOTICE 'COV_D10 CANONICAL OK: decision=% outcome=%', v_decision, v_outcome; \
		 END; \$$\$$;"
	@echo "[demo] COV_D10 SLICE 7 dify_plugin_real verification done"

# D32 SLICE 5 (botpress_real demo) verification. Mirrors the
# dify_plugin_real layout: ledger-DB assertions in
# verify_step_botpress_real.sql (6 gates per review-standards.md §5.7)
# covering integration-tagged decision count, DENY presence, commit row,
# INV-1 (no upstream hits on DENY), streaming row, and canonical_events
# forwarding.
demo-verify-botpress-real:
	@$(COMPOSE) exec -T postgres psql -U spendguard -d spendguard_ledger -v ON_ERROR_STOP=1 < verify_step_botpress_real.sql
	@echo "[demo] waiting 5s for outbox forwarder to land canonical_events rows..."
	@sleep 5
	@$(COMPOSE) exec -T postgres psql -U spendguard -d spendguard_canonical -v ON_ERROR_STOP=1 -c \
		"DO \$$\$$ DECLARE v_decision INT; v_outcome INT; BEGIN \
		   SELECT COUNT(*) INTO v_decision FROM canonical_events \
		    WHERE tenant_id = '00000000-0000-4000-8000-000000000001' \
		      AND event_type = 'spendguard.audit.decision'; \
		   SELECT COUNT(*) INTO v_outcome FROM canonical_events \
		    WHERE tenant_id = '00000000-0000-4000-8000-000000000001' \
		      AND event_type = 'spendguard.audit.outcome'; \
		   IF v_decision < 2 THEN \
		     RAISE EXCEPTION 'D32_BOTPRESS_GATE: canonical_events.decision >= 2 expected (ALLOW + DENY), got %', v_decision; \
		   END IF; \
		   IF v_outcome < 1 THEN \
		     RAISE EXCEPTION 'D32_BOTPRESS_GATE: canonical_events.outcome >= 1 expected (ALLOW commit), got %', v_outcome; \
		   END IF; \
		   RAISE NOTICE 'D32_BOTPRESS CANONICAL OK: decision=% outcome=%', v_decision, v_outcome; \
		 END; \$$\$$;"
	@echo "[demo] D32 SLICE 5 botpress_real verification done"

# D37 SLICE 5 (n8n_real demo) verification. Mirrors the botpress_real
# layout: ledger-DB assertions in verify_step_n8n_real.sql (6 gates per
# review-standards.md §8) covering integration-tagged decision count,
# DENY presence, commit row, INV-1, streaming row, and canonical_events
# forwarding.
demo-verify-n8n-real:
	@$(COMPOSE) exec -T postgres psql -U spendguard -d spendguard_ledger -v ON_ERROR_STOP=1 < verify_step_n8n_real.sql
	@echo "[demo] waiting 5s for outbox forwarder to land canonical_events rows..."
	@sleep 5
	@$(COMPOSE) exec -T postgres psql -U spendguard -d spendguard_canonical -v ON_ERROR_STOP=1 -c \
		"DO \$$\$$ DECLARE v_decision INT; v_outcome INT; BEGIN \
		   SELECT COUNT(*) INTO v_decision FROM canonical_events \
		    WHERE tenant_id = '00000000-0000-4000-8000-000000000001' \
		      AND event_type = 'spendguard.audit.decision'; \
		   SELECT COUNT(*) INTO v_outcome FROM canonical_events \
		    WHERE tenant_id = '00000000-0000-4000-8000-000000000001' \
		      AND event_type = 'spendguard.audit.outcome'; \
		   IF v_decision < 2 THEN \
		     RAISE EXCEPTION 'D37_N8N_GATE: canonical_events.decision >= 2 expected (ALLOW + DENY), got %', v_decision; \
		   END IF; \
		   IF v_outcome < 1 THEN \
		     RAISE EXCEPTION 'D37_N8N_GATE: canonical_events.outcome >= 1 expected (ALLOW commit), got %', v_outcome; \
		   END IF; \
		   RAISE NOTICE 'D37_N8N CANONICAL OK: decision=% outcome=%', v_decision, v_outcome; \
		 END; \$$\$$;"
	@echo "[demo] D37 SLICE 5 n8n_real verification done"

# D35 SLICE 5 (flowise_real demo) verification. Mirrors the
# botpress_real layout: ledger-DB assertions in
# verify_step_flowise_real.sql (6 gates per review-standards.md §5.7)
# covering integration-tagged decision count, DENY presence, commit row,
# INV-1 (no upstream hits on DENY), streaming row, and canonical_events
# forwarding.
demo-verify-flowise-real:
	@$(COMPOSE) exec -T postgres psql -U spendguard -d spendguard_ledger -v ON_ERROR_STOP=1 < verify_step_flowise_real.sql
	@echo "[demo] waiting 5s for outbox forwarder to land canonical_events rows..."
	@sleep 5
	@$(COMPOSE) exec -T postgres psql -U spendguard -d spendguard_canonical -v ON_ERROR_STOP=1 -c \
		"DO \$$\$$ DECLARE v_decision INT; v_outcome INT; BEGIN \
		   SELECT COUNT(*) INTO v_decision FROM canonical_events \
		    WHERE tenant_id = '00000000-0000-4000-8000-000000000001' \
		      AND event_type = 'spendguard.audit.decision'; \
		   SELECT COUNT(*) INTO v_outcome FROM canonical_events \
		    WHERE tenant_id = '00000000-0000-4000-8000-000000000001' \
		      AND event_type = 'spendguard.audit.outcome'; \
		   IF v_decision < 2 THEN \
		     RAISE EXCEPTION 'D35_FLOWISE_GATE: canonical_events.decision >= 2 expected (ALLOW + DENY), got %', v_decision; \
		   END IF; \
		   IF v_outcome < 1 THEN \
		     RAISE EXCEPTION 'D35_FLOWISE_GATE: canonical_events.outcome >= 1 expected (ALLOW commit), got %', v_outcome; \
		   END IF; \
		   RAISE NOTICE 'D35_FLOWISE CANONICAL OK: decision=% outcome=%', v_decision, v_outcome; \
		 END; \$$\$$;"
	@echo "[demo] D35 SLICE 5 flowise_real verification done"

# D36 SLICE 4 (langflow_real demo) verification. Mirrors the
# botpress_real / flowise_real layout: ledger-DB assertions in
# verify_step_langflow_real.sql (6 gates per review-standards.md §5.7)
# covering integration=langchain + source=langflow decision count, DENY
# presence, commit row, INV-1 (no upstream hits on DENY), streaming row,
# and canonical_events forwarding.
demo-verify-langflow-real:
	@$(COMPOSE) exec -T postgres psql -U spendguard -d spendguard_ledger -v ON_ERROR_STOP=1 < verify_step_langflow_real.sql
	@echo "[demo] waiting 5s for outbox forwarder to land canonical_events rows..."
	@sleep 5
	@$(COMPOSE) exec -T postgres psql -U spendguard -d spendguard_canonical -v ON_ERROR_STOP=1 -c \
		"DO \$$\$$ DECLARE v_decision INT; v_outcome INT; BEGIN \
		   SELECT COUNT(*) INTO v_decision FROM canonical_events \
		    WHERE tenant_id = '00000000-0000-4000-8000-000000000001' \
		      AND event_type = 'spendguard.audit.decision'; \
		   SELECT COUNT(*) INTO v_outcome FROM canonical_events \
		    WHERE tenant_id = '00000000-0000-4000-8000-000000000001' \
		      AND event_type = 'spendguard.audit.outcome'; \
		   IF v_decision < 2 THEN \
		     RAISE EXCEPTION 'D36_LANGFLOW_GATE: canonical_events.decision >= 2 expected (ALLOW + DENY), got %', v_decision; \
		   END IF; \
		   IF v_outcome < 1 THEN \
		     RAISE EXCEPTION 'D36_LANGFLOW_GATE: canonical_events.outcome >= 1 expected (ALLOW commit), got %', v_outcome; \
		   END IF; \
		   RAISE NOTICE 'D36_LANGFLOW CANONICAL OK: decision=% outcome=%', v_decision, v_outcome; \
		 END; \$$\$$;"
	@echo "[demo] D36 SLICE 4 langflow_real verification done"

# D31 SLICE 3 (coze_studio_real demo) verification. Mirrors the
# dify_plugin_real layout: ledger-DB assertions in
# verify_step_coze_studio_real.sql (A1..A7) covering the 7
# implementation.md §2 Slice 3 SQL gates — count + DENY + commit +
# streaming + INV-1 stub-hits + canonical-events soft-check + audit
# chain monotonicity (with a fallback when spendguard_verify_chain is
# unavailable in this DB).
demo-verify-coze-studio-real:
	@$(COMPOSE) exec -T postgres psql -U spendguard -d spendguard_ledger -v ON_ERROR_STOP=1 < verify_step_coze_studio_real.sql
	@echo "[demo] COV_D31 SLICE 3 coze_studio_real verification done"

# D17 SLICE 9 (cursor_mitm_fixture demo) verification. The codec runs
# against the in-memory mock sidecar lane (per SOW.md §6 legal posture
# — no real reserve/commit gRPC dance), so the ledger DB has NO rows
# attributable to this demo. The verify SQL therefore asserts the
# **negative** gates:
#
#   - Zero `api.cursor.sh` references in any audit_outbox payload
#     (review-standards §6 C1 no-live-traffic invariant).
#   - Zero Cursor session token shapes (`sk-cur-` / `cursor_pat_`)
#     leaked into the audit chain.
#
# The runner's own stdout (`CURSOR_MITM_FIXTURE_DEMO_OK` line) is the
# positive gate; it asserts 4 reserves + 3 commits + W5 byte-for-byte
# preservation and exits non-zero on any drift.
demo-verify-cursor-mitm-fixture:
	@$(COMPOSE) exec -T postgres psql -U spendguard -d spendguard_ledger -v ON_ERROR_STOP=1 < verify_step_cursor_mitm_fixture.sql
	@echo "[demo] D17 SLICE 9 cursor_mitm_fixture verification done"

# D13 (subscription_meter demo) — assert the meter rows landed without
# producing ledger entries.  Mirrors the cursor_mitm verify pattern:
# audit_outbox rows tagged reservation_source='subscription_meter' for
# the demo tenant, ZERO ledger_entries rows under the same tenant.
demo-verify-subscription-meter:
	@$(COMPOSE) exec -T postgres psql -U spendguard -d spendguard_ledger -v ON_ERROR_STOP=1 < verify_step_subscription_meter.sql
	@echo "[demo] D13 subscription_meter verification done"

# D14 COV_72 — import_devin_fixture demo. Self-contained: spins up a
# throwaway postgres:16-alpine container, applies mig 0059 (widen
# import_source CHECK to include 'devin_team_api'), runs the
# spendguard-importer-devin binary against the committed sanitized
# fixture in `services/importer_devin/tests/fixtures/devin_usage.json`,
# INSERTs the emitted CloudEvents as audit_outbox rows, and runs the
# verifier SQL to assert:
#
#   - >= 1 audit_outbox row with import_source='devin_team_api' +
#     reservation_source='subscription_meter' for tenant=demo.
#   - >= 1 team-plan row with amount_micro_usd > 0 (acceptance A10.3,
#     ACU → $ conversion correctness).
#   - >= 1 enterprise-plan row with amount_micro_usd IS NULL AND
#     reason_code='devin_enterprise_negotiated_rate' (A10.1).
#   - ZERO ledger_entries rows under demo tenant (D13 §4.3 fork).
#
# Idempotent: ON CONFLICT (event_id) DO NOTHING. Re-running lands zero
# new rows (deterministic UUIDv5).
demo-verify-import-devin-fixture:
	@bash ./import_devin_fixture_demo.sh
	@echo "[demo] D14 COV_72 import_devin_fixture verification done"

# D15 COV_74 — import_manus_fixture demo. Self-contained: spins up a
# throwaway postgres:16-alpine container, applies mig 0060 (widen
# import_source CHECK to include 'manus_team_api'), runs the
# spendguard-importer-manus binary against the committed sanitized
# fixture in `services/importer_manus/tests/fixtures/manus_usage.json`,
# INSERTs the emitted CloudEvents as audit_outbox rows, and runs the
# verifier SQL to assert:
#
#   - EXACTLY 7 audit_outbox rows with import_source='manus_team_api' +
#     reservation_source='subscription_meter' (8 sessions in fixture, 1
#     in_progress filtered per E3).
#   - team_plan total = 20_731_260 micro-USD (acceptance A5.4 headline).
#   - >= 1 enterprise row with amount=0 (P2 operator-override default).
#   - >= 1 enterprise_byok row with amount=0 (P3 LOAD-BEARING; BYOK
#     customers MUST NOT be double-billed).
#   - ZERO ledger_entries rows under ws_FAKE_% (D13 §4.3 fork).
#   - Every row carries model='manus.session/credit', tokens=0, and
#     dedupe_key LIKE 'manus:%'.
#
# Idempotent: ON CONFLICT (event_id) DO NOTHING. Re-running lands zero
# new rows (deterministic UUIDv5).
demo-verify-import-manus-fixture:
	@bash ./import_manus_fixture_demo.sh
	@echo "[demo] D15 COV_74 import_manus_fixture verification done"

# Alias preferred by the D15 spec deliverable prompt. Calls the same
# runner as demo-verify-import-manus-fixture.
demo-import-manus-fixture: demo-verify-import-manus-fixture

# D16 COV_88 — import_genspark_fixture demo. Self-contained: spins up a
# throwaway postgres:16-alpine container, applies mig 0061 (widen
# import_source CHECK to include 'genspark_team_api'), runs the
# spendguard-importer-genspark binary against the committed sanitized
# fixture in `services/importer_genspark/tests/fixtures/genspark_usage.json`,
# INSERTs the emitted CloudEvents as audit_outbox rows, and runs the
# verifier SQL to assert:
#
#   - >= 3 audit_outbox rows with import_source='genspark_team_api' +
#     reservation_source='subscription_meter' for tenant=demo.
#   - >= 1 plus-plan row with amount_micro_usd > 0 (credit → $ conversion
#     correctness).
#   - >= 1 premium-plan row with amount_micro_usd > 0 (premium-rate
#     correctness).
#   - >= 1 unknown-plan row with amount_micro_usd = 0 AND
#     reason_code='genspark_plan_unknown' (T7 / F4 fallback).
#   - ZERO ledger_entries rows under demo tenant (D13 §4.3 fork).
#   - 3200-credit plus row → 6_396_800 micro-USD (hand-computed).
#
# Idempotent: ON CONFLICT (event_id) DO NOTHING. Re-running lands zero
# new rows (deterministic UUIDv5).
demo-verify-import-genspark-fixture:
	@bash ./import_genspark_fixture_demo.sh
	@echo "[demo] D16 COV_88 import_genspark_fixture verification done"

# D18 SLICE 82 (windsurf_mitm_fixture demo) verification. Mirrors the
# cursor_mitm_fixture verification: the codec runs against the
# in-memory mock sidecar lane, so the verify SQL asserts the
# **negative** gates:
#
#   - Zero `server.codeium.com` / `windsurf-server.codeium.com`
#     references in any audit_outbox payload.
#   - Zero Codeium credential shapes (`sk-codeium-` / `wsf_` /
#     `codeium_pat_` / `cdm_`) leaked into the audit chain.
#
# The runner's own stdout (`WINDSURF_MITM_FIXTURE_DEMO_OK` line) is
# the positive gate; it asserts 4 reserves + 3 commits + 1 upstream
# error + 1 unsupported wire version + 1 decoder_skipped + byte-for-
# byte preservation and exits non-zero on any drift.
demo-verify-windsurf-mitm-fixture:
	@$(COMPOSE) exec -T postgres psql -U spendguard -d spendguard_ledger -v ON_ERROR_STOP=1 < verify_step_windsurf_mitm_fixture.sql
	@echo "[demo] D18 SLICE 82 windsurf_mitm_fixture verification done"

# COV_D12 SLICE 7 (litellm_sdk_real demo) verification. Mirrors the
# dify_plugin_real layout: ledger-DB assertions in
# verify_step_litellm_sdk_real.sql + a cross-DB canonical_events check
# under spendguard_canonical asserting >=2 decision rows + >=2 outcome
# rows landed after the outbox forwarder drained.
demo-verify-litellm-sdk-real:
	@$(COMPOSE) exec -T postgres psql -U spendguard -d spendguard_ledger -v ON_ERROR_STOP=1 < verify_step_litellm_sdk_real.sql
	@echo "[demo] waiting 5s for outbox forwarder to land canonical_events rows..."
	@sleep 5
	@$(COMPOSE) exec -T postgres psql -U spendguard -d spendguard_canonical -v ON_ERROR_STOP=1 -c \
		"DO \$$\$$ DECLARE v_decision INT; v_outcome INT; BEGIN \
		   SELECT COUNT(*) INTO v_decision FROM canonical_events \
		    WHERE tenant_id = '00000000-0000-4000-8000-000000000001' \
		      AND event_type = 'spendguard.audit.decision'; \
		   SELECT COUNT(*) INTO v_outcome FROM canonical_events \
		    WHERE tenant_id = '00000000-0000-4000-8000-000000000001' \
		      AND event_type = 'spendguard.audit.outcome'; \
		   IF v_decision < 2 THEN \
		     RAISE EXCEPTION 'COV_D12_GATE: canonical_events.decision >= 2 expected (ALLOW + STREAM), got %', v_decision; \
		   END IF; \
		   IF v_outcome < 2 THEN \
		     RAISE EXCEPTION 'COV_D12_GATE: canonical_events.outcome >= 2 expected (ALLOW + STREAM), got %', v_outcome; \
		   END IF; \
		   RAISE NOTICE 'COV_D12 CANONICAL OK: decision=% outcome=%', v_decision, v_outcome; \
		 END; \$$\$$;"
	@echo "[demo] COV_D12 SLICE 7 litellm_sdk_real verification done"

# COV_D12 SLICE 7 (litellm_sdk_deny demo) verification. Asserts the
# 3 fail-closed sub-steps produced exactly:
#   * >=1 reserve + >=1 commit (sub-step 1 ALLOW positive control)
#   * >=1 denied_decision (sub-step 2 reaches the sidecar)
# Sub-step 3 (bogus UDS) fails before the sidecar so no row from it.
demo-verify-litellm-sdk-deny:
	@$(COMPOSE) exec -T postgres psql -U spendguard -d spendguard_ledger -v ON_ERROR_STOP=1 < verify_step_litellm_sdk_deny.sql
	@echo "[demo] COV_D12 SLICE 7 litellm_sdk_deny verification done"

# COV_D06 SLICE 7 (vercel_ai_mastra demo) verification. Mirrors the
# langchain_ts layout: ledger-DB assertions in
# verify_step_vercel_ai_mastra.sql + INV-2 strict-order DO block + a
# cross-DB canonical_events check under spendguard_canonical that
# asserts at least 2 decision rows + 1 outcome row landed after the
# outbox forwarder drained.
demo-verify-vercel-ai-mastra:
	@$(COMPOSE) exec -T postgres psql -U spendguard -d spendguard_ledger -v ON_ERROR_STOP=1 < verify_step_vercel_ai_mastra.sql
	@echo "[demo] waiting 5s for outbox forwarder to land canonical_events rows..."
	@sleep 5
	@$(COMPOSE) exec -T postgres psql -U spendguard -d spendguard_canonical -v ON_ERROR_STOP=1 -c \
		"DO \$$\$$ DECLARE v_decision INT; v_outcome INT; BEGIN \
		   SELECT COUNT(*) INTO v_decision FROM canonical_events \
		    WHERE tenant_id = '00000000-0000-4000-8000-000000000001' \
		      AND event_type = 'spendguard.audit.decision'; \
		   SELECT COUNT(*) INTO v_outcome FROM canonical_events \
		    WHERE tenant_id = '00000000-0000-4000-8000-000000000001' \
		      AND event_type = 'spendguard.audit.outcome'; \
		   IF v_decision < 2 THEN \
		     RAISE EXCEPTION 'COV_D06_GATE: canonical_events.decision >= 2 expected (ALLOW + DENY), got %', v_decision; \
		   END IF; \
		   IF v_outcome < 1 THEN \
		     RAISE EXCEPTION 'COV_D06_GATE: canonical_events.outcome >= 1 expected (ALLOW commit), got %', v_outcome; \
		   END IF; \
		   RAISE NOTICE 'COV_D06 CANONICAL OK: decision=% outcome=%', v_decision, v_outcome; \
		 END; \$$\$$;"
	@echo "[demo] COV_D06 SLICE 7 vercel_ai_mastra verification done"

# COV_D38 SLICE 5 (mastra_processor demo) verification. Mirrors the
# demo-verify-langchain-ts layout (design §10): ledger-DB assertions in
# verify_step_mastra_processor.sql (COV_D38_GATE prefix: reserve >= 2,
# commit_estimated >= 2, denied_decision >= 1, INV-2 strict-order,
# audit decision rows >= 2) + a cross-DB canonical_events check under
# spendguard_canonical that asserts at least 2 decision rows + 1
# outcome row landed after the outbox forwarder drained + the
# outbox-closure check (per design §10 the closure check lives INSIDE
# this target — gate A5.4 runs it via `make demo-verify-mastra-processor`).
demo-verify-mastra-processor:
	@$(COMPOSE) exec -T postgres psql -U spendguard -d spendguard_ledger -v ON_ERROR_STOP=1 < verify_step_mastra_processor.sql
	@echo "[demo] waiting 5s for outbox forwarder to land canonical_events rows..."
	@sleep 5
	@$(COMPOSE) exec -T postgres psql -U spendguard -d spendguard_canonical -v ON_ERROR_STOP=1 -c \
		"DO \$$\$$ DECLARE v_decision INT; v_outcome INT; BEGIN \
		   SELECT COUNT(*) INTO v_decision FROM canonical_events \
		    WHERE tenant_id = '00000000-0000-4000-8000-000000000001' \
		      AND event_type = 'spendguard.audit.decision'; \
		   SELECT COUNT(*) INTO v_outcome FROM canonical_events \
		    WHERE tenant_id = '00000000-0000-4000-8000-000000000001' \
		      AND event_type = 'spendguard.audit.outcome'; \
		   IF v_decision < 2 THEN \
		     RAISE EXCEPTION 'COV_D38_GATE: canonical_events.decision >= 2 expected (ALLOW + DENY), got %', v_decision; \
		   END IF; \
		   IF v_outcome < 1 THEN \
		     RAISE EXCEPTION 'COV_D38_GATE: canonical_events.outcome >= 1 expected (ALLOW commit), got %', v_outcome; \
		   END IF; \
		   RAISE NOTICE 'COV_D38 CANONICAL OK: decision=% outcome=%', v_decision, v_outcome; \
		 END; \$$\$$;"
	@$(MAKE) demo-verify-outbox-closure
	@echo "[demo] COV_D38 SLICE 5 mastra_processor verification done"

# COV_D29 SLICE 5 (inngest_agent_kit demo) verification. Mirrors the
# openai_agents_ts layout: ledger-DB assertions in
# verify_step_inngest_agent_kit.sql + headline retry-dedup gate +
# INV-2 strict-order DO block + a cross-DB canonical_events check
# under spendguard_canonical that asserts at least 2 decision rows +
# 1 outcome row landed after the outbox forwarder drained.
#
# The D29 headline retry-dedup gate is the load-bearing assertion:
# 3 retry attempts of the same step body must collapse to ONE
# reservation row in the ledger. The SQL trips with
# `COV_D29_DEDUP_GATE` if the dedup contract is broken.
demo-verify-inngest-agent-kit:
	@$(COMPOSE) exec -T postgres psql -U spendguard -d spendguard_ledger -v ON_ERROR_STOP=1 < verify_step_inngest_agent_kit.sql
	@echo "[demo] waiting 5s for outbox forwarder to land canonical_events rows..."
	@sleep 5
	@$(COMPOSE) exec -T postgres psql -U spendguard -d spendguard_canonical -v ON_ERROR_STOP=1 -c \
		"DO \$$\$$ DECLARE v_decision INT; v_outcome INT; BEGIN \
		   SELECT COUNT(*) INTO v_decision FROM canonical_events \
		    WHERE tenant_id = '00000000-0000-4000-8000-000000000001' \
		      AND event_type = 'spendguard.audit.decision'; \
		   SELECT COUNT(*) INTO v_outcome FROM canonical_events \
		    WHERE tenant_id = '00000000-0000-4000-8000-000000000001' \
		      AND event_type = 'spendguard.audit.outcome'; \
		   IF v_decision < 2 THEN \
		     RAISE EXCEPTION 'COV_D29_GATE: canonical_events.decision >= 2 expected (ALLOW + DENY), got %', v_decision; \
		   END IF; \
		   IF v_outcome < 1 THEN \
		     RAISE EXCEPTION 'COV_D29_GATE: canonical_events.outcome >= 1 expected (ALLOW commit), got %', v_outcome; \
		   END IF; \
		   RAISE NOTICE 'COV_D29 CANONICAL OK: decision=% outcome=%', v_decision, v_outcome; \
		 END; \$$\$$;"
	@echo "[demo] COV_D29 SLICE 5 inngest_agent_kit verification done"

# COV_D07 SLICE 8 (maf_dotnet_real demo) verification. Mirrors the
# openai_agents_ts layout: ledger-DB assertions in
# verify_step_maf_dotnet.sql + INV-2 strict-order DO block + a
# cross-DB canonical_events check under spendguard_canonical that
# asserts at least 2 decision rows + 1 outcome row landed after the
# outbox forwarder drained.
demo-verify-maf-dotnet:
	@$(COMPOSE) exec -T postgres psql -U spendguard -d spendguard_ledger -v ON_ERROR_STOP=1 < verify_step_maf_dotnet.sql
	@echo "[demo] waiting 5s for outbox forwarder to land canonical_events rows..."
	@sleep 5
	@$(COMPOSE) exec -T postgres psql -U spendguard -d spendguard_canonical -v ON_ERROR_STOP=1 -c \
		"DO \$$\$$ DECLARE v_decision INT; v_outcome INT; BEGIN \
		   SELECT COUNT(*) INTO v_decision FROM canonical_events \
		    WHERE tenant_id = '00000000-0000-4000-8000-000000000001' \
		      AND event_type = 'spendguard.audit.decision'; \
		   SELECT COUNT(*) INTO v_outcome FROM canonical_events \
		    WHERE tenant_id = '00000000-0000-4000-8000-000000000001' \
		      AND event_type = 'spendguard.audit.outcome'; \
		   IF v_decision < 2 THEN \
		     RAISE EXCEPTION 'COV_D07_DOTNET_GATE: canonical_events.decision >= 2 expected (ALLOW + DENY), got %', v_decision; \
		   END IF; \
		   IF v_outcome < 1 THEN \
		     RAISE EXCEPTION 'COV_D07_DOTNET_GATE: canonical_events.outcome >= 1 expected (ALLOW commit), got %', v_outcome; \
		   END IF; \
		   RAISE NOTICE 'COV_D07_DOTNET CANONICAL OK: decision=% outcome=%', v_decision, v_outcome; \
		 END; \$$\$$;"
	@echo "[demo] COV_D07 SLICE 8 maf_dotnet_real verification done"

# COV_D07 SLICE 8 (maf_python_real demo) verification. Mirrors the
# maf_dotnet_real layout: ledger-DB assertions in
# verify_step_maf_python.sql + INV-2 strict-order DO block + a
# cross-DB canonical_events check under spendguard_canonical that
# asserts at least 2 decision rows + 1 outcome row landed after the
# outbox forwarder drained.
demo-verify-maf-python:
	@$(COMPOSE) exec -T postgres psql -U spendguard -d spendguard_ledger -v ON_ERROR_STOP=1 < verify_step_maf_python.sql
	@echo "[demo] waiting 5s for outbox forwarder to land canonical_events rows..."
	@sleep 5
	@$(COMPOSE) exec -T postgres psql -U spendguard -d spendguard_canonical -v ON_ERROR_STOP=1 -c \
		"DO \$$\$$ DECLARE v_decision INT; v_outcome INT; BEGIN \
		   SELECT COUNT(*) INTO v_decision FROM canonical_events \
		    WHERE tenant_id = '00000000-0000-4000-8000-000000000001' \
		      AND event_type = 'spendguard.audit.decision'; \
		   SELECT COUNT(*) INTO v_outcome FROM canonical_events \
		    WHERE tenant_id = '00000000-0000-4000-8000-000000000001' \
		      AND event_type = 'spendguard.audit.outcome'; \
		   IF v_decision < 2 THEN \
		     RAISE EXCEPTION 'COV_D07_PYTHON_GATE: canonical_events.decision >= 2 expected (ALLOW + DENY), got %', v_decision; \
		   END IF; \
		   IF v_outcome < 1 THEN \
		     RAISE EXCEPTION 'COV_D07_PYTHON_GATE: canonical_events.outcome >= 1 expected (ALLOW commit), got %', v_outcome; \
		   END IF; \
		   RAISE NOTICE 'COV_D07_PYTHON CANONICAL OK: decision=% outcome=%', v_decision, v_outcome; \
		 END; \$$\$$;"
	@echo "[demo] COV_D07 SLICE 8 maf_python_real verification done"

# COV_D19 SLICE 5 (agent_real_adk demo) verification. Mirrors the
# maf_python_real layout: ledger-DB assertions in
# verify_step_agent_real_adk.sql + INV-2 strict-order DO block + a
# cross-DB canonical_events check under spendguard_canonical that
# asserts at least 1 decision row + 1 outcome row landed after the
# outbox forwarder drained.
#
# Note: the COV_D19 DENY path goes through the synthetic LlmResponse
# channel — whether a `denied_decision` row appears depends on the
# contract evaluator's audit-write semantics on synthetic-response
# deny. The gate only requires the ALLOW pathway end-to-end; the
# DENY pathway is exercised at the runner level.
demo-verify-agent-real-adk:
	@$(COMPOSE) exec -T postgres psql -U spendguard -d spendguard_ledger -v ON_ERROR_STOP=1 < verify_step_agent_real_adk.sql
	@echo "[demo] waiting 5s for outbox forwarder to land canonical_events rows..."
	@sleep 5
	@$(COMPOSE) exec -T postgres psql -U spendguard -d spendguard_canonical -v ON_ERROR_STOP=1 -c \
		"DO \$$\$$ DECLARE v_decision INT; v_outcome INT; BEGIN \
		   SELECT COUNT(*) INTO v_decision FROM canonical_events \
		    WHERE tenant_id = '00000000-0000-4000-8000-000000000001' \
		      AND event_type = 'spendguard.audit.decision'; \
		   SELECT COUNT(*) INTO v_outcome FROM canonical_events \
		    WHERE tenant_id = '00000000-0000-4000-8000-000000000001' \
		      AND event_type = 'spendguard.audit.outcome'; \
		   IF v_decision < 1 THEN \
		     RAISE EXCEPTION 'COV_D19_GATE: canonical_events.decision >= 1 expected (ALLOW), got %', v_decision; \
		   END IF; \
		   IF v_outcome < 1 THEN \
		     RAISE EXCEPTION 'COV_D19_GATE: canonical_events.outcome >= 1 expected (ALLOW commit), got %', v_outcome; \
		   END IF; \
		   RAISE NOTICE 'COV_D19 CANONICAL OK: decision=% outcome=%', v_decision, v_outcome; \
		 END; \$$\$$;"
	@echo "[demo] COV_D19 SLICE 5 agent_real_adk verification done"

# COV_D20 SLICE 5 (agent_real_strands demo) verification. Mirrors the
# agent_real_adk layout: ledger-DB assertions in
# verify_step_agent_real_strands.sql + INV-2 strict-order DO block + a
# cross-DB canonical_events check under spendguard_canonical that
# asserts at least 1 decision row + 1 outcome row landed after the
# outbox forwarder drained, plus the strands integration tag is
# present on the decision rows.
demo-verify-agent-real-strands:
	@$(COMPOSE) exec -T postgres psql -U spendguard -d spendguard_ledger -v ON_ERROR_STOP=1 < verify_step_agent_real_strands.sql
	@echo "[demo] waiting 5s for outbox forwarder to land canonical_events rows..."
	@sleep 5
	@$(COMPOSE) exec -T postgres psql -U spendguard -d spendguard_canonical -v ON_ERROR_STOP=1 -c \
		"DO \$$\$$ DECLARE v_decision INT; v_outcome INT; v_strands INT; BEGIN \
		   SELECT COUNT(*) INTO v_decision FROM canonical_events \
		    WHERE tenant_id = '00000000-0000-4000-8000-000000000001' \
		      AND event_type = 'spendguard.audit.decision'; \
		   SELECT COUNT(*) INTO v_outcome FROM canonical_events \
		    WHERE tenant_id = '00000000-0000-4000-8000-000000000001' \
		      AND event_type = 'spendguard.audit.outcome'; \
		   SELECT COUNT(*) INTO v_strands FROM canonical_events \
		    WHERE tenant_id = '00000000-0000-4000-8000-000000000001' \
		      AND event_type = 'spendguard.audit.decision' \
		      AND payload_json::jsonb #>> '{decision_context,integration}' = 'strands'; \
		   IF v_decision < 1 THEN \
		     RAISE EXCEPTION 'COV_D20_GATE: canonical_events.decision >= 1 expected (ALLOW), got %', v_decision; \
		   END IF; \
		   IF v_outcome < 1 THEN \
		     RAISE EXCEPTION 'COV_D20_GATE: canonical_events.outcome >= 1 expected (ALLOW commit), got %', v_outcome; \
		   END IF; \
		   RAISE NOTICE 'COV_D20 CANONICAL OK: decision=% outcome=% strands_tagged=%', v_decision, v_outcome, v_strands; \
		 END; \$$\$$;"
	@echo "[demo] COV_D20 SLICE 5 agent_real_strands verification done"

# COV_D21 SLICE 3 (agent_real_dspy demo) verification. Mirrors the
# agent_real_strands layout: ledger-DB assertions in
# verify_step_agent_real_dspy.sql + INV-2 strict-order DO block + a
# cross-DB canonical_events check under spendguard_canonical that
# asserts at least 1 decision row landed after the outbox forwarder
# drained, plus the dspy integration tag is present on the decision
# rows. The outcome gate is a HARD EXCEPTION (HARDEN_D05_UR closed).
demo-verify-agent-real-dspy:
	@$(COMPOSE) exec -T postgres psql -U spendguard -d spendguard_ledger -v ON_ERROR_STOP=1 < verify_step_agent_real_dspy.sql
	@echo "[demo] waiting 5s for outbox forwarder to land canonical_events rows..."
	@sleep 5
	@$(COMPOSE) exec -T postgres psql -U spendguard -d spendguard_canonical -v ON_ERROR_STOP=1 -c \
		"DO \$$\$$ DECLARE v_decision INT; v_outcome INT; v_dspy INT; BEGIN \
		   SELECT COUNT(*) INTO v_decision FROM canonical_events \
		    WHERE tenant_id = '00000000-0000-4000-8000-000000000001' \
		      AND event_type = 'spendguard.audit.decision'; \
		   SELECT COUNT(*) INTO v_outcome FROM canonical_events \
		    WHERE tenant_id = '00000000-0000-4000-8000-000000000001' \
		      AND event_type = 'spendguard.audit.outcome'; \
		   SELECT COUNT(*) INTO v_dspy FROM canonical_events \
		    WHERE tenant_id = '00000000-0000-4000-8000-000000000001' \
		      AND event_type = 'spendguard.audit.decision' \
		      AND convert_from(decode(payload_json::jsonb->>'data_b64', 'base64'), 'UTF8')::jsonb \
		          #>> '{spendguard,integration}' = 'dspy'; \
		   IF v_decision < 1 THEN \
		     RAISE EXCEPTION 'COV_D21_GATE: canonical_events.decision >= 1 expected (ALLOW), got %', v_decision; \
		   END IF; \
		   IF v_dspy < 1 THEN \
		     RAISE EXCEPTION 'COV_D21_GATE: canonical_events.decision with integration=dspy >= 1 expected, got %', v_dspy; \
		   END IF; \
		   IF v_outcome < 1 THEN \
		     RAISE EXCEPTION 'COV_D21_GATE: canonical_events.outcome >= 1 expected (ALLOW commit), got %', v_outcome; \
		   END IF; \
		   RAISE NOTICE 'COV_D21 CANONICAL OK: decision=% outcome=% dspy_tagged=%', v_decision, v_outcome, v_dspy; \
		 END; \$$\$$;"
	@echo "[demo] COV_D21 SLICE 3 agent_real_dspy verification done"

# COV_D22 SLICE 4 (agent_real_agno demo) verification. Mirrors the
# agent_real_strands / agent_real_dspy layout: ledger-DB assertions in
# verify_step_agent_real_agno.sql + INV-2 strict-order DO block + a
# cross-DB canonical_events check under spendguard_canonical that
# asserts at least 1 decision row + 1 outcome row landed after the
# outbox forwarder drained, plus the agno integration tag is present
# on the decision rows.
demo-verify-agent-real-agno:
	@$(COMPOSE) exec -T postgres psql -U spendguard -d spendguard_ledger -v ON_ERROR_STOP=1 < verify_step_agent_real_agno.sql
	@echo "[demo] waiting 5s for outbox forwarder to land canonical_events rows..."
	@sleep 5
	@$(COMPOSE) exec -T postgres psql -U spendguard -d spendguard_canonical -v ON_ERROR_STOP=1 -c \
		"DO \$$\$$ DECLARE v_decision INT; v_outcome INT; v_agno INT; BEGIN \
		   SELECT COUNT(*) INTO v_decision FROM canonical_events \
		    WHERE tenant_id = '00000000-0000-4000-8000-000000000001' \
		      AND event_type = 'spendguard.audit.decision'; \
		   SELECT COUNT(*) INTO v_outcome FROM canonical_events \
		    WHERE tenant_id = '00000000-0000-4000-8000-000000000001' \
		      AND event_type = 'spendguard.audit.outcome'; \
		   SELECT COUNT(*) INTO v_agno FROM canonical_events \
		    WHERE tenant_id = '00000000-0000-4000-8000-000000000001' \
		      AND event_type = 'spendguard.audit.decision' \
		      AND payload_json::jsonb #>> '{decision_context,integration}' = 'agno'; \
		   IF v_decision < 1 THEN \
		     RAISE EXCEPTION 'COV_D22_GATE: canonical_events.decision >= 1 expected (ALLOW), got %', v_decision; \
		   END IF; \
		   IF v_outcome < 1 THEN \
		     RAISE EXCEPTION 'COV_D22_GATE: canonical_events.outcome >= 1 expected (ALLOW commit), got %', v_outcome; \
		   END IF; \
		   RAISE NOTICE 'COV_D22 CANONICAL OK: decision=% outcome=% agno_tagged=%', v_decision, v_outcome, v_agno; \
		 END; \$$\$$;"
	@echo "[demo] COV_D22 SLICE 4 agent_real_agno verification done"

# COV_D23 SLICE 4 (agent_real_beeai demo) verification. Mirrors the
# agent_real_agno / agent_real_dspy layout: ledger-DB assertions in
# verify_step_agent_real_beeai.sql + INV-2 strict-order DO block + a
# cross-DB canonical_events check under spendguard_canonical that
# asserts at least 1 decision row + 1 outcome row landed after the
# outbox forwarder drained, plus the beeai integration tag is present
# on the decision rows.
demo-verify-agent-real-beeai:
	@$(COMPOSE) exec -T postgres psql -U spendguard -d spendguard_ledger -v ON_ERROR_STOP=1 < verify_step_agent_real_beeai.sql
	@echo "[demo] waiting 5s for outbox forwarder to land canonical_events rows..."
	@sleep 5
	@$(COMPOSE) exec -T postgres psql -U spendguard -d spendguard_canonical -v ON_ERROR_STOP=1 -c \
		"DO \$$\$$ DECLARE v_decision INT; v_outcome INT; v_beeai INT; BEGIN \
		   SELECT COUNT(*) INTO v_decision FROM canonical_events \
		    WHERE tenant_id = '00000000-0000-4000-8000-000000000001' \
		      AND event_type = 'spendguard.audit.decision'; \
		   SELECT COUNT(*) INTO v_outcome FROM canonical_events \
		    WHERE tenant_id = '00000000-0000-4000-8000-000000000001' \
		      AND event_type = 'spendguard.audit.outcome'; \
		   SELECT COUNT(*) INTO v_beeai FROM canonical_events \
		    WHERE tenant_id = '00000000-0000-4000-8000-000000000001' \
		      AND event_type = 'spendguard.audit.decision' \
		      AND payload_json::jsonb #>> '{decision_context,integration}' = 'beeai'; \
		   IF v_decision < 1 THEN \
		     RAISE EXCEPTION 'COV_D23_GATE: canonical_events.decision >= 1 expected (ALLOW), got %', v_decision; \
		   END IF; \
		   IF v_outcome < 1 THEN \
		     RAISE EXCEPTION 'COV_D23_GATE: canonical_events.outcome >= 1 expected (ALLOW commit), got %', v_outcome; \
		   END IF; \
		   RAISE NOTICE 'COV_D23 CANONICAL OK: decision=% outcome=% beeai_tagged=%', v_decision, v_outcome, v_beeai; \
		 END; \$$\$$;"
	@echo "[demo] COV_D23 SLICE 4 agent_real_beeai verification done"

# COV_D24 SLICE 5 (agent_real_autogen / agent_real_ag2 demo) verification.
# Mirrors the agent_real_agno / agent_real_beeai layout: ledger-DB
# assertions in verify_step_agent_real_autogen.sql + INV-2 strict-order
# DO block + a cross-DB canonical_events check under spendguard_canonical
# that asserts at least 1 decision row + 1 outcome row landed after the
# outbox forwarder drained, plus the autogen integration tag is present
# on the decision rows.
#
# The same verifier covers both DEMO_MODE=agent_real_autogen and
# DEMO_MODE=agent_real_ag2 (review-standards §5: "Both modes share a
# single verify_step_autogen.sql"). The decision_context tag is the
# same string ('autogen') in both cases — the wrapper is one class —
# but the lineage label on decision rows tells operators which
# AssistantAgent flavor was loaded.
demo-verify-agent-real-autogen:
	@$(COMPOSE) exec -T postgres psql -U spendguard -d spendguard_ledger -v ON_ERROR_STOP=1 < verify_step_agent_real_autogen.sql
	@echo "[demo] waiting 5s for outbox forwarder to land canonical_events rows..."
	@sleep 5
	@$(COMPOSE) exec -T postgres psql -U spendguard -d spendguard_canonical -v ON_ERROR_STOP=1 -c \
		"DO \$$\$$ DECLARE v_decision INT; v_outcome INT; v_autogen INT; BEGIN \
		   SELECT COUNT(*) INTO v_decision FROM canonical_events \
		    WHERE tenant_id = '00000000-0000-4000-8000-000000000001' \
		      AND event_type = 'spendguard.audit.decision'; \
		   SELECT COUNT(*) INTO v_outcome FROM canonical_events \
		    WHERE tenant_id = '00000000-0000-4000-8000-000000000001' \
		      AND event_type = 'spendguard.audit.outcome'; \
		   SELECT COUNT(*) INTO v_autogen FROM canonical_events \
		    WHERE tenant_id = '00000000-0000-4000-8000-000000000001' \
		      AND event_type = 'spendguard.audit.decision' \
		      AND payload_json::jsonb #>> '{decision_context,integration}' = 'autogen'; \
		   IF v_decision < 1 THEN \
		     RAISE EXCEPTION 'COV_D24_GATE: canonical_events.decision >= 1 expected (ALLOW), got %', v_decision; \
		   END IF; \
		   IF v_outcome < 1 THEN \
		     RAISE EXCEPTION 'COV_D24_GATE: canonical_events.outcome >= 1 expected (ALLOW commit), got %', v_outcome; \
		   END IF; \
		   RAISE NOTICE 'COV_D24 CANONICAL OK: decision=% outcome=% autogen_tagged=%', v_decision, v_outcome, v_autogen; \
		 END; \$$\$$;"
	@echo "[demo] COV_D24 SLICE 5 agent_real_autogen verification done"

# COV_D25 SLICE 5 (agent_real_smolagents demo) verification. Mirrors
# demo-verify-agent-real-autogen layout: ledger-DB assertions in
# verify_step_agent_real_smolagents.sql + INV-2 strict-order DO block +
# a cross-DB canonical_events check under spendguard_canonical that
# asserts at least 1 decision row + 1 outcome row landed after the
# outbox forwarder drained, plus the smolagents integration tag is
# present on the decision rows.
demo-verify-agent-real-smolagents:
	@$(COMPOSE) exec -T postgres psql -U spendguard -d spendguard_ledger -v ON_ERROR_STOP=1 < verify_step_agent_real_smolagents.sql
	@echo "[demo] waiting 5s for outbox forwarder to land canonical_events rows..."
	@sleep 5
	@$(COMPOSE) exec -T postgres psql -U spendguard -d spendguard_canonical -v ON_ERROR_STOP=1 -c \
		"DO \$$\$$ DECLARE v_decision INT; v_outcome INT; v_smol INT; BEGIN \
		   SELECT COUNT(*) INTO v_decision FROM canonical_events \
		    WHERE tenant_id = '00000000-0000-4000-8000-000000000001' \
		      AND event_type = 'spendguard.audit.decision'; \
		   SELECT COUNT(*) INTO v_outcome FROM canonical_events \
		    WHERE tenant_id = '00000000-0000-4000-8000-000000000001' \
		      AND event_type = 'spendguard.audit.outcome'; \
		   SELECT COUNT(*) INTO v_smol FROM canonical_events \
		    WHERE tenant_id = '00000000-0000-4000-8000-000000000001' \
		      AND event_type = 'spendguard.audit.decision' \
		      AND payload_json::jsonb #>> '{decision_context,integration}' = 'smolagents'; \
		   IF v_decision < 1 THEN \
		     RAISE EXCEPTION 'COV_D25_GATE: canonical_events.decision >= 1 expected (ALLOW), got %', v_decision; \
		   END IF; \
		   IF v_outcome < 1 THEN \
		     RAISE EXCEPTION 'COV_D25_GATE: canonical_events.outcome >= 1 expected (ALLOW commit), got %', v_outcome; \
		   END IF; \
		   RAISE NOTICE 'COV_D25 CANONICAL OK: decision=% outcome=% smolagents_tagged=%', v_decision, v_outcome, v_smol; \
		 END; \$$\$$;"
	@echo "[demo] COV_D25 SLICE 5 agent_real_smolagents verification done"

# COV_D26 SLICE 5 (agent_real_letta demo) verification. Mirrors
# demo-verify-agent-real-autogen layout: ledger-DB assertions in
# verify_step_agent_real_letta.sql + INV-2 strict-order DO block + a
# cross-DB canonical_events check under spendguard_canonical that
# asserts at least 1 decision row + 1 outcome row landed after the
# outbox forwarder drained, with the `integration = 'letta'` tag.
demo-verify-agent-real-letta:
	@$(COMPOSE) exec -T postgres psql -U spendguard -d spendguard_ledger -v ON_ERROR_STOP=1 < verify_step_agent_real_letta.sql
	@echo "[demo] waiting 5s for outbox forwarder to land canonical_events rows..."
	@sleep 5
	@$(COMPOSE) exec -T postgres psql -U spendguard -d spendguard_canonical -v ON_ERROR_STOP=1 -c \
		"DO \$$\$$ DECLARE v_decision INT; v_outcome INT; v_letta INT; BEGIN \
		   SELECT COUNT(*) INTO v_decision FROM canonical_events \
		    WHERE tenant_id = '00000000-0000-4000-8000-000000000001' \
		      AND event_type = 'spendguard.audit.decision'; \
		   SELECT COUNT(*) INTO v_outcome FROM canonical_events \
		    WHERE tenant_id = '00000000-0000-4000-8000-000000000001' \
		      AND event_type = 'spendguard.audit.outcome'; \
		   SELECT COUNT(*) INTO v_letta FROM canonical_events \
		    WHERE tenant_id = '00000000-0000-4000-8000-000000000001' \
		      AND event_type = 'spendguard.audit.decision' \
		      AND payload_json::jsonb #>> '{decision_context,integration}' = 'letta'; \
		   IF v_decision < 1 THEN \
		     RAISE EXCEPTION 'COV_D26_GATE: canonical_events.decision >= 1 expected (ALLOW), got %', v_decision; \
		   END IF; \
		   IF v_outcome < 1 THEN \
		     RAISE EXCEPTION 'COV_D26_GATE: canonical_events.outcome >= 1 expected (ALLOW commit), got %', v_outcome; \
		   END IF; \
		   RAISE NOTICE 'COV_D26 CANONICAL OK: decision=% outcome=% letta_tagged=%', v_decision, v_outcome, v_letta; \
		 END; \$$\$$;"
	@echo "[demo] COV_D26 SLICE 5 agent_real_letta verification done"

# COV_D27 SLICE 5 (agent_real_llamaindex demo) verification. Mirrors
# demo-verify-agent-real-letta layout: ledger-DB assertions in
# verify_step_agent_real_llamaindex.sql + INV-2 strict-order DO block
# + a cross-DB canonical_events check under spendguard_canonical that
# asserts at least 1 decision row + 1 outcome row landed after the
# outbox forwarder drained.
demo-verify-agent-real-llamaindex:
	@$(COMPOSE) exec -T postgres psql -U spendguard -d spendguard_ledger -v ON_ERROR_STOP=1 < verify_step_agent_real_llamaindex.sql
	@echo "[demo] waiting 5s for outbox forwarder to land canonical_events rows..."
	@sleep 5
	@$(COMPOSE) exec -T postgres psql -U spendguard -d spendguard_canonical -v ON_ERROR_STOP=1 -c \
		"DO \$$\$$ DECLARE v_decision INT; v_outcome INT; BEGIN \
		   SELECT COUNT(*) INTO v_decision FROM canonical_events \
		    WHERE tenant_id = '00000000-0000-4000-8000-000000000001' \
		      AND event_type = 'spendguard.audit.decision'; \
		   SELECT COUNT(*) INTO v_outcome FROM canonical_events \
		    WHERE tenant_id = '00000000-0000-4000-8000-000000000001' \
		      AND event_type = 'spendguard.audit.outcome'; \
		   IF v_decision < 1 THEN \
		     RAISE EXCEPTION 'COV_D27_GATE: canonical_events.decision >= 1 expected (ALLOW), got %', v_decision; \
		   END IF; \
		   IF v_outcome < 1 THEN \
		     RAISE EXCEPTION 'COV_D27_GATE: canonical_events.outcome >= 1 expected (ALLOW commit), got %', v_outcome; \
		   END IF; \
		   RAISE NOTICE 'COV_D27 CANONICAL OK: decision=% outcome=%', v_decision, v_outcome; \
		 END; \$$\$$;"
	@echo "[demo] COV_D27 SLICE 5 agent_real_llamaindex verification done"

# COV_D28 SLICE 4 (agent_real_atomic_agents demo) verification.
# Mirrors demo-verify-agent-real-autogen / agent-real-beeai / letta
# layout: ledger-DB assertions in
# verify_step_agent_real_atomic_agents.sql + INV-2 strict-order DO
# block + a cross-DB canonical_events check under spendguard_canonical
# that asserts at least 1 decision row + 1 outcome row landed after
# the outbox forwarder drained, plus the atomic_agents integration
# tag is present on the decision rows.
demo-verify-agent-real-atomic-agents:
	@$(COMPOSE) exec -T postgres psql -U spendguard -d spendguard_ledger -v ON_ERROR_STOP=1 < verify_step_agent_real_atomic_agents.sql
	@echo "[demo] waiting 5s for outbox forwarder to land canonical_events rows..."
	@sleep 5
	@$(COMPOSE) exec -T postgres psql -U spendguard -d spendguard_canonical -v ON_ERROR_STOP=1 -c \
		"DO \$$\$$ DECLARE v_decision INT; v_outcome INT; v_atomic INT; BEGIN \
		   SELECT COUNT(*) INTO v_decision FROM canonical_events \
		    WHERE tenant_id = '00000000-0000-4000-8000-000000000001' \
		      AND event_type = 'spendguard.audit.decision'; \
		   SELECT COUNT(*) INTO v_outcome FROM canonical_events \
		    WHERE tenant_id = '00000000-0000-4000-8000-000000000001' \
		      AND event_type = 'spendguard.audit.outcome'; \
		   SELECT COUNT(*) INTO v_atomic FROM canonical_events \
		    WHERE tenant_id = '00000000-0000-4000-8000-000000000001' \
		      AND event_type = 'spendguard.audit.decision' \
		      AND payload_json::jsonb #>> '{decision_context,integration}' = 'atomic_agents'; \
		   IF v_decision < 1 THEN \
		     RAISE EXCEPTION 'COV_D28_GATE: canonical_events.decision >= 1 expected (ALLOW), got %', v_decision; \
		   END IF; \
		   IF v_outcome < 1 THEN \
		     RAISE EXCEPTION 'COV_D28_GATE: canonical_events.outcome >= 1 expected (ALLOW commit), got %', v_outcome; \
		   END IF; \
		   RAISE NOTICE 'COV_D28 CANONICAL OK: decision=% outcome=% atomic_agents_tagged=%', v_decision, v_outcome, v_atomic; \
		 END; \$$\$$;"
	@echo "[demo] COV_D28 SLICE 4 agent_real_atomic_agents verification done"

# COV_D08 SLICE 5 (openai_agents_ts demo) verification. Mirrors the
# langchain_ts / vercel_ai_mastra layout: ledger-DB assertions in
# verify_step_openai_agents_ts.sql + INV-2 strict-order DO block + a
# cross-DB canonical_events check under spendguard_canonical that
# asserts at least 2 decision rows + 1 outcome row landed after the
# outbox forwarder drained.
demo-verify-openai-agents-ts:
	@$(COMPOSE) exec -T postgres psql -U spendguard -d spendguard_ledger -v ON_ERROR_STOP=1 < verify_step_openai_agents_ts.sql
	@echo "[demo] waiting 5s for outbox forwarder to land canonical_events rows..."
	@sleep 5
	@$(COMPOSE) exec -T postgres psql -U spendguard -d spendguard_canonical -v ON_ERROR_STOP=1 -c \
		"DO \$$\$$ DECLARE v_decision INT; v_outcome INT; BEGIN \
		   SELECT COUNT(*) INTO v_decision FROM canonical_events \
		    WHERE tenant_id = '00000000-0000-4000-8000-000000000001' \
		      AND event_type = 'spendguard.audit.decision'; \
		   SELECT COUNT(*) INTO v_outcome FROM canonical_events \
		    WHERE tenant_id = '00000000-0000-4000-8000-000000000001' \
		      AND event_type = 'spendguard.audit.outcome'; \
		   IF v_decision < 2 THEN \
		     RAISE EXCEPTION 'COV_D08_GATE: canonical_events.decision >= 2 expected (ALLOW + DENY), got %', v_decision; \
		   END IF; \
		   IF v_outcome < 1 THEN \
		     RAISE EXCEPTION 'COV_D08_GATE: canonical_events.outcome >= 1 expected (ALLOW commit), got %', v_outcome; \
		   END IF; \
		   RAISE NOTICE 'COV_D08 CANONICAL OK: decision=% outcome=%', v_decision, v_outcome; \
		 END; \$$\$$;"
	@echo "[demo] COV_D08 SLICE 5 openai_agents_ts verification done"

# COV_D04 SLICE 5 (langchain_ts demo) verification. Mirrors the
# envoy_extproc layout: ledger-DB assertions in
# verify_step_langchain_ts.sql + INV-2 strict-order DO block + a
# cross-DB canonical_events check under spendguard_canonical that
# asserts at least 2 decision rows + 1 outcome row landed after the
# outbox forwarder drained.
demo-verify-langchain-ts:
	@$(COMPOSE) exec -T postgres psql -U spendguard -d spendguard_ledger -v ON_ERROR_STOP=1 < verify_step_langchain_ts.sql
	@echo "[demo] waiting 5s for outbox forwarder to land canonical_events rows..."
	@sleep 5
	@$(COMPOSE) exec -T postgres psql -U spendguard -d spendguard_canonical -v ON_ERROR_STOP=1 -c \
		"DO \$$\$$ DECLARE v_decision INT; v_outcome INT; BEGIN \
		   SELECT COUNT(*) INTO v_decision FROM canonical_events \
		    WHERE tenant_id = '00000000-0000-4000-8000-000000000001' \
		      AND event_type = 'spendguard.audit.decision'; \
		   SELECT COUNT(*) INTO v_outcome FROM canonical_events \
		    WHERE tenant_id = '00000000-0000-4000-8000-000000000001' \
		      AND event_type = 'spendguard.audit.outcome'; \
		   IF v_decision < 2 THEN \
		     RAISE EXCEPTION 'COV_D04_GATE: canonical_events.decision >= 2 expected (ALLOW + DENY), got %', v_decision; \
		   END IF; \
		   IF v_outcome < 1 THEN \
		     RAISE EXCEPTION 'COV_D04_GATE: canonical_events.outcome >= 1 expected (ALLOW commit), got %', v_outcome; \
		   END IF; \
		   RAISE NOTICE 'COV_D04 CANONICAL OK: decision=% outcome=%', v_decision, v_outcome; \
		 END; \$$\$$;"
	@echo "[demo] COV_D04 SLICE 5 langchain_ts verification done"

# COV_D39 SLICE 3 (ag_ui_events demo) verification — the hard gate
# (design.md §9 / implementation.md §8):
#   1. sse-probe captures the ag-ui-runner's recorded event stream.
#   2. verify_sse.py asserts the EXACT 4-frame sequence (snapshot →
#      created → committed → denied), required non-empty fields,
#      unit_id presence, ID joins, denied taxonomy, and the §7
#      canonical-bytes round-trip; prints RESERVATION_ID=<uuid>.
#   3. Display↔ledger join: that reservation_id must exist in the
#      ledger `reservations` projection for the demo tenant
#      (PK column: reservation_id — services/ledger/migrations/
#      0010_projections.sql; resolves the design §9.3 marker).
#   4. verify_step_ag_ui_events.sql house-style ledger gates.
#   5. TD-04: counting-stub /_count must report exactly 1 call — the
#      DENY step provably never reached the provider (enforcement
#      happened at the sidecar; AG-UI events are display-only).
demo-verify-ag-ui-events:
	@$(COMPOSE) -f ag_ui_events/docker-compose.yaml run --rm --no-deps sse-probe \
	  > /tmp/spendguard-ag-ui-capture.sse
	@python3 ag_ui_events/verify_sse.py /tmp/spendguard-ag-ui-capture.sse \
	  | tee /tmp/spendguard-ag-ui-verify.out
	@RES_ID=$$(grep '^RESERVATION_ID=' /tmp/spendguard-ag-ui-verify.out | cut -d= -f2); \
	  $(COMPOSE) exec -T postgres psql -U spendguard -d spendguard_ledger -v ON_ERROR_STOP=1 -c \
	  "DO \$$\$$ BEGIN IF NOT EXISTS (SELECT 1 FROM reservations WHERE reservation_id = '$$RES_ID'::uuid AND tenant_id = '00000000-0000-4000-8000-000000000001') THEN RAISE EXCEPTION 'COV_D39_GATE: SSE reservation_id % not found in ledger reservations', '$$RES_ID'; END IF; END; \$$\$$;"
	@$(COMPOSE) exec -T postgres psql -U spendguard -d spendguard_ledger -v ON_ERROR_STOP=1 \
	  < verify_step_ag_ui_events.sql
	@echo "[demo] TD-04: asserting counting-stub saw exactly 1 call (deny never reached the provider)..."
	@$(COMPOSE) -f ag_ui_events/docker-compose.yaml exec -T counting-stub python -c \
	  "import urllib.request, json, sys; \
	   calls = json.load(urllib.request.urlopen('http://localhost:8765/_count', timeout=5))['calls']; \
	   sys.exit(0 if calls == 1 else print(f'COV_D39_GATE: counting-stub calls == 1 expected, got {calls}', file=sys.stderr) or 1)"
	@echo "[demo] COV_D39 ag_ui_events verification done"

demo-verify-openclaw-base-url:
	@$(COMPOSE) -f openclaw_base_url/docker-compose.yaml exec -T postgres psql -U spendguard -d spendguard_ledger -v ON_ERROR_STOP=1 < verify_step_openclaw_base_url.sql
	@echo "[demo] waiting 5s for outbox forwarder to land canonical_events rows..."
	@sleep 5
	@$(COMPOSE) -f openclaw_base_url/docker-compose.yaml exec -T postgres psql -U spendguard -d spendguard_canonical -v ON_ERROR_STOP=1 -c \
		"DO \$$\$$ DECLARE v_decision INT; v_outcome INT; BEGIN \
		   SELECT COUNT(*) INTO v_decision FROM canonical_events \
		    WHERE tenant_id = '00000000-0000-4000-8000-000000000001' \
		      AND event_type = 'spendguard.audit.decision'; \
		   SELECT COUNT(*) INTO v_outcome FROM canonical_events \
		    WHERE tenant_id = '00000000-0000-4000-8000-000000000001' \
		      AND event_type = 'spendguard.audit.outcome'; \
		   IF v_decision <> 3 THEN \
		     RAISE EXCEPTION 'COV_D40A_GATE: canonical_events.decision == 3 expected (ALLOW + DENY + STREAM), got %', v_decision; \
		   END IF; \
		   IF v_outcome <> 2 THEN \
		     RAISE EXCEPTION 'COV_D40A_GATE: canonical_events.outcome == 2 expected (ALLOW + STREAM only; DENY must not commit), got %', v_outcome; \
		   END IF; \
		   RAISE NOTICE 'COV_D40A CANONICAL OK: decision=% outcome=%', v_decision, v_outcome; \
		 END; \$$\$$;"
	@$(COMPOSE) -f openclaw_base_url/docker-compose.yaml exec -T counting-stub python -c \
	  "import urllib.request, json, sys; \
	   calls = json.load(urllib.request.urlopen('http://localhost:8765/_count', timeout=5))['calls']; \
	   print(f'[demo] COV_D40A counting-stub calls={calls}'); \
	   sys.exit(0 if calls == 2 else print(f'COV_D40A_GATE: counting-stub calls == 2 expected (ALLOW + STREAM only; DENY must not dispatch), got {calls}', file=sys.stderr) or 1)"
	@$(MAKE) demo-verify-outbox-closure
	@echo "[demo] COV_D40A openclaw_base_url verification done"

demo-verify-openclaw-provider-plugin:
	@$(COMPOSE) -f openclaw_provider_plugin/docker-compose.yaml exec -T postgres psql -U spendguard -d spendguard_ledger -v ON_ERROR_STOP=1 < verify_step_openclaw_provider_plugin.sql
	@echo "[demo] waiting 5s for outbox forwarder to land canonical_events rows..."
	@sleep 5
	@$(COMPOSE) -f openclaw_provider_plugin/docker-compose.yaml exec -T postgres psql -U spendguard -d spendguard_canonical -v ON_ERROR_STOP=1 -c \
		"DO \$$\$$ DECLARE v_decision INT; v_outcome INT; BEGIN \
		   SELECT COUNT(*) INTO v_decision FROM canonical_events \
		    WHERE tenant_id = '00000000-0000-4000-8000-000000000001' \
		      AND event_type = 'spendguard.audit.decision'; \
		   SELECT COUNT(*) INTO v_outcome FROM canonical_events \
		    WHERE tenant_id = '00000000-0000-4000-8000-000000000001' \
		      AND event_type = 'spendguard.audit.outcome'; \
		   IF v_decision < 4 THEN \
		     RAISE EXCEPTION 'COV_D40B_GATE: canonical_events.decision >= 4 expected (ALLOW + DENY + STREAM + PROVIDER_ERROR), got %', v_decision; \
		   END IF; \
		   IF v_outcome < 3 THEN \
		     RAISE EXCEPTION 'COV_D40B_GATE: canonical_events.outcome >= 3 expected (ALLOW + STREAM + PROVIDER_ERROR), got %', v_outcome; \
		   END IF; \
		   RAISE NOTICE 'COV_D40B CANONICAL OK: decision=% outcome=%', v_decision, v_outcome; \
		 END; \$$\$$;"
	@$(COMPOSE) -f openclaw_provider_plugin/docker-compose.yaml exec -T counting-stub python -c \
	  "import urllib.request, json, sys; \
	   calls = json.load(urllib.request.urlopen('http://localhost:8765/_count', timeout=5))['calls']; \
	   print(f'[demo] COV_D40B counting-stub calls={calls}'); \
	   sys.exit(0 if calls == 3 else print(f'COV_D40B_GATE: counting-stub calls == 3 expected (ALLOW + STREAM + PROVIDER_ERROR only; DENY must not dispatch), got {calls}', file=sys.stderr) or 1)"
	@$(MAKE) demo-verify-outbox-closure
	@echo "[demo] COV_D40B openclaw_provider_plugin verification done"

demo-verify-litellm-real:
	@$(COMPOSE) exec -T postgres psql -U spendguard -d spendguard_ledger -v ON_ERROR_STOP=1 < verify_step_litellm_real.sql
	@echo "[demo] waiting 5s for outbox forwarder to land canonical_events rows..."
	@sleep 5
	@$(COMPOSE) exec -T postgres psql -U spendguard -d spendguard_canonical -v ON_ERROR_STOP=1 -c \
		"DO \$$\$$ DECLARE v_decision INT; v_outcome INT; BEGIN \
		   SELECT COUNT(*) INTO v_decision FROM canonical_events \
		    WHERE tenant_id = '00000000-0000-4000-8000-000000000001' \
		      AND event_type = 'spendguard.audit.decision'; \
		   SELECT COUNT(*) INTO v_outcome FROM canonical_events \
		    WHERE tenant_id = '00000000-0000-4000-8000-000000000001' \
		      AND event_type = 'spendguard.audit.outcome'; \
		   IF v_decision < 1 THEN \
		     RAISE EXCEPTION 'SLICE6_GATE: canonical_events.decision >= 1 expected, got %', v_decision; \
		   END IF; \
		   IF v_outcome < 1 THEN \
		     RAISE EXCEPTION 'SLICE6_GATE: canonical_events.outcome >= 1 expected, got %', v_outcome; \
		   END IF; \
		   RAISE NOTICE 'SLICE6 CANONICAL OK: decision=% outcome=%', v_decision, v_outcome; \
		 END; \$$\$$;"
	@echo "[demo] Slice C3 — verify GH #77 sidecar enrichment in canonical_events..."
	@$(COMPOSE) exec -T postgres psql -U spendguard -d spendguard_canonical -v ON_ERROR_STOP=1 -c \
		"DO \$$\$$ DECLARE v_with_spendguard INT; v_integration TEXT; BEGIN \
		   SELECT COUNT(*) INTO v_with_spendguard FROM canonical_events \
		    WHERE tenant_id = '00000000-0000-4000-8000-000000000001' \
		      AND event_type = 'spendguard.audit.decision' \
		      AND cost_advisor_safe_decode_payload(payload_json)->'spendguard' IS NOT NULL; \
		   IF v_with_spendguard < 1 THEN \
		     RAISE EXCEPTION 'SLICE_C3_GATE: canonical_events with spendguard sub-object >= 1 expected (GH #77 enrichment), got %', v_with_spendguard; \
		   END IF; \
		   SELECT (cost_advisor_safe_decode_payload(payload_json)->'spendguard'->>'integration') INTO v_integration FROM canonical_events \
		    WHERE tenant_id = '00000000-0000-4000-8000-000000000001' \
		      AND event_type = 'spendguard.audit.decision' \
		      AND cost_advisor_safe_decode_payload(payload_json)->'spendguard' IS NOT NULL \
		    LIMIT 1; \
		   IF v_integration <> 'litellm' THEN \
		     RAISE EXCEPTION 'SLICE_C3_GATE: spendguard.integration expected litellm, got %', v_integration; \
		   END IF; \
		   RAISE NOTICE 'SLICE_C3 GH#77 OK: % decision rows with spendguard sub-object; integration=%', v_with_spendguard, v_integration; \
		 END; \$$\$$;"
	@echo "[demo] Slice 6 litellm_real verification done (with Slice C3 Q3 reanimation)"

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

.PHONY: demo-verify-step7 demo-verify-step8 demo-verify-step7_5 demo-verify-step9 demo-verify-ttl-sweep demo-verify-outbox-closure demo-verify-session-reservation demo-verify-deny demo-verify-pricing demo-verify-p0_6 demo-verify-p1 demo-verify-p1_5 demo-verify-litellm-guardrail demo-verify-envoy-extproc demo-verify-kong-gateway-real demo-verify-dify-plugin-real demo-verify-langchain-ts demo-verify-vercel-ai-mastra demo-verify-mastra-processor demo-verify-openai-agents-ts demo-verify-inngest-agent-kit demo-verify-maf-dotnet demo-verify-maf-python demo-verify-litellm-sdk-real demo-verify-litellm-sdk-deny demo-verify-agent-real-adk demo-verify-subscription-meter demo-verify-import-devin-fixture demo-verify-import-manus-fixture demo-import-manus-fixture demo-verify-import-genspark-fixture demo-verify-all-d05-ur demo-verify-ag-ui-events demo-verify-openclaw-base-url demo-verify-openclaw-provider-plugin

# HARDEN_D05_UR master verification target — runs all 14+ demos sequentially
# and asserts each exits 0 with canonical success lines. This target replaces
# the per-demo outbox-closure softening that was in place during the marathon.
demo-verify-all-d05-ur:
	@echo "[harden_d05_ur] running all 14+ demos sequentially..."
	@$(MAKE) demo-up DEMO_MODE=langchain_ts && $(MAKE) demo-verify-langchain-ts
	@$(MAKE) demo-down
	@$(MAKE) demo-up DEMO_MODE=vercel_ai_mastra && $(MAKE) demo-verify-vercel-ai-mastra
	@$(MAKE) demo-down
	@$(MAKE) demo-up DEMO_MODE=openai_agents_ts && $(MAKE) demo-verify-openai-agents-ts
	@$(MAKE) demo-down
	@$(MAKE) demo-up DEMO_MODE=inngest_agent_kit && $(MAKE) demo-verify-inngest-agent-kit
	@$(MAKE) demo-down
	@$(MAKE) demo-up DEMO_MODE=agent_real_adk_stub && $(MAKE) demo-verify-agent-real-adk
	@$(MAKE) demo-down
	@$(MAKE) demo-up DEMO_MODE=agent_real_strands && $(MAKE) demo-verify-agent-real-strands
	@$(MAKE) demo-down
	@$(MAKE) demo-up DEMO_MODE=agent_real_dspy && $(MAKE) demo-verify-agent-real-dspy
	@$(MAKE) demo-down
	@$(MAKE) demo-up DEMO_MODE=agent_real_agno && $(MAKE) demo-verify-agent-real-agno
	@$(MAKE) demo-down
	@$(MAKE) demo-up DEMO_MODE=agent_real_beeai && $(MAKE) demo-verify-agent-real-beeai
	@$(MAKE) demo-down
	@$(MAKE) demo-up DEMO_MODE=agent_real_autogen && $(MAKE) demo-verify-agent-real-autogen
	@$(MAKE) demo-down
	@$(MAKE) demo-up DEMO_MODE=agent_real_smolagents && $(MAKE) demo-verify-agent-real-smolagents
	@$(MAKE) demo-down
	@$(MAKE) demo-up DEMO_MODE=agent_real_letta && $(MAKE) demo-verify-agent-real-letta
	@$(MAKE) demo-down
	@$(MAKE) demo-up DEMO_MODE=agent_real_llamaindex && $(MAKE) demo-verify-agent-real-llamaindex
	@$(MAKE) demo-down
	@$(MAKE) demo-up DEMO_MODE=agent_real_atomic_agents && $(MAKE) demo-verify-agent-real-atomic-agents
	@$(MAKE) demo-down
	@$(MAKE) demo-up DEMO_MODE=maf_python_real && $(MAKE) demo-verify-maf-python
	@$(MAKE) demo-down
	@$(MAKE) demo-up DEMO_MODE=maf_dotnet_real && $(MAKE) demo-verify-maf-dotnet
	@$(MAKE) demo-down
	@echo "[harden_d05_ur] ALL 14+ demos PASS"

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
