# D37 — Implementation

Pair with `design.md` (public surface), `tests.md` (verification), `acceptance.md` (gates). Symbols imported from `@spendguard/sdk` come from D05 §4; symbols imported from `@spendguard/langchain` come from D04 §4. Neither is re-derived here.

## 1. Repo layout

```
sdk/typescript/integrations/n8n/
├── package.json
├── tsconfig.json
├── gulpfile.js                # n8n community-node convention — copies icons + asset bundling
├── .eslintrc.js               # @n8n_io/eslint-config-node — REQUIRED by n8n linter
├── biome.json                 # internal code style; n8n eslint stays separate
├── vitest.config.ts
├── README.md
├── LICENSE_NOTICES.md
├── CHANGELOG.md
├── credentials/
│   └── SpendGuardApi.credentials.ts
├── nodes/
│   └── SpendGuardChatModel/
│       ├── SpendGuardChatModel.node.ts
│       ├── SpendGuardChatModel.node.json     # node codex metadata
│       └── spendguard.svg                    # node icon (16x16 + 60x60 dual)
├── src/
│   ├── clientPool.ts          # singleton SpendGuardClient cache, keyed by credential hash
│   ├── runIdentity.ts         # executionId / nodeName / custom run-id resolution
│   ├── errors.ts              # mapping SpendGuardError → NodeApiError
│   └── version.ts             # auto-generated VERSION
└── tests/
    ├── node.test.ts           # SpendGuardChatModel.supplyData behaviour
    ├── credential.test.ts     # SpendGuardApi schema validation
    ├── clientPool.test.ts     # singleton + eviction semantics
    ├── runIdentity.test.ts    # run-id derivation
    ├── errors.test.ts         # SpendGuardError → NodeApiError mapping
    ├── _support/
    │   ├── mockN8nContext.ts  # ISupplyDataFunctions mock
    │   └── mockUpstreamModel.ts # tiny BaseChatModel fixture
    └── e2e/
        └── selfHostedN8n.test.ts # uses docker-compose self-hosted n8n + mock sidecar
```

Top-level demo files (slice 5):

```
examples/n8n/
├── README.md
├── workflows/
│   └── n8n_real.workflow.json  # importable workflow with AI Agent + SpendGuard wrapper
└── scripts/
    └── trigger_workflow.ts     # invokes the workflow via n8n's REST API
```

## 2. `package.json` (n8n community-node convention)

```json
{
  "name": "n8n-nodes-spendguard",
  "version": "0.1.0",
  "description": "SpendGuard pre-call budget enforcement for n8n AI Agent workflows.",
  "license": "Apache-2.0",
  "homepage": "https://github.com/m24927605/agentic-spendguard",
  "author": { "name": "SpendGuard authors" },
  "repository": {
    "type": "git",
    "url": "https://github.com/m24927605/agentic-spendguard.git",
    "directory": "sdk/typescript/integrations/n8n"
  },
  "type": "commonjs",
  "main": "dist/index.js",
  "engines": { "node": ">=20.10" },
  "keywords": ["n8n-community-node-package", "spendguard", "ai", "langchain", "budget", "fincops"],
  "files": ["dist/"],
  "scripts": {
    "build": "tsc && gulp build:icons",
    "dev": "tsc --watch",
    "lint": "eslint nodes credentials package.json && biome check src tests",
    "lintfix": "eslint nodes credentials package.json --fix && biome check src tests --write",
    "format": "biome format src tests --write",
    "typecheck": "tsc --noEmit",
    "test": "vitest run",
    "prepack": "pnpm run build && pnpm run lint"
  },
  "n8n": {
    "n8nNodesApiVersion": 1,
    "credentials": ["dist/credentials/SpendGuardApi.credentials.js"],
    "nodes": ["dist/nodes/SpendGuardChatModel/SpendGuardChatModel.node.js"]
  },
  "dependencies": {
    "@spendguard/sdk": "0.1.0",
    "@spendguard/langchain": "0.1.0"
  },
  "peerDependencies": {
    "n8n-workflow": "*",
    "@langchain/core": "^0.3.0"
  },
  "devDependencies": {
    "@biomejs/biome": "^1.9.4",
    "@langchain/core": "^0.3.0",
    "@n8n_io/eslint-config-node": "^1.3.0",
    "@types/node": "^20.14.0",
    "eslint": "^8.57.0",
    "eslint-plugin-n8n-nodes-base": "^1.16.0",
    "gulp": "^4.0.2",
    "n8n-workflow": "^1.50.0",
    "typescript": "^5.6.0",
    "vitest": "^2.1.0"
  },
  "publishConfig": { "access": "public", "provenance": true }
}
```

CJS not ESM — n8n's loader is CJS-only as of n8n 1.50; this is the second deviation from D04 (which is ESM). Bundle budget: tarball ≤ 200 KB (n8n nodes ship more metadata than a bare adapter).

