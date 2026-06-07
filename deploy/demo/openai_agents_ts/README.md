# `DEMO_MODE=openai_agents_ts`

End-to-end docker-compose demo for the `@spendguard/openai-agents`
adapter — the TS sibling of the Python `openai_agents_composite` demo.

```bash
make demo-up DEMO_MODE=openai_agents_ts
```

## What it boots

Layered on top of the base `deploy/demo/compose.yaml` stack
(`postgres + ledger + canonical-ingest + sidecar + webhook-receiver +
outbox-forwarder`):

| Service                 | Role                                                                                                |
| ----------------------- | --------------------------------------------------------------------------------------------------- |
| `counting-stub`         | In-network mock OpenAI `/v1/chat/completions` provider. Counts every call; serves `/_count` to readers. |
| `openai-agents-runner`  | Node 20 container — runs `examples/openai-agents-ts-composite/demo.mjs --real` against the sidecar UDS. |

## What it asserts

Three calls drive through the SpendGuard-wrapped `@openai/agents` `Agent`:

1. **ALLOW** — small message within budget. `client.reserve()` returns
   `CONTINUE`, the `OpenAIChatCompletionsModel` HTTP call fires, the
   counting-stub counter increments by `1`, `client.commitEstimated()`
   fires `SUCCESS` with `usage.totalTokens`.
2. **DENY** — Agent's `modelSettings.extraBody` carries
   `spendguard_estimate_override=2000000000` which blows past the
   demo-seeded `1B`-atomic hard-cap. The sidecar contract evaluator emits
   `SPENDGUARD_DENY`; `withSpendGuard`'s `reserve()` throws
   `DecisionDenied`; `Runner.run(...)` halts BEFORE the inner OpenAI HTTP
   call fires. Counting-stub counter `+0` (proves the gate fires pre-call).
3. **STREAM** — for `v0.1.x` the streaming path is **pass-through with no
   PRE/POST gating** (`design.md` §3 non-goal). The demo issues a second
   non-stream call to verify the bracket discipline survives across a
   stream interleave. Counter `+1`.

Success line on a clean run (LOCKED — CI greps for the exact spelling):

```
[demo] openai_agents_ts ALL 3 steps PASS (ALLOW + DENY + STREAM)
```

## Verify gates

After the runner exits, the Makefile runs:

```bash
make demo-verify-openai-agents-ts
```

Which executes [`verify_step_openai_agents_ts.sql`](../verify_step_openai_agents_ts.sql)
against `spendguard_ledger` and a cross-DB
`canonical_events` count against `spendguard_canonical`. The gates:

- `ledger_transactions.reserve >= 2` (ALLOW + STREAM each produce a reservation).
- `ledger_transactions.commit_estimated >= 2`.
- `ledger_transactions.denied_decision >= 1` (DENY step).
- `audit_outbox` carries `>= 2` decision rows in the last 5 min.
- `canonical_events` carries `>= 2` decision + `>= 1` outcome rows.
- INV-2 strict-order: first reservation row precedes first outcome row.

## Anti-scope

This overlay does NOT redeclare `postgres`, `sidecar`, `ledger`,
`canonical-ingest`, `bundles-init`, `pki-init` or any other base service —
the Makefile target brings up the base stack first, then this overlay.

## Related

- [`examples/openai-agents-ts-composite/`](../../../examples/openai-agents-ts-composite/)
  — the demo's source.
- [`sdk/typescript-openai-agents/`](../../../sdk/typescript-openai-agents/)
  — the adapter package.
- [`deploy/demo/langchain_ts/`](../langchain_ts/) —
  LangChain.js sibling demo (same shape, different framework).
- [`deploy/demo/vercel_ai_mastra/`](../vercel_ai_mastra/) —
  Vercel AI + Mastra sibling demo.
