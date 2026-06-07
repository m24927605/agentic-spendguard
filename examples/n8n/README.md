# SpendGuard for n8n

This example shows how to wire SpendGuard's pre-call budget guardrails
into an n8n self-hosted AI Agent workflow with the
[`n8n-nodes-spendguard`](https://www.npmjs.com/package/n8n-nodes-spendguard)
community node.

## What's inside

- [`workflows/n8n_real.workflow.json`](workflows/n8n_real.workflow.json) — an
  importable workflow with `Manual Trigger → AI Agent` plus a
  `Chat Model → SpendGuard Chat Model → AI Agent` sub-node chain.

The SpendGuard Chat Model sub-node sits between the upstream
`lmChatAnthropic` (or any `ai_languageModel`-producing node) and the
`AI Agent` node. When the AI Agent invokes the model, SpendGuard fires
`LLM_CALL_PRE` against the configured sidecar UDS; a DENY surfaces in
the workflow execution as `NodeApiError(httpCode: "403")` with the
SpendGuard `decisionId` in the description.

## Install

1. Enable community packages on your self-hosted n8n:
   ```bash
   export N8N_COMMUNITY_PACKAGES_ENABLED=true
   ```
2. Install the package:
   ```bash
   n8n npm install n8n-nodes-spendguard
   ```
3. Add the SpendGuard API credential (tenant ID, sidecar UDS path,
   budget ID, window instance ID).
4. Import this workflow JSON.

## Notes

- **Self-hosted only.** n8n Cloud's policy blocks UDS / local FS mounts;
  the community node v0.1.x targets self-hosted.
- The sub-node returns the upstream model verbatim — no Proxy, no
  clone — so the AI Agent's RunManager events flow unchanged.
- See the [docs site integration page](https://agenticspendguard.dev/docs/integrations/n8n/)
  for the full worked example, screenshots, and known limitations.
