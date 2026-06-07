# `DEMO_MODE=inngest_agent_kit`

End-to-end docker-compose demo for the `@spendguard/inngest-agent-kit`
adapter — the headline retry-dedup gate for Inngest AgentKit `step.ai`
calls.

```bash
make demo-up DEMO_MODE=inngest_agent_kit
```

## What it boots

Layered on top of the base `deploy/demo/compose.yaml` stack
(`postgres + ledger + canonical-ingest + sidecar + webhook-receiver +
outbox-forwarder`):

| Service                       | Role                                                                                                                            |
| ----------------------------- | ------------------------------------------------------------------------------------------------------------------------------- |
| `counting-stub`               | In-network mock OpenAI `/v1/chat/completions` provider. Counts every call; serves `/_count` to readers.                          |
| `inngest-agent-kit-runner`    | Node 20 container — runs `examples/inngest-agent-kit/index.mjs --real` against the sidecar UDS, drives `wrapWithSpendGuard(step.ai)`. |

## What it asserts

Three calls drive through the SpendGuard-wrapped `step.ai`:

1. **ALLOW** — small message within budget. `wrapWithSpendGuard`'s
   `reserve()` returns `CONTINUE`, the wrapped `step.ai.infer` upstream
   HTTP call fires, the counting-stub counter increments by `1`,
   `commitEstimated()` fires `SUCCESS` with `usage.total_tokens`.
2. **DENY** — body carries `spendguard_estimate_override=2000000000`
   which blows past the demo-seeded `1B`-atomic hard-cap. The sidecar
   contract evaluator emits `SPENDGUARD_DENY`; `wrapWithSpendGuard`'s
   `reserve()` throws `DecisionDenied`; the inner `step.ai.infer` HTTP
   call never leaves the process. Counting-stub counter `+0` (proves the
   gate fires pre-call).
3. **RETRY_DEDUP** — driver re-invokes the SAME step body 3× with the
   SAME `(runId, step.id, idempotencyKey)` and incremented
   `step.attempt`. **The SpendGuard reserve fires EXACTLY ONCE** across
   all 3 attempts thanks to the in-process `InMemoryIdempotencyCache`.
   Counting-stub counter `+3` (one per attempt — the upstream HTTP layer
   still fires; the dedup happens above it at the SpendGuard reservation
   layer). This is the D29 headline contract (review-standards §4).

Success line on a clean run (LOCKED — CI greps for the exact spelling):

```
[demo] inngest_agent_kit ALL 3 steps PASS (ALLOW + DENY + RETRY_DEDUP)
```

## Verify gates

After the runner exits, the Makefile runs:

```bash
make demo-verify-inngest-agent-kit
```

Which executes [`verify_step_inngest_agent_kit.sql`](../verify_step_inngest_agent_kit.sql)
against `spendguard_ledger` and a cross-DB `canonical_events` count
against `spendguard_canonical`. The gates:

- `ledger_transactions.reserve >= 2` (ALLOW + RETRY_DEDUP each produce
  EXACTLY one reservation — RETRY_DEDUP produces ONE despite 3 attempts).
- `ledger_transactions.commit_estimated >= 2`.
- `ledger_transactions.denied_decision >= 1` (DENY step).
- **`reserve == 2` exactly (headline retry-dedup gate)** — if the
  RETRY_DEDUP step produces 3 reservations (one per attempt) instead of
  1, the gate fails. The SQL counts distinct `decision_id` rows in
  `ledger_transactions` for `operation_kind = 'reserve'` and asserts the
  total stays at `2`.
- `audit_outbox` carries `>= 2` decision rows in the last 5 min.
- `canonical_events` carries `>= 2` decision + `>= 1` outcome rows.
- INV-2 strict-order: first reservation row precedes first outcome row.

## Anti-scope

This overlay does NOT redeclare `postgres`, `sidecar`, `ledger`,
`canonical-ingest`, `bundles-init`, `pki-init` or any other base service —
the Makefile target brings up the base stack first, then this overlay.

## Related

- [`examples/inngest-agent-kit/`](../../../examples/inngest-agent-kit/) —
  the demo's source.
- [`sdk/typescript-inngest-agent-kit/`](../../../sdk/typescript-inngest-agent-kit/)
  — the adapter package.
- [`deploy/demo/langchain_ts/`](../langchain_ts/) — LangChain.js sibling
  demo (same shape, different framework).
- [`deploy/demo/vercel_ai_mastra/`](../vercel_ai_mastra/) — Vercel AI +
  Mastra sibling demo.
- [`deploy/demo/openai_agents_ts/`](../openai_agents_ts/) — OpenAI Agents
  SDK TS sibling demo.
