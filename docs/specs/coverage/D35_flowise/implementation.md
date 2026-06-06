# D35 — Implementation

This document specifies the directory layout, file responsibilities, and code skeleton for `@spendguard/flowise-nodes`. Pair with `design.md` (public surface) and `tests.md` (verification). SDK symbols imported from `@spendguard/sdk` and `@spendguard/langchain` are LOCKED at D05 §4 and D04 §4 — D35 does not re-derive them.

## 1. Repo layout

```
sdk/typescript/integrations/flowise/
├── package.json
├── tsconfig.json
├── tsup.config.ts
├── biome.json
├── vitest.config.ts
├── README.md
├── LICENSE_NOTICES.md
├── CHANGELOG.md
├── src/
│   ├── index.ts                      # public re-exports (Flowise nodes list)
│   ├── nodes/
│   │   └── SpendGuardChatModelWrapper.ts   # the INode class
│   ├── clientCache.ts                # module-level SpendGuardClient cache keyed by sidecarUds
│   ├── claimEstimator.ts             # JSON-string → ClaimEstimator helper for the no-code path
│   ├── assets/
│   │   └── spendguard.svg            # canvas icon (bundled at build time, base64-embedded)
│   └── version.ts                    # auto-generated VERSION constant
└── tests/
    ├── wrapper.test.ts               # unit tests vs. mock sidecar + mock BaseChatModel
    ├── clientCache.test.ts           # cache hit / miss, separate UDS paths get separate clients
    ├── claimEstimator.test.ts        # JSON parsing, default fallback, error paths
    ├── _support/
    │   ├── mockSidecar.ts            # re-exports D04 mock helper
    │   └── mockChatModel.ts          # tiny BaseChatModel sub that fires real RunManager events
    └── e2e/
        └── flowiseContainer.test.ts  # testcontainers vs flowiseai/flowise:2.x
```

Top-level files at `examples/flowise/` (slice 5):

```
examples/flowise/
├── README.md
├── chatflow.json                     # pre-baked Flowise chatflow using SpendGuardChatModelWrapper
└── run_flowise_real.ts               # Node entry: POST chatflow, invoke, assert
```

`examples/flowise/` is wired into the existing `deploy/demo/demo/run_demo.py` orchestrator via a `DEMO_MODE == "flowise_real"` dispatch branch (slice 5 §4). The Flowise instance runs in its own compose service alongside the sidecar.

## 2. `package.json` skeleton

```json
{
  "name": "@spendguard/flowise-nodes",
  "version": "0.1.0",
  "description": "SpendGuard Flowise custom nodes — pre-call budget enforcement via the SpendGuardChatModelWrapper.",
  "license": "Apache-2.0",
  "homepage": "https://github.com/m24927605/agentic-spendguard",
  "repository": {
    "type": "git",
    "url": "https://github.com/m24927605/agentic-spendguard.git",
    "directory": "sdk/typescript/integrations/flowise"
  },
  "type": "module",
  "engines": { "node": ">=20.10" },
  "sideEffects": false,
  "files": ["dist/", "README.md", "LICENSE_NOTICES.md", "CHANGELOG.md"],
  "main": "./dist/index.js",
  "types": "./dist/index.d.ts",
  "exports": {
    ".": { "types": "./dist/index.d.ts", "import": "./dist/index.js" }
  },
  "scripts": {
    "build": "tsup",
    "lint": "biome check src tests",
    "typecheck": "tsc --noEmit",
    "test": "vitest run",
    "size": "tsx ../../scripts/verify-size.ts --max 50kb --gz 16kb dist/index.js",
    "prepack": "pnpm run build && pnpm run size"
  },
  "peerDependencies": {
    "@spendguard/sdk": "^0.1.0",
    "@spendguard/langchain": "^0.1.0",
    "flowise-components": ">=2.0.0"
  },
  "devDependencies": {
    "@types/node": "^20.10.0",
    "testcontainers": "^10.10.0",
    "tsup": "^8.0.0",
    "biome": "^1.9.0",
    "vitest": "^2.0.0",
    "tsx": "^4.0.0"
  }
}
```

## 3. `SpendGuardChatModelWrapper` — INode class skeleton

The Flowise `INode` contract requires a class with a `constructor()` populating `this.label / name / type / icon / category / baseClasses / inputs`, plus an `init()` method. The `init()` signature is `init(nodeData: INodeData, _input: string, options: ICommonObject): Promise<unknown>`. The returned value flows out of the canvas anchor into the next node.

