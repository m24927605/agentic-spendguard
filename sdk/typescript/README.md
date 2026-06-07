# `@spendguard/sdk`

[![npm](https://img.shields.io/npm/v/@spendguard/sdk.svg)](https://www.npmjs.com/package/@spendguard/sdk)
[![license](https://img.shields.io/npm/l/@spendguard/sdk.svg)](https://github.com/m24927605/agentic-spendguard/blob/main/LICENSE)
[![publish](https://github.com/m24927605/agentic-spendguard/actions/workflows/sdk-ts-publish.yml/badge.svg)](https://github.com/m24927605/agentic-spendguard/actions/workflows/sdk-ts-publish.yml)

> Runtime safety-layer client for AI-agent frameworks (TypeScript).
> Mirror of [`spendguard-sdk`](https://pypi.org/project/spendguard-sdk/) (Python).

`@spendguard/sdk` is the shared substrate that per-framework adapters
(`@spendguard/langchain`, `@spendguard/vercel-ai`, `@spendguard/openai-agents`,
`@spendguard/inngest-agentkit`) build against. It implements the gRPC-over-UDS
client for the SpendGuard sidecar, plus the deterministic helpers that produce
**byte-identical** output to the Python SDK and the Rust sidecar — the
audit-chain invariants that make idempotency replay and dedup correct
across languages.

## Install

```bash
pnpm add @spendguard/sdk @opentelemetry/api
# or
npm install @spendguard/sdk @opentelemetry/api
# or
bun add @spendguard/sdk @opentelemetry/api
```

`@opentelemetry/api` is an **optional** peer dep — install it only if you
plan to pass an `otelTracer`. Adapters that never enable OTel pay zero
extra dependency cost.

## Quick start

```ts
// reserve -> commitEstimated -> release  (the v0.1.0 happy path)
import {
  SpendGuardClient,
  deriveIdempotencyKey,
  newUuid7,
  USD_MICROS_PER_USD,
} from "@spendguard/sdk";

const client = new SpendGuardClient({
  tenantId: "acme-prod",
  socketPath: "/run/spendguard/sidecar.sock",
  handshake: { protocolVersion: 1, capabilities: {} },
});

await client.handshake();

const decision = await client.reserve({
  trigger: "LLM_CALL_PRE",
  runId: newUuid7(),
  stepId: newUuid7(),
  llmCallId: newUuid7(),
  decisionId: newUuid7(),
  route: "openai:gpt-4o-mini",
  projectedClaims: [
    { scopeId: "team:eng", amountAtomic: "150000", unit: { unit: "usd_micros", denomination: 1 } },
  ],
  idempotencyKey: deriveIdempotencyKey({
    tenantId: "acme-prod",
    sessionId: "sess-1",
    runId: "run-1",
    stepId: "step-1",
    llmCallId: "call-1",
    trigger: "LLM_CALL_PRE",
  }),
});

if (decision.decision === "CONTINUE") {
  // ...call the provider (OpenAI, Anthropic, etc.) here...
  await client.commitEstimated({
    runId: "run-1", stepId: "step-1", llmCallId: "call-1",
    decisionId: decision.decisionId,
    reservationId: decision.reservationIds[0]!,
    estimatedAmountAtomic: String(123 * USD_MICROS_PER_USD / 1_000_000),
    unit: { unit: "usd_micros", denomination: 1 },
    pricing: { pricingVersion: "v2026.05.09-1", pricingHash: new Uint8Array() },
    providerEventId: "openai:chatcmpl-abc",
    outcome: "SUCCESS",
  });
  await client.release({
    runId: "run-1", stepId: "step-1", llmCallId: "call-1",
    decisionId: decision.decisionId,
    reservationId: decision.reservationIds[0]!,
    releaseReason: "COMPLETED",
  });
}
```

The substrate also exposes `requestDecision()` as a referential alias for
`reserve()` (`client.reserve === client.requestDecision`) so call-sites
written against the Python SDK transliterate without semantic surprise.

## Subpath exports

Tree-shake what you need:

| Subpath | Contents |
|---|---|
| `@spendguard/sdk` | Full barrel (everything below). |
| `@spendguard/sdk/client` | `SpendGuardClient` + request/response types. |
| `@spendguard/sdk/errors` | Typed error hierarchy. |
| `@spendguard/sdk/ids` | `newUuid7`, `deriveIdempotencyKey`, `deriveUuidFromSignature`. |
| `@spendguard/sdk/pricing` | `PricingLookup`, `USD_MICROS_PER_USD`. |
| `@spendguard/sdk/pricing/demo` | Embedded `DEMO_PRICING` snapshot. |
| `@spendguard/sdk/promptHash` | `computePromptHash`. |
| `@spendguard/sdk/runPlan` | `withRunPlan`, `currentRunPlan` (AsyncLocalStorage). |
| `@spendguard/sdk/otel` | Optional OTel span wrapper. |
| `@spendguard/sdk/retry` | Bounded retry helper. |
| `@spendguard/sdk/cache` | In-memory idempotency cache. |
| `@spendguard/sdk/proto` | Generated proto types. |

## Cross-language byte-equivalence (P0)

Three functions in this SDK MUST produce byte-identical output to the
Python SDK and the Rust sidecar:

- `computePromptHash(text, tenantId)` — HMAC-SHA256 lowercase hex.
- `deriveIdempotencyKey({ ... })` — `sg-` + 32 hex chars.
- `deriveUuidFromSignature(sig, { scope })` — BLAKE2b-based UUID.

Drift breaks audit-chain rule dedup and the idempotency-replay collapse
contract. The corpus that pins this is shipped inside the npm tarball at
`fixtures/cross-language/v1.json` — both this SDK and the Python SDK consume
the same file. See `sdk/fixtures/cross-language/README.md` in the source repo
for the runbook.

## Compatibility

- **Node 20.10+** is the primary target.
- **Bun 1.1+** and **Deno 1.46+** are tested as secondary targets.
- Browser is **NOT supported in v0.1.x** — UDS transport is server-only.

## Links

- [Full design spec](https://github.com/m24927605/agentic-spendguard/blob/main/docs/specs/coverage/D05_ts_sdk_substrate/design.md)
  — public surface (§4), architecture (§6), locked decisions (§9), bundle-size budget (§10).
- [Implementation spec](https://github.com/m24927605/agentic-spendguard/blob/main/docs/specs/coverage/D05_ts_sdk_substrate/implementation.md)
  — repo layout + codegen pipeline.
- [Review standards](https://github.com/m24927605/agentic-spendguard/blob/main/docs/specs/coverage/D05_ts_sdk_substrate/review-standards.md)
  — P0/P1/P2 review gates applied to every D05 slice.
- [Python SDK on PyPI](https://pypi.org/project/spendguard-sdk/) — the
  lockstep counterpart.
- [CHANGELOG.md](./CHANGELOG.md) — release history.
- [LICENSE_NOTICES.md](./LICENSE_NOTICES.md) — third-party license attribution.

## License

Apache-2.0. See `LICENSE` in the repository root.