`@spendguard/sdk` + `@spendguard/langchain` are **runtime deps** (not peerDeps) because n8n's community-node loader does NOT walk peer-dep manifests of installed nodes — peer deps would 404 at runtime. Versions are exact-pinned (`0.1.0`, not `^0.1.0`) to make the audit-chain invariant version-deterministic for installs.

## 3. `SpendGuardApi.credentials.ts`

```ts
import type { ICredentialType, INodeProperties } from "n8n-workflow";

export class SpendGuardApi implements ICredentialType {
  name = "spendGuardApi";
  displayName = "SpendGuard API";
  documentationUrl = "https://github.com/m24927605/agentic-spendguard/blob/main/docs/site/docs/integrations/n8n.md";
  properties: INodeProperties[] = [
    {
      displayName: "Tenant ID",
      name: "tenantId",
      type: "string",
      default: "",
      required: true,
      description: "SpendGuard tenant identifier (UUID).",
    },
    {
      displayName: "Sidecar UDS Path",
      name: "socketPath",
      type: "string",
      default: "/var/run/spendguard/sidecar.sock",
      description: "Unix domain socket path; required for v0.1.x.",
    },
    {
      displayName: "Budget ID",
      name: "budgetId",
      type: "string",
      default: "",
      required: true,
    },
    {
      displayName: "Window Instance ID",
      name: "windowInstanceId",
      type: "string",
      default: "",
      required: true,
    },
    {
      displayName: "Runtime Kind",
      name: "runtimeKind",
      type: "string",
      default: "n8n",
      description: "Forwarded to SpendGuard for telemetry. Override only if you need to attribute multi-tenant n8n.",
    },
  ];
}
```

No `test` function: handshake happens at first node invocation, not at credential save (UDS may not be reachable from the n8n UI host).

## 4. `SpendGuardChatModel.node.ts` (skeleton)

```ts
import {
  type INodeType,
  type INodeTypeDescription,
  type ISupplyDataFunctions,
  type SupplyData,
  NodeConnectionType,
} from "n8n-workflow";
import type { BaseChatModel } from "@langchain/core/language_models/chat_models";
import { SpendGuardCallbackHandler } from "@spendguard/langchain";

import { resolveRunIdentity } from "../../src/runIdentity";
import { acquireClient } from "../../src/clientPool";
import { mapToNodeApiError } from "../../src/errors";

export class SpendGuardChatModel implements INodeType {
  description: INodeTypeDescription = {
    displayName: "SpendGuard Chat Model",
    name: "spendGuardChatModel",
    icon: "file:spendguard.svg",
    group: ["transform"],
    version: 1,
    description: "Wrap an AI Language Model sub-node with SpendGuard reserve+commit gating.",
    defaults: { name: "SpendGuard Chat Model" },
    codex: {
      categories: ["AI"],
      subcategories: { AI: ["Language Models"] },
      resources: {
        primaryDocumentation: [{ url: "https://github.com/m24927605/agentic-spendguard/blob/main/docs/site/docs/integrations/n8n.md" }],
      },
    },
    inputs: `={{ ((parameters) => { return [{ type: '${NodeConnectionType.AiLanguageModel}', displayName: 'Model' }]; })($parameter) }}`,
    outputs: [{ type: NodeConnectionType.AiLanguageModel, displayName: "Wrapped Model" }],
    credentials: [{ name: "spendGuardApi", required: true }],
    properties: [
      { displayName: "Budget ID Override", name: "budgetIdOverride", type: "string", default: "" },
      { displayName: "Route", name: "route", type: "string", default: "llm.call" },
      {
        displayName: "Run ID Source",
        name: "runIdSource",
        type: "options",
        options: [
          { name: "Execution ID + Node Name", value: "executionId" },
          { name: "Node Name", value: "nodeName" },
          { name: "Custom Expression", value: "custom" },
        ],
        default: "executionId",
      },
      {
        displayName: "Custom Run ID",
        name: "customRunId",
        type: "string",
        default: "",
        displayOptions: { show: { runIdSource: ["custom"] } },
      },
      { displayName: "Claim Amount (USD micros)", name: "claimAmountAtomic", type: "string", default: "1000000" },
      { displayName: "Unit", name: "unit", type: "string", default: "usd_micros" },
    ],
  };

  async supplyData(this: ISupplyDataFunctions, itemIndex: number): Promise<SupplyData> {
    const credentials = await this.getCredentials("spendGuardApi");
    const params = {
      budgetId: (this.getNodeParameter("budgetIdOverride", itemIndex, "") as string) || (credentials.budgetId as string),
      route: this.getNodeParameter("route", itemIndex, "llm.call") as string,
      runIdSource: this.getNodeParameter("runIdSource", itemIndex, "executionId") as
        | "executionId" | "nodeName" | "custom",
      customRunId: this.getNodeParameter("customRunId", itemIndex, "") as string,
      claimAmountAtomic: this.getNodeParameter("claimAmountAtomic", itemIndex, "1000000") as string,
      unit: this.getNodeParameter("unit", itemIndex, "usd_micros") as string,
    };

    const upstream = (await this.getInputConnectionData(
      NodeConnectionType.AiLanguageModel,
      itemIndex,
    )) as BaseChatModel;

    if (!upstream) {
      throw new Error("SpendGuard Chat Model: no upstream ai_languageModel connected.");
    }

    const client = await acquireClient(credentials);
    const identity = resolveRunIdentity({ ctx: this, params, itemIndex });

    try {
      const handler = new SpendGuardCallbackHandler({
        client,
        budgetId: params.budgetId,
        windowInstanceId: credentials.windowInstanceId as string,
        unit: params.unit,
        route: params.route,
        // Run-tree pinning: n8n's executionId is the session boundary.
        sessionIdOverride: identity.sessionId,
        runIdOverride: identity.runId,
        stepId: identity.stepId,
        claimEstimator: () => [
          { scopeId: params.budgetId, amountAtomic: params.claimAmountAtomic, unit: params.unit },
        ],
      });

      // Defensive: avoid double-registration if the workflow reuses the wrapper.
      upstream.callbacks = upstream.callbacks ?? [];
      const arr = Array.isArray(upstream.callbacks) ? upstream.callbacks : [upstream.callbacks];
      if (!arr.includes(handler)) arr.push(handler);
      upstream.callbacks = arr;

      return { response: upstream };
    } catch (err) {
      throw mapToNodeApiError(this.getNode(), err);
    }
  }
}
```

