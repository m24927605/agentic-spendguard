# `@spendguard/inngest-agent-kit` — composite demo

Runnable Node example for the **`@spendguard/inngest-agent-kit`** adapter
— the headline retry-dedup gate for Inngest AgentKit `step.ai.infer`
calls.

Drives **three calls** through a SpendGuard-wrapped `step.ai`, mirroring
the matrix the LangChain.js / Vercel AI / OpenAI Agents TS composite
demos enforce in the same repository — but with `RETRY_DEDUP` replacing
`STREAM` because `step.ai.infer` is non-streaming by design (`design.md`
§3 non-goal):

- **ALLOW** — small message within budget → counter `+1`, `SUCCESS` commit.
- **DENY** — `spendguard_estimate_override` blows past the seeded
  `1B`-atomic hard-cap; the sidecar contract evaluator emits
  `SPENDGUARD_DENY`; `wrapWithSpendGuard`'s `reserve()` throws
  `DecisionDenied`; the wrapped `step.ai.infer` halts BEFORE the inner
  HTTP call leaves the process; counter `+0`.
- **RETRY_DEDUP** — replays the SAME step body 3× with the SAME
  `(runId, step.id, idempotencyKey)` and incremented `step.attempt`.
  With an `InMemoryIdempotencyCache` supplied to the wrap, the SpendGuard
  reserve fires EXACTLY ONCE across all 3 attempts even though the
  upstream HTTP layer fires 3 times. **This is the D29 headline
  contract** (review-standards §4).

Success line on a clean run (LOCKED — CI greps for the exact spelling):

```
[demo] inngest_agent_kit ALL 3 steps PASS (ALLOW + DENY + RETRY_DEDUP)
```

## Modes

### `--mock` (laptop iteration — no sidecar, no real Inngest dev runtime)

```bash
node index.mjs --mock
```

Drives an in-process `SpendGuardClient` double + an in-process `step.ai`
double through the SLICE 2 + SLICE 3 PRE/POST bracket. Asserts:

- `client.reserve` fires before `stepAi.infer` (review-standards §3.1).
- `stepAi.callCount === 1` after the DENY step (the canonical "DENY ⇒
  inner step.ai is NEVER invoked" invariant; review-standards §5.1).
- `client.commitEstimated` fires exactly once on the ALLOW path.
- 3 retry attempts of the same step produce **1 reserve / 3 commits / 3
  provider calls** — the dedup headline.

Exit code:

- `0` — all 3 invariants hold; success line printed.
- `7` — at least one invariant violated; FAIL line printed.

### `--real` (sidecar + counting-stub or real OpenAI)

```bash
SPENDGUARD_SIDECAR_UDS=/var/run/spendguard/adapter.sock \
SPENDGUARD_TENANT_ID=00000000-0000-4000-8000-000000000001 \
SPENDGUARD_BUDGET_ID=44444444-4444-4444-8444-444444444444 \
OPENAI_API_KEY=sk-... \
node index.mjs --real
```

Connects to a real SpendGuard sidecar UDS, builds a thin `step.ai`
stand-in over the OpenAI HTTP wire shape (the demo container does not
boot the full `@inngest/agent-kit` dev runtime — the SpendGuard wrap
operates purely on the `step.ai` shape so the adapter's contract is
verified independently of AgentKit's runtime install path), and drives
three SpendGuard-wrapped invocations.

The `DEMO_MODE=inngest_agent_kit` Makefile target in
`deploy/demo/Makefile` wires this up against a `counting-stub` upstream
and asserts the per-step counter delta is
`+1 / +0 / +(1 + SPENDGUARD_DEMO_RETRIES)` (default `+3`).

Required env (real mode):

| Variable                        | Default                                       | Purpose                                                                |
| ------------------------------- | --------------------------------------------- | ---------------------------------------------------------------------- |
| `SPENDGUARD_SIDECAR_UDS`        | `/var/run/spendguard/adapter.sock`            | Sidecar UDS path. The Makefile demo mounts this from the sidecar container. |
| `SPENDGUARD_TENANT_ID`          | `00000000-0000-4000-8000-000000000001`        | Tenant UUID — billed scope.                                            |
| `SPENDGUARD_BUDGET_ID`          | `44444444-4444-4444-8444-444444444444`        | Budget UUID — projected claim's `scopeId`.                             |
| `SPENDGUARD_DEMO_RETRIES`       | `2`                                           | Number of extra retry attempts the RETRY_DEDUP step replays.           |
| `SPENDGUARD_COUNTING_STUB_URL`  | `http://counting-stub:8765`                   | Optional. When set, the demo polls `/_count` between steps.            |
| `OPENAI_BASE_URL`               | `${SPENDGUARD_COUNTING_STUB_URL}/v1`          | Override to point the upstream HTTP at a different endpoint.            |
| `OPENAI_API_KEY`                | `demo-counting-stub-no-real-key`              | Required when `OPENAI_BASE_URL` resolves to `api.openai.com`.          |
| `SPENDGUARD_HANDSHAKE_TIMEOUT_MS` | `30000`                                     | Handshake retry budget.                                                |

## Wire it into your project

```ts
import { Inngest } from "inngest";
import { SpendGuardClient, InMemoryIdempotencyCache } from "@spendguard/sdk";
import { wrapWithSpendGuard } from "@spendguard/inngest-agent-kit";

const client = new SpendGuardClient({
  socketPath: "/var/run/spendguard/adapter.sock",
  tenantId: "00000000-0000-4000-8000-000000000001",
  runtimeKind: "inngest-agent-kit",
});
await client.connect();
await client.handshake();

const cache = new InMemoryIdempotencyCache();
const inngest = new Inngest({ id: "my-app" });

export const agentFn = inngest.createFunction(
  { id: "agent-fn", retries: 2 },
  { event: "agent/run" },
  async ({ step }) => {
    const sgStep = wrapWithSpendGuard(step.ai, client, {
      tenantId: "00000000-0000-4000-8000-000000000001",
      budgetId: "44444444-4444-4444-8444-444444444444",
      idempotencyCache: cache,
      claimEstimator: ({ model, body }) => [{
        scopeId: "44444444-4444-4444-8444-444444444444",
        amountAtomic: "1000000",
        unit: { unit: "USD_MICROS", denomination: 1 },
      }],
    });
    return await sgStep.infer("call-openai", { model, body });
  },
);
```

See [`docs/integrations/inngest-agent-kit`](https://github.com/m24927605/agentic-spendguard/blob/main/docs/site-v2/src/content/docs/docs/integrations/inngest-agent-kit.mdx)
for the full integration page including the retry-dedup contract
explainer.
