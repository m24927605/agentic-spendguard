# @spendguard/flowise-nodes

Pre-call budget enforcement for any [Flowise](https://flowiseai.com/)
canvas. Drop the **SpendGuard ChatModel Wrapper** node between any
`BaseChatModel` and your downstream Chain / Agent, and every chat
invocation reserves spend against the SpendGuard sidecar **before** the
upstream HTTP fires. DENY surfaces as an error the chat flow can catch;
ALLOW commits real `inputTokens + outputTokens` end-of-call.

The wrapper is a thin glue node — internally it attaches the
[`SpendGuardCallbackHandler`](https://www.npmjs.com/package/@spendguard/langchain)
from D04 to the wrapped chat model's `callbacks` array. Same audit
chain, same KMS-signed CloudEvents, same SQL gates as the Python /
LangChain.js paths — just packaged as a Flowise `INode` so the no-code
builder doesn't need to write TypeScript.

## Install

### Path 1 — npm install into Flowise source

```bash
cd /path/to/Flowise
pnpm add @spendguard/flowise-nodes
pnpm restart
```

The wrapper appears under **Spend Guard → SpendGuard ChatModel Wrapper**
in the canvas sidebar.

### Path 2 — drop-in for the official Docker image

```bash
npm pack @spendguard/flowise-nodes
mkdir -p ~/.flowise/nodes/spendguard
tar -xzf spendguard-flowise-nodes-0.1.0.tgz -C ~/.flowise/nodes/spendguard --strip-components=1
docker restart flowise
```

### Path 3 — custom Dockerfile layer

```dockerfile
FROM flowiseai/flowise:2
USER root
RUN npm install -g @spendguard/flowise-nodes && \
    cp -r /usr/local/lib/node_modules/@spendguard/flowise-nodes/dist \
          /root/.flowise/nodes/spendguard
USER node
```

## Configure on the canvas

| Input                       | Required | Notes                                                                                  |
| --------------------------- | -------- | -------------------------------------------------------------------------------------- |
| `chatModel`                 | required | Any Flowise `BaseChatModel` (ChatOpenAI, ChatAnthropic, ChatBedrock, ...).             |
| `tenantId`                  | required | SpendGuard tenant UUID.                                                                |
| `budgetId`                  | required | SpendGuard budget UUID to charge.                                                      |
| `windowInstanceId`          | required | Active SpendGuard budget-window instance UUID.                                         |
| `unit`                      | default  | Defaults to `usd_micros`; override per provider unit.                                  |
| `sidecarUds`                | optional | Falls back to env `SPENDGUARD_SIDECAR_UDS`.                                            |
| `route`                     | optional | Defaults to `llm.call`; per-route SpendGuard policies attach here.                     |
| `claimEstimatorJson`        | optional | JSON `{"amountAtomic":"...","scopeId":"..."}`. Empty → $1 USD-micros default per call. |

## Behaviour

| Path                       | What happens                                                                                                                       |
| -------------------------- | ---------------------------------------------------------------------------------------------------------------------------------- |
| Sidecar **ALLOW**          | Wrapper appends the handler; the wrapped model dispatches upstream as normal; commit posts real `inputTokens + outputTokens`.      |
| Sidecar **DENY**           | The D04 handler throws inside `handleChatModelStart`. Flowise surfaces the error in the prediction response.                       |
| Sidecar **DEGRADE**        | Default fail-closed — handler throws.                                                                                              |
| `claimEstimatorJson` blank | Conservative $1 USD-micros default claim per call (works out-of-the-box for a no-code builder).                                    |
| `claimEstimatorJson` set   | JSON override drives the reservation amount.                                                                                       |

## Limitations

- **Self-hosted Flowise only.** Flowise Cloud doesn't allow custom node
  drop-in.
- **ChatModel anchor only.** Embeddings, vector stores, tool nodes, and
  RAG sources are gated via the SpendGuard egress proxy (see the
  [drop-in guide](https://agenticspendguard.dev/docs/drop-in/)).
- **Per-node budget only.** Cross-node budget invariants are the
  contract layer's job (D04 / D05).
- **No mid-stream cap.** End-of-stream commit; the sidecar pre-call cap
  still gates ahead of the upstream HTTP.

## Demo

```bash
make demo-up DEMO_MODE=flowise_real
```

Brings up the base SpendGuard stack plus an in-network mock OpenAI and a
Node 20 runner that exercises the wrapper's reserve / commit / release
lifecycle in a 3-step matrix (ALLOW + DENY + STREAM). The matrix
verifies INV-1 (DENY skips upstream) and INV-5 (real usage commits) at
the SpendGuard ledger layer.

## Spec

`docs/specs/coverage/D35_flowise/`
