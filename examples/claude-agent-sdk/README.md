# `claude-agent-sdk` + SpendGuard — egress-proxy recipe

Runnable Python example for Anthropic's first-party
[`claude-agent-sdk`](https://github.com/anthropics/claude-agent-sdk-python)
gated by the SpendGuard egress proxy. The SDK subprocesses the
`claude` CLI binary, so every `messages` request leaves your process
as an HTTPS call from inside the CLI — SpendGuard intercepts it at the
egress proxy after the [D02 `spendguard install`](https://agenticspendguard.dev/docs/integrations/claude-agent-sdk/#prerequisites)
step trusts the SpendGuard root CA on the host.

There is **no SpendGuard SDK adapter** for this framework. The SDK's
`PreToolUse` hook is **tool-scope**, not LLM-scope — it cannot
intercept the underlying `POST /v1/messages` exchange. The egress
proxy is the only honest LLM-scope gate. See the full integration
page for the architectural rationale:
[Anthropic claude-agent-sdk — egress proxy recipe](https://agenticspendguard.dev/docs/integrations/claude-agent-sdk/).

## What this example proves

One invariant: a single `query(...)` call against `claude-agent-sdk`
produces exactly one `RESERVE_RESPONSE` row plus one matching
`COMMIT_OUTCOME` row in `audit_outbox` with `provider = 'anthropic'`,
`model LIKE 'claude-%'`, and non-zero `committed_input_tokens` and
`committed_output_tokens` on the commit row.

## Prerequisites

- Python 3.11 or newer.
- A running SpendGuard egress proxy at `http://localhost:9000` — the
  [Quickstart](https://agenticspendguard.dev/docs/quickstart/) brings
  it up in 5 minutes.
- `spendguard install` ran clean on this host (D02). The installer
  drops the SpendGuard root CA into the OS trust store and writes
  `HTTPS_PROXY` plus `NODE_EXTRA_CA_CERTS` into your shell rc. Open a
  fresh terminal after running it so the env vars are picked up.
- `ANTHROPIC_API_KEY` exported in the current shell. BYOK only —
  Claude Code Pro / Max subscription metering lives in the separate
  [Subscription-tier meter](https://agenticspendguard.dev/docs/integrations/subscription-meter/)
  integration.

## Run it

```bash
cd examples/claude-agent-sdk
pip install -e .
python example.py
```

`pip install -e .` pulls `claude-agent-sdk` (which in turn pulls the
`claude` CLI binary). The example then issues one short prompt
through the SDK; the CLI subprocess inherits `HTTPS_PROXY` from your
shell and routes the `messages` request through the SpendGuard egress
proxy.

Expected output (one assistant message; exact text varies):

```
SystemMessage(...)
AssistantMessage(content=[TextBlock(text='1. slicing: s[::-1]; 2. reversed(): "".join(reversed(s)).')])
ResultMessage(...)
```

## Verify the audit chain

After the example exits, query `audit_outbox` directly to confirm the
two rows landed:

```sql
SELECT
    payload->>'event_type'              AS event_type,
    payload->>'provider'                AS provider,
    payload->>'model'                   AS model,
    payload->>'committed_input_tokens'  AS input_tokens,
    payload->>'committed_output_tokens' AS output_tokens,
    payload->>'request_id'              AS request_id
FROM audit_outbox
WHERE payload->>'provider' = 'anthropic'
  AND payload->>'model' LIKE 'claude-%'
ORDER BY created_at DESC
LIMIT 2;
```

Both rows MUST share the same `request_id`; the `COMMIT_OUTCOME` row
MUST carry `committed_input_tokens > 0` and
`committed_output_tokens > 0`. The
[SpendGuard dashboard](https://agenticspendguard.dev/docs/operations/dashboard/)
surfaces the same rows under "Recent decisions".

## Troubleshooting

- **`ssl.SSLCertVerificationError` from the CLI subprocess.** The
  SpendGuard root CA is not in the trust store the CLI is reading.
  Re-run `spendguard install`; on Linux verify
  `update-ca-certificates` ran; on macOS verify the cert was added
  to the system keychain via `security add-trusted-cert`. On Node,
  confirm `NODE_EXTRA_CA_CERTS` points at a PEM file the current
  user can read.
- **Calls hit `api.anthropic.com` directly — no audit rows.** Your
  current shell did not pick up the rc snippet that `spendguard install`
  wrote. Open a fresh terminal or `source ~/.zshrc` / `~/.bashrc`,
  then re-check `echo "$HTTPS_PROXY"`. If you launched Python from an
  IDE, restart the IDE so it inherits the updated environment.
- **`PreToolUse` blocks a tool but the LLM call still happened.**
  Working as designed — `PreToolUse` is tool-scope, not LLM-scope.
  Move the budget cap to the egress proxy (this recipe).

## Related

- [Anthropic claude-agent-sdk — egress proxy recipe](https://agenticspendguard.dev/docs/integrations/claude-agent-sdk/) —
  full integration page with PreToolUse rationale and TypeScript recipe.
- [Quickstart](https://agenticspendguard.dev/docs/quickstart/) — full
  SpendGuard stack up in 5 minutes.
- [Drop in 14 tools (overview)](https://agenticspendguard.dev/docs/drop-in/) —
  Pattern 2 quick wins for other OpenAI-compatible CLIs and IDEs.