`sessionIdOverride` / `runIdOverride` / `stepId` are tiny additions D04's handler accepts (mirrors Python's `step_id` / `run_id` kwargs). If they're not present on D04 v0.1.0 the slice plan adds them as a compatible additive minor in D04 v0.1.1; cross-package coordination is enforced by acceptance gate A11.1.

## 5. `src/clientPool.ts`

```ts
import { SpendGuardClient } from "@spendguard/sdk";
import { createHash } from "node:crypto";

const POOL = new Map<string, Promise<SpendGuardClient>>();
const MAX = 16;

function key(creds: Record<string, unknown>): string {
  const h = createHash("sha256");
  h.update(String(creds.tenantId ?? ""));
  h.update("|");
  h.update(String(creds.socketPath ?? ""));
  return h.digest("hex").slice(0, 16);
}

export async function acquireClient(creds: Record<string, unknown>): Promise<SpendGuardClient> {
  const k = key(creds);
  let p = POOL.get(k);
  if (!p) {
    if (POOL.size >= MAX) {
      // FIFO eviction; closed clients drain inflight work via D05's asyncDispose.
      const [oldestKey, oldestP] = POOL.entries().next().value!;
      POOL.delete(oldestKey);
      oldestP.then((c) => c.close()).catch(() => {});
    }
    p = (async () => {
      const client = new SpendGuardClient({
        socketPath: String(creds.socketPath),
        tenantId: String(creds.tenantId),
        runtimeKind: String(creds.runtimeKind ?? "n8n"),
      });
      await client.connect();
      await client.handshake();
      return client;
    })();
    POOL.set(k, p);

    p.catch(() => POOL.delete(k));
  }
  return p;
}

process.on("beforeExit", () => {
  for (const [, p] of POOL) p.then((c) => c.close()).catch(() => {});
});
```

## 6. `src/runIdentity.ts`

```ts
import type { ISupplyDataFunctions } from "n8n-workflow";

export interface RunIdentity {
  sessionId: string;
  runId: string;
  stepId: string;
}

export function resolveRunIdentity(args: {
  ctx: ISupplyDataFunctions;
  params: { runIdSource: "executionId" | "nodeName" | "custom"; customRunId: string };
  itemIndex: number;
}): RunIdentity {
  const executionId = args.ctx.getExecutionId();
  const nodeName = args.ctx.getNode().name;

  let runId: string;
  switch (args.params.runIdSource) {
    case "nodeName":
      runId = nodeName;
      break;
    case "custom":
      runId = args.params.customRunId || `${executionId}:${nodeName}`;
      break;
    case "executionId":
    default:
      runId = `${executionId}:${nodeName}`;
      break;
  }

  return { sessionId: executionId, runId, stepId: nodeName };
}
```

## 7. `src/errors.ts`

