# D10 — Dify Model Provider Plugin — design.md

**Status:** Spec — Tier 2, build plan §2.2.
**Parent strategy:** [`docs/strategy/framework-coverage-2026-06.md`](../../../strategy/framework-coverage-2026-06.md) — "Largest no-code SaaS alternative".
**Owner:** Backend Architect.
**Closest analog:** [`sdk/python/src/spendguard/integrations/litellm.py`](../../../../sdk/python/src/spendguard/integrations/litellm.py) — same proxy-style reserve/commit/release.

## 1. Problem

Dify v1.0+ ships the **Model Provider Plugin SDK** (`langgenius/dify-plugin-sdks` ≥ 0.2) — a Python plugin daemon running in a separate container. Dify core routes `chat-messages` / `completion-messages` to the plugin daemon, which is the *only* gate that catches every LLM call from a Dify workspace (chat apps, agents, workflows, RAG steps). Dify has no budget primitive (only a `quota` for rate-limiting) and no signed audit. Self-hosted Dify against OpenAI / Anthropic / Bedrock has no pre-call dollar gate, no reservation, no audit chain.

D10 ships SpendGuard as a Dify Model Provider Plugin named `spendguard`. Admins select "SpendGuard" in the provider dropdown; `_invoke()` reserves against the sidecar, forwards to the real upstream (OpenAI, Anthropic, Gemini, Bedrock), commits real usage on success, releases on failure. Same lifecycle as the LiteLLM proxy callback (`litellm.py` §3.4 Path B), different transport.

## 2. Goals

1. Distributable plugin `spendguard-dify-plugin` built via `dify plugin init` / `dify plugin package`, installable to self-hosted Dify and Dify Cloud.
2. Extends `LargeLanguageModel` with `_invoke()`, `_generate()`, `_stream_generate()`, `validate_credentials()`, `get_num_tokens()` per v1 SDK contract.
3. Reserve via sidecar UDS+mTLS **before** any upstream HTTP; DENY → `InvokeAuthorizationError`; DEGRADE → `InvokeServerUnavailableError` (fail-closed).
4. Configurable upstream: `credentials.upstream_provider` ∈ `{openai, anthropic, gemini, bedrock}`; passthrough of `upstream_api_key` / `upstream_base_url`. Transparent forwarder once granted.
5. Streaming: `_stream_generate()` proxies SSE chunk-by-chunk; commit at end-of-stream with real usage.
6. Demo: `make demo-up DEMO_MODE=dify_plugin_real` boots self-hosted Dify + plugin daemon + sidecar + a real upstream call; verifies reserve → upstream → commit.
7. Docs page `docs/site/docs/integrations/dify.md` with install commands for Cloud (`dify plugin install`) and self-host (`.difypkg` sideload or compose mount).

## 3. Non-goals

- Workflow-step gating beyond model provider (RAG retrieval, tool-call cost on workflow edges): future slot types, not v1.
- Replacing Dify's native quota (operator rate limit, orthogonal axis).
- Bedrock IAM / GCP SA inside Dify — passthrough only.
- Per-app fine-grained budget keys (resolver reads `workspace_id` / `app_id` only in v1).

## 4. Architecture

```
Dify core (chat-messages) → ModelManager → plugin RPC bus
   ↓
spendguard-dify-plugin daemon container
  SpendGuardLLM(LargeLanguageModel)
    _invoke(model, credentials, prompt_messages, ...)
      1. Build BudgetBinding (env-driven default resolver)
      2. Estimator → projected claim
      3. SpendGuardClient.request_decision (UDS, mTLS) → ALLOW|DENY|DEGRADE
      4. ALLOW → forward to upstream via {openai|anthropic|gemini|bedrock} client
      5. SUCCESS → emit_llm_call_post(SUCCESS, real usage)
      6. FAILURE → emit_llm_call_post(FAILURE), re-raise
```

Dify core never sees provider HTTP — it talks only to the plugin RPC bus. Clean MITM point, no header rewriting required in Dify core.

## 5. Key decisions

- **Composition with a `_DifyReservation` delegate, not inheritance.** Delegate owns the reserve/commit/release dance; `SpendGuardLLM` only translates the Dify SDK signature. Same rationale as D11 `SpendGuardGuardrail._delegate`.
- **Per-upstream forwarder via an `UpstreamClient` interface.** v1 ships OpenAI + Anthropic; Gemini + Bedrock land as follow-up slices (tracked as GH issues, not v1 blockers).
- **End-of-stream commit only.** Mirrors `_async_log_success_streaming` (`litellm.py:572`). Mid-stream cap out of scope.
- **Plugin daemon runs in its own container** per Dify v1 contract; image base = `langgenius/dify-plugin-sdk-python:0.2-slim`; bundles SpendGuard SDK + upstream SDKs.
- **`validate_credentials()` issues a 1-token reserve+release roundtrip** against the sidecar to prove SpendGuard wiring at install time, not just an upstream credentials probe.
- **`get_num_tokens()` reuses the sidecar `count_tokens` UDS RPC** rather than bundling `tiktoken` (image-size + supply-chain win).
- **Fail-closed default.** DEGRADE → `InvokeServerUnavailableError`. Dev escape: `SPENDGUARD_DIFY_FAIL_OPEN=1`, mirroring `SPENDGUARD_LITELLM_FAIL_OPEN`.
- **Tenant mapping:** Dify `credentials["workspace_id"]` → SpendGuard `tenant_id`. Single-process, multi-workspace.

## 6. Slice plan

| Slice | Title | Size |
|-------|-------|------|
| `COV_D10_S1_plugin_scaffold` | `dify plugin init` skeleton under `plugins/dify/` | S |
| `COV_D10_S2_provider_manifest` | `manifest.yaml` + `provider/spendguard.yaml` credentials schema | S |
| `COV_D10_S3_llm_class_skeleton` | `SpendGuardLLM` + `_DifyReservation` delegate | M |
| `COV_D10_S4_openai_upstream` | OpenAI upstream — non-streaming `_invoke` / `_generate` | M |
| `COV_D10_S5_anthropic_upstream` | Anthropic upstream + `get_num_tokens` via sidecar | M |
| `COV_D10_S6_streaming_path` | `_stream_generate` SSE proxy + end-of-stream commit | M |
| `COV_D10_S7_demo_mode` | `DEMO_MODE=dify_plugin_real` + docker-compose + verify SQL | M |
| `COV_D10_S8_docs_publish` | `docs/site/docs/integrations/dify.md` + publish job | S |

8 slices, ~2200 LOC (~1100 impl + 700 test + 400 yaml/docs/compose).

## 7. Open questions (locked at spec write)

1. **SDK floor:** `dify-plugin-sdks >= 0.2.0`. v0.1 had a breaking `_invoke()` signature change.
2. **`get_num_tokens` accuracy:** sidecar `count_tokens` only. No bundled tokenizer in v1.
3. **Streaming fallback when upstream omits `usage`:** estimator-snapshot commit + WARN (same as `litellm.py:599-607`).
4. **Bedrock IAM / GCP SA:** deferred. v1 covers OpenAI + Anthropic where install friction is lowest.
5. **Cloud vs self-host install:** both supported; docs page lists `.difypkg` sideload for self-host and registry install for Dify Cloud.
