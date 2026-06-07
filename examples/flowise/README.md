# Flowise + SpendGuard — end-to-end example

This directory holds a pre-baked Flowise chatflow plus a Node runner
script that demonstrate the production wire shape for
`@spendguard/flowise-nodes`. The demo container
(`deploy/demo/flowise_real/`) ships a focused integration-tier runner;
this directory ships the artefacts you would copy into a real Flowise
2.x install if you wanted to drive the full canvas.

## Files

| File                      | Purpose                                                                                      |
| ------------------------- | -------------------------------------------------------------------------------------------- |
| `chatflow.json`           | A serialised Flowise 2.x chatflow: `ChatOpenAI → SpendGuardChatModelWrapper → Conversation Chain`. |
| `run_flowise_real.mjs`    | Node 20 entry script — POSTs the chatflow to `POST /api/v1/chatflows`, then drives a prediction. |
| `README.md`               | This file.                                                                                   |

## How to run locally

1. Boot the demo stack: `make demo-up DEMO_MODE=flowise_real`.
2. Or boot Flowise 2.x by hand
   (`docker run -p 3000:3000 flowiseai/flowise:2`), install the wrapper
   per the package README, then:

```bash
export FLOWISE_URL=http://localhost:3000
export SPENDGUARD_SIDECAR_UDS=/run/spendguard/sg.sock
node run_flowise_real.mjs
```

The runner POSTs `chatflow.json` to Flowise's `POST /api/v1/chatflows`
endpoint, captures the returned chatflow ID, then POSTs a prediction to
`POST /api/v1/prediction/<id>` with prompt `hi`. The Flowise runtime
invokes `SpendGuardChatModelWrapper.init()`, which reserves spend via
the sidecar and appends the D04 callback handler before dispatching to
the wrapped ChatOpenAI node.

## Demo invariants

- The wrapper sits between the ChatOpenAI node and the Conversation
  Chain — neither node needs to know SpendGuard exists.
- DENY surfaces as an error in the Flowise prediction response; the
  upstream OpenAI HTTP does NOT fire (INV-1).
- ALLOW commits real `inputTokens + outputTokens` via the sidecar's
  `/v1/trace` endpoint (INV-5).

## Source

- Wrapper package: `integrations/flowise/`
- Demo overlay: `deploy/demo/flowise_real/`
- Spec: `docs/specs/coverage/D35_flowise/`
