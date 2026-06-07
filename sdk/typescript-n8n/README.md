# `n8n-nodes-spendguard`

> SpendGuard pre-call budget enforcement for n8n AI Agent workflows.
> Drop a sub-node between any Chat Model and the AI Agent — the AI
> Agent's run manager fires `LLM_CALL_PRE` against the SpendGuard
> sidecar before the provider HTTP fires.

**Self-hosted n8n only.** n8n Cloud's runner policy blocks UDS / local
FS mounts; v0.1.x targets self-hosted.

## Install

```bash
# Enable community packages on your self-hosted n8n.
export N8N_COMMUNITY_PACKAGES_ENABLED=true

# Install the package.
n8n npm install n8n-nodes-spendguard
```

In the n8n editor, add a **SpendGuard API** credential (tenant ID,
sidecar UDS path, budget ID, window instance ID) and drop the
**SpendGuard Chat Model** node between any `ai_languageModel`-producing
node and your **AI Agent**.

## Wiring

```
[Anthropic Chat Model] ──(ai_languageModel)──> [SpendGuard Chat Model] ──(ai_languageModel)──> [AI Agent]
```

The SpendGuard Chat Model node's `supplyData()`:

1. Reads the SpendGuard API credential.
2. Resolves the upstream `BaseChatModel` via
   `getInputConnectionData(AiLanguageModel, 0)`.
3. Attaches a `@spendguard/langchain` `SpendGuardCallbackHandler` to
   the model's `callbacks` array (no Proxy, no clone — the same model
   reference is returned downstream).
4. Returns the upstream model verbatim.

When the AI Agent invokes the model, the callback handler fires
`LLM_CALL_PRE` against the SpendGuard sidecar UDS. DENY throws
`DecisionDenied`; n8n's run manager surfaces it as
`NodeApiError(httpCode: "403")` with the SpendGuard `decisionId` in the
description.

## Behaviour

| Sidecar verdict      | n8n surface                                                 |
| -------------------- | ----------------------------------------------------------- |
| ALLOW                | Model invokes normally; `handleLLMEnd` commits real usage.  |
| DENY                 | `NodeApiError(httpCode: "403")` — no upstream HTTP.         |
| APPROVAL_REQUIRED    | `NodeApiError(httpCode: "428")` with approvalRequestId.     |
| UNAVAILABLE          | `NodeApiError(httpCode: "503")`.                            |
| HANDSHAKE failure    | `NodeApiError(httpCode: "502")`.                            |

## Run identity

The SpendGuard `sessionId` is n8n's `executionId`; the `stepId` is the
node display name. The `runId` is composed via the node's
**Run ID Source** parameter:

- `Execution ID + Node Name` (default) → `${executionId}:${nodeName}`.
- `Node Name` → `nodeName`.
- `Custom Expression` → user-supplied expression (or
  `${executionId}:${nodeName}` when empty).

The SpendGuard idempotency key is derived via
`@spendguard/sdk::deriveIdempotencyKey` so audit-chain dedup works
across Python, TS, and every other SpendGuard adapter.

## Limitations

- **Self-hosted only.** n8n Cloud is blocked by UDS / local FS policy.
- **n8n ≥ 1.50.** Earlier releases used string literals instead of
  `NodeConnectionType` enum constants.
- **CJS only.** n8n's community-node loader does not support ESM.
- **AiLanguageModel only.** `ai_tool` and `ai_memory` are
  contract-layer concerns; out of scope for v0.1.x.

## Worked example

See [`examples/n8n/workflows/n8n_real.workflow.json`](https://github.com/m24927605/agentic-spendguard/blob/main/examples/n8n/workflows/n8n_real.workflow.json)
for an importable workflow with Manual Trigger → AI Agent +
Chat Model → SpendGuard Chat Model → AI Agent.

## Demo

```bash
make demo-up DEMO_MODE=n8n_real
```

3-step matrix — ALLOW + DENY + STREAM — against the in-network mock
SpendGuard sidecar. Asserts INV-1 (DENY skips upstream) + INV-5
(real usage commits) at the SpendGuard ledger layer.

## License

Apache-2.0. See `LICENSE_NOTICES.md`.
