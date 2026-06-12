# `@spendguard/openclaw-provider-plugin`

SpendGuard provider wrapper for OpenClaw provider plugins. It reserves
budget through the SpendGuard sidecar before OpenClaw dispatches the upstream
provider call, then settles SUCCESS, provider error, timeout, or abort paths
against the same reservation.

> The OpenClaw provider plugin runs in the OpenClaw process. It is an
> enforcement hook, not a sandbox boundary. Operators should install it only
> in trusted OpenClaw deployments. Use D40a base-URL routing when the plugin
> API changes or when plugin installation is not acceptable.

## Status

`0.1.0-pre` closes coverage deliverable D40b. The OpenClaw provider-plugin
surface is pinned to `openclaw@2026.6.2` at commit
`d4819948f37d45fe8f1428401316eaae456cdf16`.

Locked spec set:
[`docs/specs/coverage/D40b_openclaw_provider_plugin/`](https://github.com/m24927605/agentic-spendguard/tree/main/docs/specs/coverage/D40b_openclaw_provider_plugin).

## Install

```bash
pnpm add @spendguard/sdk @spendguard/openclaw-provider-plugin openclaw
```

Node `>=22.19.0` is required by the pinned OpenClaw package. ESM-only.
`@spendguard/sdk` and `openclaw` are peer dependencies; your application's
lockfile owns the exact runtime install.

## Quickstart

Wrap the upstream OpenClaw provider at the provider runtime hook. D40b pins
that hook to `wrapStreamFn(ctx)`: SpendGuard reserves before it calls
`ctx.streamFn(params)` or the upstream provider's wrapper.

```ts
import { SpendGuardClient } from "@spendguard/sdk";
import { createSpendGuardOpenClawProvider } from "@spendguard/openclaw-provider-plugin";

const client = new SpendGuardClient({
  socketPath: "/var/run/spendguard/adapter.sock",
  tenantId: process.env.SPENDGUARD_TENANT_ID,
  runtimeKind: "openclaw-provider-plugin",
});
await client.connect();
await client.handshake();

const guardedProvider = createSpendGuardOpenClawProvider(upstreamProvider, {
  client,
  tenantId: process.env.SPENDGUARD_TENANT_ID,
  budgetId: process.env.SPENDGUARD_BUDGET_ID,
  windowInstanceId: process.env.SPENDGUARD_WINDOW_INSTANCE_ID,
  unitId: process.env.SPENDGUARD_UNIT_ID,
  pricing: {
    pricingVersion: process.env.SPENDGUARD_PRICING_VERSION ?? "",
    pricingHash: Uint8Array.from(
      Buffer.from(process.env.SPENDGUARD_PRICE_SNAPSHOT_HASH_HEX ?? "", "hex"),
    ),
    fxRateVersion: process.env.SPENDGUARD_FX_RATE_VERSION ?? "",
    unitConversionVersion: process.env.SPENDGUARD_UNIT_CONVERSION_VERSION ?? "",
  },
});
```

Register `guardedProvider` through OpenClaw's provider plugin registration
surface. The demo fixture shows the local install shape in
[`deploy/demo/openclaw_provider_plugin/openclaw.config.json`](https://github.com/m24927605/agentic-spendguard/tree/main/deploy/demo/openclaw_provider_plugin/openclaw.config.json).

## Options

| Option | Required | What it does |
|---|---|---|
| `client` | yes | Connected `SpendGuardClient`; you own `connect`, `handshake`, and `close`. |
| `tenantId` | yes | Tenant the call bills to. |
| `budgetId` | yes | Budget scope UUID for the projected claim. |
| `windowInstanceId` | yes | Ledger window-instance UUID; every reserve claim must carry it. |
| `unitId` | yes | Ledger unit-row UUID; every reserve claim must carry it. |
| `pricing` | yes | Pricing freeze tuple repeated on settlement; must match the reservation. |
| `route` | no | Route label. Default: `openclaw-provider`. |
| `claimEstimator` | no | Replaces the default `ceil(chars/4)` USD-micros claim projection. |
| `runIdProvider` | no | Provides the SpendGuard run id from the OpenClaw context. |

There is deliberately no `failOpen`, `degradeOnUnavailable`, or
`SPENDGUARD_DISABLE` option. A sidecar DENY or sidecar outage aborts before
the upstream provider is invoked.

## Lifecycle

1. Flatten OpenClaw prompt/messages.
2. Derive `runId`, `llmCallId`, and idempotency key via `@spendguard/sdk`.
3. `client.reserve({ trigger: "LLM_CALL_PRE", ... })`.
4. DENY / sidecar unavailable: throw before provider dispatch.
5. ALLOW: call the upstream provider.
6. Non-stream success: `commitEstimated({ outcome: "SUCCESS", ... })`.
7. Async stream success: settle exactly once after the iterator completes.
8. Provider error / timeout / abort: settle once with the matching outcome
   (`PROVIDER_ERROR`, `CLIENT_TIMEOUT`, or `RUN_ABORTED`) and rethrow.

Settlements reuse the reserve-time unit and pricing tuple. Failure paths use
the TS SDK single-event `commitEstimated` shape and do not set
`outcomeKind`.

## D40a fallback

D40a is the durable base-URL recipe:
[`docs/drop-in/openclaw/`](https://agenticspendguard.dev/docs/drop-in/openclaw/).
Use it when you cannot install in-process OpenClaw plugins or when OpenClaw's
plugin API churns. D40a routes OpenAI-compatible traffic through the
SpendGuard egress proxy; D40b is the in-process provider wrapper.

## Run the demo

```bash
make demo-down
make demo-up DEMO_MODE=openclaw_provider_plugin
make -C deploy/demo demo-verify-openclaw-provider-plugin
```

The demo is local and keyless. It boots the real SpendGuard sidecar, a local
OpenAI-compatible counting stub, and a Node 22.19 runner that validates the
OpenClaw plugin config fixture before calling the wrapper.

```text
[demo] openclaw_provider_plugin ALL 4 steps PASS (ALLOW + DENY + STREAM + PROVIDER_ERROR)
```

The hard gate proves:

| Step | Gate |
|---|---|
| ALLOW | Reserve precedes provider dispatch; stub counter increments once; SUCCESS settlement. |
| DENY | Sidecar denial aborts before dispatch; stub counter unchanged; no settlement. |
| STREAM | Async iterator drains; SUCCESS settles once after terminal usage. |
| PROVIDER_ERROR | Stub returns HTTP 500; wrapper settles `PROVIDER_ERROR` and rethrows. |

SQL verification asserts `reserve=3`, `commit_estimated=2`, `release=1`,
`denied_decision=1`, canonical `decision=4`, canonical `outcome=3`, and
counting-stub `calls=3`.

## Documentation

- [Integration guide](https://agenticspendguard.dev/docs/integrations/openclaw-provider-plugin/)
- [Demo overlay](https://github.com/m24927605/agentic-spendguard/tree/main/deploy/demo/openclaw_provider_plugin)
- [D40a base-URL recipe](https://agenticspendguard.dev/docs/drop-in/openclaw/)
- [CHANGELOG](./CHANGELOG.md) · [License notices](./LICENSE_NOTICES.md)

## License

Apache-2.0. OpenClaw is consumed as a peer dependency and is licensed
separately by the OpenClaw project.