```ts
// src/nodes/SpendGuardChatModelWrapper.ts
import type { INode, INodeData, ICommonObject } from "flowise-components";
import type { BaseChatModel } from "@langchain/core/language_models/chat_models";
import { SpendGuardCallbackHandler } from "@spendguard/langchain";
import { getOrCreateClient } from "../clientCache.js";
import { buildClaimEstimator } from "../claimEstimator.js";

export class SpendGuardChatModelWrapper implements INode {
  label = "SpendGuard ChatModel Wrapper";
  name = "spendGuardChatModelWrapper";
  version = 1.0;
  type = "BaseChatModel";
  icon = "spendguard.svg";
  category = "Spend Guard";
  description =
    "Wraps any ChatModel with SpendGuard pre-call budget enforcement. " +
    "Drop this between your ChatModel and your Chain/Agent.";
  baseClasses = ["BaseChatModel", "BaseLanguageModel"];

  inputs = [
    {
      label: "Chat Model",
      name: "chatModel",
      type: "BaseChatModel",
    },
    {
      label: "Tenant ID",
      name: "tenantId",
      type: "string",
    },
    {
      label: "Budget ID",
      name: "budgetId",
      type: "string",
    },
    {
      label: "Budget Window Instance ID",
      name: "windowInstanceId",
      type: "string",
    },
    {
      label: "Unit",
      name: "unit",
      type: "string",
      default: "usd_micros",
    },
    {
      label: "Sidecar UDS Path",
      name: "sidecarUds",
      type: "string",
      optional: true,
      description:
        "Defaults to env SPENDGUARD_SIDECAR_UDS. Path to the SpendGuard sidecar's Unix-domain socket.",
    },
    {
      label: "Route",
      name: "route",
      type: "string",
      default: "llm.call",
      optional: true,
    },
    {
      label: "Claim Estimator (JSON)",
      name: "claimEstimatorJson",
      type: "string",
      rows: 4,
      optional: true,
      description:
        'JSON describing a fixed claim, e.g. {"amountAtomic":"1000000","scopeId":"default"}. ' +
        "Omit for a conservative $1 USD-micros default per call.",
    },
  ];

  async init(
    nodeData: INodeData,
    _input: string,
    options: ICommonObject,
  ): Promise<unknown> {
    const chatModel = nodeData.inputs?.chatModel as BaseChatModel | undefined;
    if (!chatModel) {
      throw new Error("SpendGuardChatModelWrapper: chatModel input is required");
    }

    const tenantId = (nodeData.inputs?.tenantId as string) ?? "";
    const budgetId = (nodeData.inputs?.budgetId as string) ?? "";
    const windowInstanceId =
      (nodeData.inputs?.windowInstanceId as string) ?? "";
    const unit = (nodeData.inputs?.unit as string) ?? "usd_micros";
    const sidecarUds =
      (nodeData.inputs?.sidecarUds as string) ??
      process.env.SPENDGUARD_SIDECAR_UDS ??
      "";
    const route = (nodeData.inputs?.route as string) ?? "llm.call";
    const claimEstimatorJson =
      (nodeData.inputs?.claimEstimatorJson as string) ?? "";

    if (!tenantId || !budgetId || !windowInstanceId || !sidecarUds) {
      throw new Error(
        "SpendGuardChatModelWrapper: tenantId, budgetId, windowInstanceId, " +
          "and sidecarUds (or env SPENDGUARD_SIDECAR_UDS) are all required",
      );
    }

    const client = await getOrCreateClient({ sidecarUds, tenantId });
    const claimEstimator = buildClaimEstimator({
      json: claimEstimatorJson,
      unit,
    });

    const handler = new SpendGuardCallbackHandler({
      client,
      budgetId,
      windowInstanceId,
      unit,
      claimEstimator,
      route,
    });

    // Mutate IN PLACE so downstream chain nodes see the handler too.
    chatModel.callbacks = [...(chatModel.callbacks ?? []), handler];
    return chatModel;
  }
}

// Flowise loads the `nodeClass` export from the package's index.
module.exports = { nodeClass: SpendGuardChatModelWrapper };
```

The `module.exports = { nodeClass }` line is required by Flowise's loader convention even in an ESM build — tsup's `cjs: false, esm: true` config emits both a top-level ESM export AND the `module.exports` shim Flowise reads.

## 4. `clientCache.ts`

Module-level cache so re-running `init()` per invocation does not re-open the UDS connection.