```ts
import { NodeApiError } from "n8n-workflow";
import type { INode } from "n8n-workflow";
import {
  DecisionDenied,
  DecisionStopped,
  DecisionSkipped,
  ApprovalRequired,
  SidecarUnavailable,
  HandshakeError,
} from "@spendguard/sdk";

export function mapToNodeApiError(node: INode, err: unknown): NodeApiError {
  if (err instanceof DecisionStopped || err instanceof DecisionDenied || err instanceof DecisionSkipped) {
    return new NodeApiError(node, { message: err.message, code: 403 }, {
      message: `SpendGuard denied: ${err.reasonCodes.join(", ") || "decision_denied"}`,
      description: `Decision ID ${err.decisionId}. Audit event: ${err.auditDecisionEventId ?? "(pending)"}.`,
      httpCode: "403",
    });
  }
  if (err instanceof ApprovalRequired) {
    return new NodeApiError(node, { message: err.message, code: 428 }, {
      message: "SpendGuard requires approval before this call can proceed.",
      description: `Approval request ${err.approvalRequestId}. Approve in the SpendGuard console and re-run the workflow.`,
      httpCode: "428",
    });
  }
  if (err instanceof SidecarUnavailable) {
    return new NodeApiError(node, { message: err.message, code: 503 }, {
      message: "SpendGuard sidecar unavailable.",
      httpCode: "503",
    });
  }
  if (err instanceof HandshakeError) {
    return new NodeApiError(node, { message: err.message, code: 502 }, {
      message: "SpendGuard handshake failed.",
      httpCode: "502",
    });
  }
  return new NodeApiError(node, err as Error);
}
```

## 8. Demo workflow (slice 5)

`examples/n8n/workflows/n8n_real.workflow.json` defines a 4-node workflow:

```
[Manual Trigger]
       │ main
       ▼
[AI Agent] ──(ai_languageModel)── [SpendGuard Chat Model] ──(ai_languageModel)── [Anthropic Chat Model]
```

`scripts/trigger_workflow.ts` does:

1. POST `/api/v1/credentials` to create the SpendGuard credential against the local demo sidecar UDS.
2. POST `/api/v1/workflows` to import the workflow JSON.
3. POST `/api/v1/workflows/{id}/activate` then `/workflows/{id}/run`.
4. Poll `/api/v1/executions/{id}` for terminal state.
5. Exit 0 on `success`; non-zero on `error` (deny test asserts the latter with `code: 403`).

`deploy/demo/compose.yml` adds:

```yaml
demo-n8n:
  image: n8nio/n8n:1.50.1
  environment:
    N8N_COMMUNITY_PACKAGES_ENABLED: "true"
    N8N_COMMUNITY_PACKAGES_REGISTRY: "https://registry.npmjs.org"
    SPENDGUARD_SIDECAR_UDS: "/var/run/spendguard/sidecar.sock"
  volumes:
    - "spendguard-sock:/var/run/spendguard:ro"
    - "./demo/n8n/init:/data/init:ro"
  command: >
    /bin/sh -c "n8n npm install n8n-nodes-spendguard@0.1.0 &&
                n8n start"
```

For local-dev (`DEMO_LOCAL_DEV=1`) the install step is replaced by a volume-mounted `npm pack`'d tarball — see slice 5.

## 9. Build pipeline (slice 1)

- `tsc` emits `dist/` (CJS, target `ES2022`, `module: commonjs`, `outDir: dist`).
- `gulpfile.js` copies the SVG icons + node JSON files to `dist/`.
- `eslint-plugin-n8n-nodes-base` is the n8n-community linter — required by `n8n.io/community-nodes` review. Runs in `pnpm run lint`.
- biome handles `src/` + `tests/` (not `nodes/` / `credentials/` — those are eslint-only to satisfy the n8n plugin).
- Two-linter setup is the standard n8n community-node pattern; not litigated.

## 10. Publish pipeline (slice 6)

`.github/workflows/sdk-ts-n8n-publish.yml`:

- Trigger: `push: tags: ["n8n-spendguard-v*"]` + `workflow_dispatch`.
- Jobs: `lint`, `typecheck`, `test`, `build`, `publish`.
- Publish job: `permissions: id-token: write`, `npm publish --provenance --access public`.
- Pre-publish gate: `pnpm pack` produces tarball ≤ 200 KB and contains `dist/`, `package.json`, `LICENSE_NOTICES.md`, `README.md`, `CHANGELOG.md` only.
- Provenance is mandatory — n8n's community-node verification checks npm attestation.

## 11. Version policy

- `@spendguard/sdk` and `@spendguard/langchain` deps are exact-pinned (no caret). A SpendGuard minor bump forces a coordinated D37 release. This keeps the audit-chain idempotency-key derivation byte-identical for any given `n8n-nodes-spendguard@X.Y.Z` install.
- n8n bump policy: `n8n-workflow@^1.50.0` peer; tested against `1.50` (floor), `1.55` (mid-range), and latest at each release.
- Node engine: `>=20.10` matches D05 D04 floor.
