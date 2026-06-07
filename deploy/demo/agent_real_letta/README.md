# DEMO_MODE=agent_real_letta

COV_D26 SLICE 5 — Letta (ex-MemGPT) Python adapter end-to-end demo.

## What this demo proves

`spendguard.integrations.letta.SpendGuardLettaClient` subclasses
`letta.llm_api.llm_client_base.LLMClientBase` (the LLM abstraction
shared by Letta 0.8+ — every per-provider client `OpenAIClient` /
`AnthropicClient` / `GoogleAIClient` / `DeepSeekClient` derives from
it) and wraps an inner `OpenAIClient` so every internal LLM call
inside `Agent.step()` routes through the SpendGuard sidecar's
`RequestDecision(LLM_CALL_PRE)` → `EmitLlmCallPost(SUCCESS)` lifecycle.

* **ALLOW path** — the wrapper reserves budget BEFORE Letta dispatches
  the model HTTP; the `Agent.step()` reaches the model; the wrapper
  commits with real `ChatCompletionResponse.usage.total_tokens`.
* **DENY path** — `DecisionDenied` propagates directly out of
  `wrapper.send_llm_request(...)` (no DEVIATION-1-style wrap needed —
  `LLMClientBase` has no framework-side catch on the
  `send_llm_request` path, verified against letta 0.8.0); the model
  is never called.
* **Provider coverage** — the same wrapper instance works against
  every Letta provider subclass via composition. `OpenAIClient` /
  `AnthropicClient` / `GoogleAIClient` / `DeepSeekClient` are all
  gated identically because gating sits at the ABC layer.

Both paths land in the ledger DB (`reserve` + `commit_estimated` or
`denied_decision` rows) plus the canonical event log
(`spendguard.audit.decision` + `spendguard.audit.outcome` events).

## When NOT to use D26 (read this first)

**~70% of Letta deployments are server-mode (`letta server` REST).**
For server mode use the [D02 closed CLI install] + [D03 base-URL
drop-in] egress-proxy path — D26 is the **library-mode-only** adapter.
LiteLLM-routed Letta deployments are covered transitively by the
[D12 LiteLLM SDK shim] without any D26 work.

| If you run Letta as | Use | Why |
|---|---|---|
| Self-hosted `letta server` REST | D02 + D03 egress proxy | One drop-in covers every provider call; no SDK changes in Letta |
| Embedded library (`from letta import ...`) — this demo | **D26 `wrap_llm_client(...)`** | The only safe per-call gate without upstream hooks |
| LiteLLM-routed (any Letta deployment) | D12 LiteLLM SDK shim covers transitively | No D26 work needed |

## Run it

```bash
cd deploy/demo
make demo DEMO_MODE=agent_real_letta
```

The Makefile target brings up the base stack (`postgres` + `sidecar` +
`ledger` + `canonical-ingest` + `outbox-forwarder`) then applies this
overlay (`counting-stub` + `letta-runner`). The runner installs
`spendguard-sdk[letta]` + `letta>=0.8,<1.0`, connects to the sidecar
via UDS, and drives one `Agent.step(...)` through the wrapper.
Verification gates:

* `verify_step_agent_real_letta.sql` asserts `reserve >= 1` and
  `commit_estimated >= 1` in `ledger_transactions` and the
  INV-2 strict-order timestamp ordering.
* `demo-verify-agent-real-letta` follows with a cross-DB
  `canonical_events` check asserting at least one
  `spendguard.audit.decision` row tagged with
  `decision_context.integration = 'letta'`.

## Smoke-fallback path

When `letta` cannot be installed in the runner container (CI smoke
gate, network-restricted environments), the driver falls back to a
duck-typed inner client (`_FakeInner`) that mirrors the
`LLMClientBase.send_llm_request` shape. The PRE/POST lifecycle still
fires end-to-end — only the upstream `Agent.step` orchestration is
skipped. This mirrors the `agent_real_autogen` / `agent_real_agno`
fallback pattern.

[D02 closed CLI install]: ../litellm_proxy/
[D03 base-URL drop-in]: ../runtime/
[D12 LiteLLM SDK shim]: ../litellm_sdk_real/