```ts
// src/clientCache.ts
import { SpendGuardClient } from "@spendguard/sdk";

interface CacheKey {
  sidecarUds: string;
  tenantId: string;
}

const cache = new Map<string, SpendGuardClient>();

function keyOf(k: CacheKey): string {
  return `${k.tenantId}::${k.sidecarUds}`;
}

export async function getOrCreateClient(k: CacheKey): Promise<SpendGuardClient> {
  const key = keyOf(k);
  const existing = cache.get(key);
  if (existing) return existing;

  const client = new SpendGuardClient({
    socketPath: k.sidecarUds,
    tenantId: k.tenantId,
    runtimeKind: "flowise",
    runtimeVersion: "0.1.0",
  });
  await client.connect();
  await client.handshake();
  cache.set(key, client);
  return client;
}

// Test-only: clear the cache between test cases.
export function _resetCacheForTests(): void {
  cache.clear();
}
```

## 5. `claimEstimator.ts`

Convert the no-code JSON input into a D04-compatible `ClaimEstimator` function. Default = $1 USD-micros per call when the JSON is empty.

```ts
// src/claimEstimator.ts
import type { ClaimEstimator } from "@spendguard/langchain";

interface BuildArgs {
  json: string;
  unit: string;
}

export function buildClaimEstimator({ json, unit }: BuildArgs): ClaimEstimator {
  if (!json.trim()) {
    // Conservative default — covers the no-code drop-in path.
    return () => [
      {
        scopeId: "default",
        amountAtomic: "1000000",
        unit,
      },
    ];
  }

  let parsed: unknown;
  try {
    parsed = JSON.parse(json);
  } catch (err) {
    throw new Error(
      `SpendGuardChatModelWrapper: claimEstimatorJson is not valid JSON: ${(err as Error).message}`,
    );
  }

  const claim = parsed as {
    amountAtomic?: string;
    scopeId?: string;
    unit?: string;
  };
  if (!claim.amountAtomic) {
    throw new Error(
      "SpendGuardChatModelWrapper: claimEstimatorJson must include 'amountAtomic' as a decimal string",
    );
  }
  return () => [
    {
      scopeId: claim.scopeId ?? "default",
      amountAtomic: claim.amountAtomic!,
      unit: claim.unit ?? unit,
    },
  ];
}
```

## 6. `index.ts` — public re-exports

```ts
// src/index.ts
export { SpendGuardChatModelWrapper } from "./nodes/SpendGuardChatModelWrapper.js";
// Flowise's loader reads `nodeClass` off the default export of each file
// it discovers in the package. The wrapper file itself sets module.exports.
```

## 7. Build (`tsup.config.ts`)

```ts
import { defineConfig } from "tsup";

export default defineConfig({
  entry: ["src/index.ts", "src/nodes/SpendGuardChatModelWrapper.ts"],
  format: ["esm"],
  dts: true,
  splitting: false,
  sourcemap: true,
  clean: true,
  treeshake: true,
  target: "node20",
  loader: {
    ".svg": "base64",
  },
  external: [
    "@spendguard/sdk",
    "@spendguard/langchain",
    "flowise-components",
    "@langchain/core",
  ],
});
```

The SVG icon is base64-embedded so the published tarball ships a single JS file per node — no separate asset path that the Flowise loader has to discover.

## 8. Flowise install paths

Three install paths are supported and documented in the README:

| Path | Command | Use case |
|---|---|---|
| `npm install` into Flowise repo | `cd /path/to/Flowise && pnpm add @spendguard/flowise-nodes` then restart | for users who built Flowise from source |
| `~/.flowise/nodes/` drop-in | `npm pack` → unpack `dist/` to `~/.flowise/nodes/spendguard/` → restart | for the official Flowise Docker image where `npm install` inside the container is hard |
| Custom Docker layer | `Dockerfile` snippet documented in README that extends `flowiseai/flowise:2.x` and `npm install @spendguard/flowise-nodes` | for production Kubernetes installs |

## 9. Integration with `examples/flowise/`

Slice 5 ships `examples/flowise/chatflow.json` — a serialised Flowise chatflow with three nodes:

1. `ChatOpenAI` (model `gpt-4o-mini`, OpenAI API key via env)
2. `SpendGuardChatModelWrapper` wrapping (1), tenant + budget configured for the demo control-plane seed
3. `Conversation Chain` consuming the wrapper output

The runner script `run_flowise_real.ts`:

1. POSTs the chatflow JSON to Flowise's `POST /api/v1/chatflows`.
2. POSTs a prediction to `POST /api/v1/prediction/<id>` with prompt `hi`.
3. Asserts the response body looks like a chat completion AND the sidecar UDS has logged a `RequestDecision` with `trigger=LLM_CALL_PRE`, `route=llm.call`.
4. (Deny variant, when `SPENDGUARD_DEMO_DENY=1`) Asserts the prediction call returns a 4xx body containing `STOP` or the `DecisionStopped` reason code surface.

The demo container layer adds Node 20 + the wrapper package on top of `flowiseai/flowise:2.x`, mirroring D04's `examples/langchain-ts/` pattern.
