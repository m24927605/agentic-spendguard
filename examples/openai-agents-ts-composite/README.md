# `@spendguard/openai-agents` — composite demo

Runnable Node example for the **`@spendguard/openai-agents`** adapter — the
TypeScript sibling of [`examples/openai-agents-composite/`](../openai-agents-composite/)
(Python).

It drives **three calls** through a SpendGuard-wrapped `@openai/agents`
`Model`, mirroring the `ALLOW + DENY + STREAM` matrix the LangChain.js /
Vercel AI / Pydantic-AI / LiteLLM composite demos enforce in the same
repository:

- **ALLOW** — small message within budget → counter `+1`, `SUCCESS` commit.
- **DENY** — `spendguard_estimate_override` blows past the seeded
  `1B`-atomic hard-cap; the sidecar contract evaluator emits
  `SPENDGUARD_DENY`; `withSpendGuard`'s `reserve()` throws `DecisionDenied`;
  `Runner.run(...)` halts BEFORE the inner OpenAI HTTP call leaves the
  process; counter `+0`.
- **STREAM** — for `v0.1.x` the streaming path is **pass-through with no
  PRE/POST gating** (`design.md` §3 non-goal). The demo drives a second
  non-stream call after the stream interleave to verify the bracket
  discipline survives.

Success line on a clean run (LOCKED — CI greps for the exact spelling):

```
[demo] openai_agents_ts ALL 3 steps PASS (ALLOW + DENY + STREAM)
```

## Modes

### `--mock` (laptop iteration — no sidecar, no real OpenAI)

```bash
node demo.mjs --mock
```

Drives an in-process `SpendGuardClient` double + an in-process `Model`
double through the SLICE 2 + SLICE 3 PRE/POST bracket. Asserts:

- `client.reserve` fires before `inner.getResponse` (reviewer gate 1.1).
- `inner.callCount === 0` after the DENY step (reviewer gate 1.6 — the
  canonical "DENY ⇒ inner Model is NEVER invoked" invariant).
- `client.commitEstimated` fires exactly once on the ALLOW path.

Exit code:

- `0` — all 3 invariants hold; success line printed.
- `7` — at least one invariant violated; FAIL line printed.

### `--real` (sidecar + counting-stub or real OpenAI)

```bash
SPENDGUARD_SIDECAR_UDS=/var/run/spendguard/adapter.sock \
SPENDGUARD_TENANT_ID=00000000-0000-4000-8000-000000000001 \
SPENDGUARD_BUDGET_ID=44444444-4444-4444-8444-444444444444 \
OPENAI_API_KEY=sk-... \
node demo.mjs --real
```

Connects to a real SpendGuard sidecar UDS, builds a real `@openai/agents`
`Agent`, and drives three `Runner.run(...)` invocations through
`withSpendGuard(model)`. The `DEMO_MODE=openai_agents_ts` Makefile target
in `deploy/demo/Makefile` wires this up against a `counting-stub`
upstream and asserts the per-step counter delta is `+1 / +0 / +1`.

The default `--real` config points at `counting-stub` so a token-free
demo works in CI; flip `OPENAI_BASE_URL=https://api.openai.com/v1`
(or omit the env var to take the SDK default) for a real OpenAI run.

Required env (real mode):

| Variable                       | Default                                         | Purpose                                                                 |
| ------------------------------ | ----------------------------------------------- | ----------------------------------------------------------------------- |
| `SPENDGUARD_SIDECAR_UDS`       | `/var/run/spendguard/adapter.sock`              | Sidecar UDS path. The Makefile demo mounts this from the sidecar container.|
| `SPENDGUARD_TENANT_ID`         | `00000000-0000-4000-8000-000000000001`          | Tenant UUID — billed scope.                                             |
| `SPENDGUARD_BUDGET_ID`         | `44444444-4444-4444-8444-444444444444`          | Budget UUID — projected claim's `scopeId`.                              |
| `SPENDGUARD_COUNTING_STUB_URL` | `http://counting-stub:8765`                     | Optional. When set, the demo polls `/_count` between steps to assert the per-step counter delta. Drop it for a token-free real-OpenAI run. |
| `OPENAI_BASE_URL`              | `${SPENDGUARD_COUNTING_STUB_URL}/v1`            | Override to point the OpenAI SDK at a different upstream.               |
| `OPENAI_API_KEY`               | `demo-counting-stub-no-real-key`                | Required when `OPENAI_BASE_URL` resolves to `api.openai.com`.           |
| `SPENDGUARD_HANDSHAKE_TIMEOUT_MS` | `30000`                                       | Handshake retry budget.                                                 |

## Mock mode invariants

The mock mode is the deliberate review-standards §1.6 enforcement point —
the demo MUST exit non-zero if any of these fail. They map 1:1 to the
behaviour-invariant gates in `docs/specs/coverage/D08_openai_agents_ts/review-standards.md`:

| Gate | Mock assertion                                                                                  |
| ---- | ------------------------------------------------------------------------------------------------ |
| 1.1  | After step 1, `client.reserveCount === 1` AND `inner.callCount === 1` (PRE before inner).        |
| 1.3  | After step 2 DENY, `inner.callCount` UNCHANGED from step 1 (still `1`).                          |
| 1.5  | Step 3 STREAM does NOT trigger a `reserve` call (pass-through).                                  |
| 1.6  | Step 2 throws `DecisionDenied` AND the per-step `inner.callCount` delta is `0`.                  |

The mock harness lives entirely inside `demo.mjs` for portability; the
SLICE 3 integration test suite under `sdk/typescript-openai-agents/tests/integration.test.ts`
covers the same matrix at the vitest level.

## Wire it into your project

```ts
import { Agent, Runner } from "@openai/agents";
import { OpenAIChatCompletionsModel, OpenAIProvider } from "@openai/agents-openai";
import { SpendGuardClient, newUuid7 } from "@spendguard/sdk";
import { withSpendGuard, runContext } from "@spendguard/openai-agents";

const client = new SpendGuardClient({
  socketPath: "/var/run/spendguard/adapter.sock",
  tenantId: "00000000-0000-4000-8000-000000000001",
  runtimeKind: "openai-agents-ts",
});
await client.connect();
await client.handshake();

const provider = new OpenAIProvider({ apiKey: process.env.OPENAI_API_KEY });
const inner = new OpenAIChatCompletionsModel(provider.openaiClient, "gpt-4o-mini");
const guarded = withSpendGuard(inner, {
  client,
  tenantId: "00000000-0000-4000-8000-000000000001",
  budgetId: "44444444-4444-4444-8444-444444444444",
});

const agent = new Agent({
  name: "my-budget-aware-agent",
  instructions: "Reply concisely.",
  model: guarded,
});

try {
  const runId = newUuid7();
  const result = await runContext({ runId }, () =>
    Runner.run(agent, "Say hello in three words."),
  );
  console.log("Runner.run OK", { runId, output: result.finalOutput });
} finally {
  await client.close();
}
```

See [`docs/integrations/openai-agents-ts`](https://github.com/m24927605/agentic-spendguard/blob/main/docs/site-v2/src/content/docs/docs/integrations/openai-agents-ts.mdx)
for the full integration page.
