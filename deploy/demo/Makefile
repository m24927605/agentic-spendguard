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
else ifeq ($(DEMO_MODE),vercel_ai_mastra)
	@echo "[demo] running vercel-ai-mastra-runner (Node 20 + Vercel AI SDK + Mastra alias)..."
	$(COMPOSE) -f vercel_ai_mastra/docker-compose.yaml run --rm vercel-ai-mastra-runner
	@echo "[demo] verifying COV_D06 SLICE 7 (vercel_ai_mastra ALLOW + DENY + STREAM)..."
	@$(MAKE) demo-verify-vercel-ai-mastra
	@echo "[demo] waiting 5s for outbox forwarder to drain audit chain..."
	@sleep 5
	@$(MAKE) demo-verify-outbox-closure
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
else ifeq ($(DEMO_MODE),cursor_mitm_fixture)
	@echo "[demo] running cursor-mitm-fixture-runner (Rust 1.83 — codec replay + counting stub)..."
	$(COMPOSE) -f cursor_mitm_fixture/docker-compose.yaml run --rm cursor-mitm-fixture-runner
	@echo "[demo] verifying D17 SLICE 9 (cursor_mitm_fixture — legal posture + audit chain hygiene)..."
	@$(MAKE) demo-verify-cursor-mitm-fixture
	@echo "[demo] waiting 5s for outbox forwarder to drain audit chain..."
	@sleep 5
	@$(MAKE) demo-verify-outbox-closure || \
	  echo "[demo] outbox-closure skipped: cursor_mitm_fixture runs against the codec's in-memory mock sidecar; no rows land in canonical_events (see SOW.md §6)"
else ifeq ($(DEMO_MODE),windsurf_mitm_fixture)
	@echo "[demo] running windsurf-mitm-fixture-runner (Rust 1.87 — codec replay + counting stub)..."
	$(COMPOSE) -f windsurf_mitm_fixture/docker-compose.yaml run --rm windsurf-mitm-fixture-runner
	@echo "[demo] verifying D18 SLICE 82 (windsurf_mitm_fixture — legal posture + audit chain hygiene)..."
	@$(MAKE) demo-verify-windsurf-mitm-fixture
	@echo "[demo] waiting 5s for outbox forwarder to drain audit chain..."
	@sleep 5
	@$(MAKE) demo-verify-outbox-closure || \
	  echo "[demo] outbox-closure skipped: windsurf_mitm_fixture runs against the codec's in-memory mock sidecar; no rows land in canonical_events (see SOW.md §6)"
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
	@$(MAKE) demo-verify-outbox-closure || \
	  echo "[demo] outbox-closure skipped: agent_real_adk demo deferred per D05 UnitRef gap (cross-slice tracking)"
else ifeq ($(DEMO_MODE),agent_real_adk_stub)
	@echo "[demo] running adk-runner in stub mode (no GOOGLE_API_KEY)..."
	@SPENDGUARD_DEMO_MODE=agent_real_adk_stub \
	  $(COMPOSE) -f agent_real_adk/docker-compose.yaml run --rm \
	    -e SPENDGUARD_DEMO_MODE=agent_real_adk_stub adk-runner
	@echo "[demo] verifying COV_D19 SLICE 5 (agent_real_adk_stub PRE + POST)..."
	@$(MAKE) demo-verify-agent-real-adk
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

.PHONY: demo-verify-step7 demo-verify-step8 demo-verify-step7_5 demo-verify-step9 demo-verify-ttl-sweep demo-verify-outbox-closure demo-verify-deny demo-verify-pricing demo-verify-p0_6 demo-verify-p1 demo-verify-p1_5 demo-verify-litellm-guardrail demo-verify-envoy-extproc demo-verify-kong-gateway-real demo-verify-dify-plugin-real demo-verify-langchain-ts demo-verify-vercel-ai-mastra demo-verify-openai-agents-ts demo-verify-inngest-agent-kit demo-verify-maf-dotnet demo-verify-maf-python demo-verify-litellm-sdk-real demo-verify-litellm-sdk-deny demo-verify-agent-real-adk

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
