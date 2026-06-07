# LangChain.js + SpendGuard — runnable Node example

> **Status: first-party reference example.** Drops a SpendGuard
> `SpendGuardCallbackHandler` into a stock LangChain.js `ChatOpenAI`
> so every `model.invoke()` / `model.stream()` reserves against a
> budget BEFORE the upstream OpenAI HTTP call leaves the process.
> No model wrapper, no proxy — drop into `callbacks: [handler]`.

This is the JS/TS sibling of
[`examples/litellm-proxy-composite/`](../litellm-proxy-composite/),
extracted into a standalone copy-pasteable Node project so contributors
can try the integration without spinning up the full Python demo
Docker stack.

## What this proves

Two hard invariants:

1. **SpendGuard DENY ⇒ the upstream provider is NEVER invoked.** If
   the budget is exhausted (or the contract evaluator emits
   `SPENDGUARD_DENY`), the handler's `handleChatModelStart` throws
   `DecisionDenied`; LangChain's `RunManager` propagates it through
   `model.invoke()` BEFORE ChatOpenAI's `fetch` call to the provider
   fires. Verified end-to-end by the DENY step's counting-stub
   `pre==post` assertion.
2. **End-of-stream commit reconciles real usage.** `handleLLMEnd`
   fires once per stream completion; the SUCCESS commit ships the
   provider's `usage.completion_tokens`, not the estimator
   worst-case.

## Quickstart

The example is wired for the in-tree counting-stub upstream (no real
OpenAI key required). To run end-to-end:

```bash
# 1) Boot the full SpendGuard demo stack with the langchain_ts overlay.
cd /path/to/agentic-spendguard
make -C deploy/demo demo-up DEMO_MODE=langchain_ts

# Equivalent: run the standalone example directly inside the demo
# network. The langchain-runner service builds this directory's
# package.json + index.mjs into a Node 20 container and executes it.
```

Expected output (from the langchain-runner container logs):

```
[demo] langchain_ts driver: socket=/var/run/spendguard/adapter.sock tenant=... openai_base=http://counting-stub:8765/v1
[demo] handshake ok session_id=...
[demo] (1) ALLOW step — invoking ChatOpenAI within budget
[demo] (1) ALLOW reply="..." counter pre=0 post=1
[demo] (2) DENY step — forcing hard-cap overflow
[demo] (2) DENY caught DecisionDenied: ...
[demo] (2) DENY counter pre=1 post=1 threw=true kind=DecisionDenied
[demo] (3) STREAM step — streaming chunks within budget
[demo] (3) STREAM chunks=1 counter pre=1 post=2
[demo] langchain_ts ALL 3 steps PASS (ALLOW + DENY + STREAM)
```

## Topology

```
┌─────────────────────────────────────────────────────────────┐
│  index.mjs (Node 20)                                        │
│    ChatOpenAI                                               │
│    + SpendGuardCallbackHandler                              │
│    + SpendGuardClient (UDS)                                 │
└────────────────┬───────────────┬────────────────────────────┘
                 │               │
                 │ UDS gRPC      │ HTTP (OpenAI baseURL)
                 ▼               ▼
┌─────────────────────────────────────────────────────────────┐
│  spendguard-sidecar (Rust)                                  │
│    • contract DSL (hard-cap, approval, deny rules)          │
│    • per-pod fencing lease                                  │
└────────────────┬────────────────────────────────────────────┘
                 │ mTLS gRPC
                 ▼
┌─────────────────────────────────────────────────────────────┐
│  spendguard-ledger (Rust) + Postgres                        │
│    • Stripe-style auth → capture                            │
│    • signed append-only audit chain                         │
└─────────────────────────────────────────────────────────────┘

                 ┌─────────────────────────────────────────────┐
                 │  counting-stub (Python) :8765               │
                 │    OpenAI-shape /v1/chat/completions        │
                 │    GET /_count for the running tally        │
                 └─────────────────────────────────────────────┘
```

## What you write

To deploy this against your own production LangChain.js stack, you write:

- Your `SpendGuardClient` — connects to your sidecar UDS (or TCP).
- One `SpendGuardCallbackHandler` per call (or share one across calls;
  the handler keys inflight state by LangChain's `runId`).
- Drop the handler onto every `BaseChatModel` / `BaseLLM` via
  `callbacks: [handler]`. No model subclassing required.

```ts
import { SpendGuardClient } from "@spendguard/sdk";
import { SpendGuardCallbackHandler } from "@spendguard/langchain";
import { ChatOpenAI } from "@langchain/openai";

const client = new SpendGuardClient({
  socketPath: "/var/run/spendguard/adapter.sock",
  tenantId: process.env.TENANT_ID,
  runtimeKind: "langchain-js",
});
await client.connect();
await client.handshake();

const handler = new SpendGuardCallbackHandler({ client });
const model = new ChatOpenAI({ model: "gpt-4o-mini", callbacks: [handler] });
const res = await model.invoke("hello");
```

See [`docs/specs/coverage/D04_langchain_ts/design.md`](../../docs/specs/coverage/D04_langchain_ts/design.md)
§5 for the full public-surface contract.

## File listing

| File | Purpose | LOC |
|---|---|---|
| `README.md` | this file | ~120 |
| `package.json` | Node 20 + LangChain.js + SpendGuard adapter pins | ~25 |
| `index.mjs` | 3-step demo runner (ALLOW + DENY + STREAM) | ~200 |

## Env vars

All forwarded by `deploy/demo/langchain_ts/docker-compose.yaml`. Override
at compose time if needed:

| Var | Default | Purpose |
|---|---|---|
| `SPENDGUARD_SIDECAR_UDS` | `/var/run/spendguard/adapter.sock` | UDS path inside the container |
| `SPENDGUARD_TENANT_ID` | demo UUID | tenant scope |
| `OPENAI_BASE_URL` | `http://counting-stub:8765/v1` | ChatOpenAI upstream |
| `OPENAI_API_KEY` | `demo-counting-stub-no-real-key` | non-empty placeholder; counting stub does not validate |
| `SPENDGUARD_COUNTING_STUB_URL` | `http://counting-stub:8765` | for the `/_count` probe in the demo driver |
| `SPENDGUARD_DEMO_STEP` | (unset) | optionally run a single step (`allow` / `deny` / `stream`) instead of all 3 |

## Limitations

- Counting stub upstream: the example uses an in-container HTTP stub
  instead of real OpenAI. To swap in real providers, drop the
  `configuration.baseURL` override on the `ChatOpenAI` constructor and
  set `OPENAI_API_KEY` in the environment.
- Single-tenant: the example wires one tenant/budget. Multi-tenant
  dispatch is the consumer's job — call `new SpendGuardClient({
  tenantId })` per tenant or rely on env-resolved defaults.
- DEGRADE patch application: v0.1.0 surfaces DEGRADE outcomes through
  the substrate's error path; built-in claim mutation lands in a later
  slice (mirrors the Python `spendguard-sdk[langchain]` v0.5.1 stance).

## Tear-down

```bash
make -C deploy/demo demo-down
```

## Related

- [`docs/specs/coverage/D04_langchain_ts/`](../../docs/specs/coverage/D04_langchain_ts/) — D04 spec
- [`sdk/typescript-langchain/`](../../sdk/typescript-langchain/) — `@spendguard/langchain` source
- [`deploy/demo/langchain_ts/`](../../deploy/demo/langchain_ts/) — demo compose overlay
- Other examples: [`litellm-proxy-composite/`](../litellm-proxy-composite/), [`openai-agents-composite/`](../openai-agents-composite/)
