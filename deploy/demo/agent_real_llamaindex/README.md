# DEMO_MODE=agent_real_llamaindex

COV_D27 SLICE 5 — LlamaIndex Python adapter end-to-end demo.

## What this demo proves

`spendguard.integrations.llamaindex.SpendGuardLlamaIndexHandler`
subclasses `llama_index.core.callbacks.base_handler.BaseCallbackHandler`
and gates every `CBEventType.LLM` event published by LlamaIndex's
provider integrations (`llama-index-llms-openai`, `-anthropic`,
`-gemini`, `-bedrock-converse`). Operators install the handler via
`Settings.callback_manager = CallbackManager([handler])` once — every
LLM call across the entire LlamaIndex query graph routes through the
SpendGuard sidecar's `RequestDecision(LLM_CALL_PRE)` →
`EmitLlmCallPost(SUCCESS)` lifecycle.

* **ALLOW path** — `on_event_start` reserves budget BEFORE LlamaIndex
  dispatches the provider HTTP; the provider returns; `on_event_end`
  commits with real token usage extracted from `response.raw`.
* **DENY path** — `SpendGuardLlamaIndexDenied` raises from inside
  `on_event_start`; LlamaIndex's `CallbackManager.event(...)` context
  manager propagates it out through the enclosing `LLM.chat` /
  `LLM.predict` call BEFORE the provider HTTP fires. The counting-stub
  records ZERO new hits on the DENY turn (INV-2 zero-provider-HTTP).
* **Vendor coverage** — the same handler instance works against every
  provider sub-package via response-shape detection. OpenAI's
  `raw["usage"]["total_tokens"]` / Anthropic's `input_tokens +
  output_tokens` / Gemini's `usage_metadata.total_token_count` /
  Bedrock Converse's `inputTokens + outputTokens` are all extracted
  by the handler's `_extract_total_tokens` cascade.

Both paths land in the ledger DB (`reserve` + `commit_estimated` or
`denied_decision` rows) plus the canonical event log
(`spendguard.audit.decision` + `spendguard.audit.outcome` events).

## Two-path coverage matrix (read this first)

LlamaIndex providers split into two routes; D27 covers one of them and
the other is covered transitively by D12 (the LiteLLM SDK shim).

| LlamaIndex package | Coverage | Install |
|--------------------|----------|---------|
| `llama-index-llms-litellm` (`LiteLLM(...)`) | **D12** (LiteLLM SDK shim) — transitive | `pip install 'spendguard-sdk'` + `spendguard_litellm_shim.install(...)` |
| `llama-index-llms-openai` (`OpenAI(...)`) | **D27** — this demo | `pip install 'spendguard-sdk[llamaindex]'` |
| `llama-index-llms-anthropic` | **D27** | (same) |
| `llama-index-llms-gemini` / `-google-genai` | **D27** | (same) |
| `llama-index-llms-bedrock-converse` | **D27** | (same) |

Operators using mixed setups install BOTH D12 + D27. D12's contextvar
recursion guard prevents double-reservation on the LiteLLM-routed
inner call (the LlamaIndex event still fires PRE; D12 short-circuits
the inner `acompletion` reserve).

## Run it

```bash
cd deploy/demo
# Stub variant — no API key, uses MockLLM (CI gate).
make demo DEMO_MODE=agent_real_llamaindex_stub
# Live variant — uses the counting-stub on /v1/chat/completions
# (so no real OpenAI key needed even though llama-index-llms-openai
# is installed; OPENAI_BASE_URL overrides the endpoint).
make demo DEMO_MODE=agent_real_llamaindex
```

The Makefile target brings up the base stack (`postgres` + `sidecar` +
`ledger` + `canonical-ingest` + `outbox-forwarder`) then applies this
overlay (`counting-stub` + `llamaindex-runner`). The runner installs
`spendguard-sdk[llamaindex]` (which resolves `llama-index-core>=0.12`)
plus `llama-index-llms-openai` for the live variant, connects to the
sidecar via UDS, and drives one `VectorStoreIndex.from_documents` →
`query_engine.query("...")` cycle through the handler. Verification
gates:

* `verify_step_agent_real_llamaindex.sql` asserts `reserve >= 1` and
  `commit_estimated >= 1` in `ledger_transactions` and the INV-2
  strict-order timestamp ordering (earliest reserve predates earliest
  commit).
* `demo-verify-agent-real-llamaindex` follows with a cross-DB
  `canonical_events` check asserting at least one
  `spendguard.audit.decision` row landed and the decision/outcome
  counts match the expected ALLOW + DENY pattern.

## Smoke-fallback path

When the provider sub-package (`llama-index-llms-openai`) cannot be
installed in the runner container (CI smoke gate, network-restricted
environments), the driver falls back to `MockLLM` from
`llama-index-core` directly. The PRE/POST lifecycle still fires
end-to-end — only the upstream provider HTTP is replaced by MockLLM's
deterministic in-process response. This mirrors the
`agent_real_autogen` / `agent_real_letta` fallback patterns.

## DENY path proof

The DENY turn in the live driver exhausts the demo budget (sets the
SpendGuard contract cap to 0) then runs a second `query_engine.query`.
The handler raises `SpendGuardLlamaIndexDenied` from `on_event_start`
BEFORE the inner `OpenAI(...)._chat` would have hit
`counting-stub:/v1/chat/completions`. We assert by reading the
counting-stub's `/_count` endpoint before and after the DENY turn:
both reads return the same hit count, proving zero provider HTTP on
DENY.

[D02 closed CLI install]: ../litellm_proxy/
[D03 base-URL drop-in]: ../runtime/
[D12 LiteLLM SDK shim]: ../litellm_sdk_real/
