# D32 — Implementation

**Reads:** [`design.md`](design.md), [`acceptance.md`](acceptance.md), [`review-standards.md`](review-standards.md).
**Touches:** New integration tree under `integrations/botpress/` + demo orchestration + public docs. No Rust changes (HTTP companion reused from D09 SLICE 1). No proto changes. No DB schema changes. No edits to `sdk/typescript/`.

## 1. Module layout

```
integrations/botpress/                              # NEW — Botpress integration package
├── package.json                                    # name=@spendguard/botpress-integration, ESM, peer-deps
├── tsconfig.json                                   # extends D05 base; outDir=dist
├── tsup.config.ts                                  # bundles ESM + .d.ts; externals @spendguard/sdk + @botpress/sdk
├── biome.json                                      # extends D05 base
├── vitest.config.ts                                # vitest with msw + @botpress/sdk shims
├── integration.definition.ts                       # Botpress IntegrationDefinition (Slice 1-2)
├── botpress.integration.yaml                       # Botpress integration manifest (Slice 1)
├── src/
│   ├── index.ts                                    # Integration entrypoint; default-exports `integration` (Slice 2)
│   ├── config.ts                                   # Zod schema for integration config (Slice 2)
│   ├── reservation.ts                              # SpendGuardReservation delegate (Slices 2-3)
│   ├── hooks/
│   │   ├── beforeAiGeneration.ts                   # Slice 3
│   │   └── afterAiGeneration.ts                    # Slice 3
│   ├── lifecycle/
│   │   ├── validateConfiguration.ts                # Slice 2 — 1-token reserve+release roundtrip
│   │   └── register.ts                             # Slice 2 — register integration with Botpress
│   ├── adapter/
│   │   ├── usage.ts                                # Botpress event → SpendGuard real-usage adapter (Slice 3)
│   │   ├── binding.ts                              # config + conv ctx → BudgetBinding (Slice 3)
│   │   └── errors.ts                               # SpendGuard error → Botpress RuntimeError translation (Slice 3)
│   └── version.ts                                  # exported VERSION constant
├── tests/
│   ├── reservation.test.ts                         # unit (Slice 3)
│   ├── beforeAiGeneration.test.ts                  # unit (Slice 3)
│   ├── afterAiGeneration.test.ts                   # unit (Slice 3)
│   ├── adapter.test.ts                             # unit (Slice 3)
│   ├── lifecycle.test.ts                           # unit — validateConfiguration roundtrip (Slice 3)
│   ├── integration-v12.test.ts                     # boots real Botpress v12 container (Slice 4)
│   ├── _mockSidecar.ts                             # msw HTTP server simulating D09 companion (Slice 3)
│   └── _fixtures.ts                                # shared fixtures
├── README.md                                       # operator-facing (Slice 1)
└── CHANGELOG.md                                    # standard package CHANGELOG (Slice 5)

deploy/demo/
├── Makefile                                        # +DEMO_MODE=botpress_real branch (Slice 5)
├── compose.yaml                                    # untouched
├── botpress/                                       # NEW
│   ├── compose.botpress.yaml                       # Botpress v12 + integration mount overlay
│   ├── bot/
│   │   └── sample_bot.json                         # seed bot definition + model config
│   ├── seed.sh                                     # POSTs the bot + integration config to Botpress admin API
│   └── README.md
├── verify_step_botpress.sql                        # NEW — SQL gate (Slice 5)
└── demo/run_demo.py                                # +run_botpress_real_mode (Slice 5)

docs/site/docs/integrations/
└── botpress.md                                     # NEW — public docs page (Slice 5)

.github/workflows/
└── botpress-integration-publish.yml                # NEW — npm OIDC publish on tag `botpress-integration-v*` (Slice 5)
```

## 2. Slice breakdown

### Slice 1 — Integration scaffold (S)

**Files:** `integrations/botpress/package.json`, `tsconfig.json`, `tsup.config.ts`, `biome.json`, `vitest.config.ts`, `README.md`, `botpress.integration.yaml`, `src/version.ts`.

Runs `botpress integrations init --name spendguard --template empty` against a vendored Botpress CLI binary or `npx @botpress/cli init` (no network needed beyond initial cli install). Commits the verbatim scaffold tree, then patches `botpress.integration.yaml` `name` / `version` / `description` / `icon` / `readme`. `package.json` declares:

```json
{
  "name": "@spendguard/botpress-integration",
  "version": "0.1.0",
  "type": "module",
  "main": "./dist/index.js",
  "types": "./dist/index.d.ts",
  "scripts": {
    "build": "tsup",
    "test": "vitest run",
    "test:integration": "vitest run -c vitest.integration.config.ts",
    "lint": "biome check src tests",
    "bp:push": "botpress integrations push"
  },
  "peerDependencies": {
    "@spendguard/sdk": "^0.1.0",
    "@botpress/sdk": "^0.7.0"
  },
  "engines": { "node": ">=20.10.0" }
}
```

Acceptance: `pnpm -F @spendguard/botpress-integration build` exits 0; produces `dist/index.js` (≥ 1 KB stub).

### Slice 2 — Hooks registration skeleton + config + validateConfiguration (M)

**Files:** `src/index.ts`, `src/config.ts`, `src/reservation.ts` (skeleton), `src/lifecycle/validateConfiguration.ts`, `src/lifecycle/register.ts`, `src/hooks/beforeAiGeneration.ts` (stub), `src/hooks/afterAiGeneration.ts` (stub), `tests/reservation.test.ts` (skeleton tests), `tests/lifecycle.test.ts`.

```ts
// src/config.ts
import { z } from "@botpress/sdk";

export const ConfigurationSchema = z.object({
  sidecarUrl: z.string().url().describe("HTTP companion URL (loopback or sidecar-pod port)"),
  spendguardBudgetId: z.string().min(1),
  spendguardWindowInstanceId: z.string().min(1),
  upstreamProvider: z.enum(["openai", "anthropic", "bedrock"]),
  tenantId: z.string().min(1),
  tlsCertPath: z.string().optional().describe("Path to SVID cert PEM"),
  tlsKeyPath: z.string().optional(),
  tlsRootCaPath: z.string().optional(),
});

export type Configuration = z.infer<typeof ConfigurationSchema>;
```

```ts
// src/index.ts
import { Integration } from "@botpress/sdk";
import { ConfigurationSchema } from "./config.js";
import { beforeAiGeneration } from "./hooks/beforeAiGeneration.js";
import { afterAiGeneration } from "./hooks/afterAiGeneration.js";
import { validateConfiguration } from "./lifecycle/validateConfiguration.js";

export default new Integration({
  configuration: { schema: ConfigurationSchema },
  register: validateConfiguration,
  unregister: async () => {},
  channels: {},
  actions: {},
  hooks: {
    beforeAiGeneration,
    afterAiGeneration,
  },
});
```

```ts
// src/reservation.ts (skeleton)
/**
 * Reservation/commit/release delegate for the Botpress integration.
 *
 * Mirrors sdk/python/src/spendguard/integrations/litellm.py SpendGuardLiteLLMCallback
 * and integrations/dify/models/llm/_reservation.py _DifyReservation. Composition over
 * inheritance: the Botpress hook signature and the SpendGuard reservation lifecycle are
 * orthogonal state machines.
 */
import {
  SpendGuardClient,
  deriveIdempotencyKey,
  computePromptHash,
  type DecisionOutcome,
} from "@spendguard/sdk";
import type { Configuration } from "./config.js";

export interface BotpressCallContext {
  readonly botId: string;
  readonly conversationId: string;
  readonly userId: string;
  readonly model: string;
  readonly messages: ReadonlyArray<{ role: string; content: string }>;
  readonly maxTokens: number;
}

export interface ReservationHandle {
  readonly decisionId: string;
  readonly reservationId: string;
  readonly llmCallId: string;
  readonly runId: string;
  readonly stepId: string;
  readonly estimatorSnapshot: Readonly<Record<string, unknown>>;
}

export class SpendGuardReservation {
  private client: SpendGuardClient | null = null;
  private readonly failOpenDev =
    (process.env.SPENDGUARD_BOTPRESS_FAIL_OPEN ?? "").trim() === "1";

  constructor(private readonly config: Configuration) {}

  async reserve(ctx: BotpressCallContext): Promise<ReservationHandle> { /* Slice 3 */ }
  async commitSuccess(
    handle: ReservationHandle,
    realUsage: { inputTokens: number; outputTokens: number },
    providerEventId: string,
  ): Promise<void> { /* Slice 3 */ }
  async releaseFailure(handle: ReservationHandle, exc: unknown): Promise<void> { /* Slice 3 */ }
}
```

`validateConfiguration` issues a 1-token reserve+release roundtrip via the same `SpendGuardReservation.reserve` + `releaseFailure` codepath, proving sidecar wiring at integration install time (Slice 2 acceptance gate; INV-4).

### Slice 3 — Reserve/commit wiring (M)

**Files:** `src/reservation.ts` (full impl), `src/adapter/binding.ts`, `src/adapter/usage.ts`, `src/adapter/errors.ts`, `src/hooks/beforeAiGeneration.ts` (full impl), `src/hooks/afterAiGeneration.ts` (full impl), `tests/reservation.test.ts`, `tests/beforeAiGeneration.test.ts`, `tests/afterAiGeneration.test.ts`, `tests/adapter.test.ts`, `tests/_mockSidecar.ts`.

```ts
// src/hooks/beforeAiGeneration.ts
import type { Integration } from "@botpress/sdk";
import { RuntimeError } from "@botpress/sdk";
import { SpendGuardReservation } from "../reservation.js";
import { toBindingFromHookCtx } from "../adapter/binding.js";
import { toRuntimeError } from "../adapter/errors.js";

export const beforeAiGeneration: NonNullable<
  Parameters<typeof Integration>[0]["hooks"]
>["beforeAiGeneration"] = async ({ ctx, client, data, configuration }) => {
  const reservation = new SpendGuardReservation(configuration);
  const callCtx = toBindingFromHookCtx({ ctx, data, client });
  try {
    const handle = await reservation.reserve(callCtx);
    // Stash the handle on data.context so afterAiGeneration can locate it
    data._spendguardHandle = handle;
    return { data };
  } catch (e) {
    throw toRuntimeError(e);
  }
};
```

```ts
// src/hooks/afterAiGeneration.ts
export const afterAiGeneration = async ({ ctx, client, data, configuration }) => {
  const handle = data._spendguardHandle;
  if (!handle) return { data }; // before-hook never ran (e.g. another integration short-circuited)
  const reservation = new SpendGuardReservation(configuration);
  const realUsage = extractUsageFromBotpressEvent(data);
  try {
    if (realUsage) {
      await reservation.commitSuccess(handle, realUsage, data.providerEventId ?? "");
    } else {
      // Estimator-snapshot fallback + WARN (mirrors D10 INV-5 secondary path)
      logger.warn("spendguard:botpress: falling back to estimator (no usage on event)");
      await reservation.commitSuccess(handle, snapshotToUsage(handle.estimatorSnapshot), "");
    }
  } catch (e) {
    await reservation.releaseFailure(handle, e);
    throw toRuntimeError(e);
  }
  return { data };
};
```

`src/adapter/usage.ts` translates Botpress's normalised `event.payload.usage` (`{ inputTokens, outputTokens }`) for OpenAI + Anthropic shape; Bedrock-via-Botpress emits the same normalised shape per `@botpress/sdk` 0.7.

`src/adapter/binding.ts` builds `BudgetBinding` from `configuration` + `ctx.botId` + `data.conversationId` + Botpress message list; computes prompt hash via D05 `computePromptHash`.

`src/adapter/errors.ts` maps `DecisionDenied` / `SidecarUnavailable` / `SpendGuardConfigError` → Botpress `RuntimeError` with `code: "BUDGET_DENIED" | "BUDGET_DEGRADED" | "BUDGET_CONFIG"` so the Botpress runtime can short-circuit and surface a deterministic error to the conversation.

`SpendGuardReservation.reserve` follows the D05 client pattern: lazy-init HTTP+mTLS client (`SpendGuardClient` over the D09 companion endpoint), 5s deadline, 1s per-attempt timeout, deadline-bounded retries. Idempotency key via `deriveIdempotencyKey({tenantId, sessionId: ctx.conversationId, runId, stepId, llmCallId, trigger: "LLM_CALL_PRE"})`.

`commitSuccess` posts to `/v1/trace` with `LLM_CALL_POST.SUCCESS` + `estimatedAmountAtomic=String(inputTokens+outputTokens)` + `providerReportedAmountAtomic=""` (matches existing TS adapter contract).

`releaseFailure` swallows release-RPC errors but logs WARN (TTL sweep backstop), classifies `AbortError` / `CancelledError` → CANCELLED via the same regex as `_classify_failure` in `litellm.py:735-760`.

### Slice 4 — Tests against self-hosted Botpress v12 (M)

**Files:** `tests/integration-v12.test.ts`, `vitest.integration.config.ts`, `tests/_fixtures.ts` (extended), `.github/workflows/botpress-integration-ci.yml`.

Vitest integration suite boots a self-hosted Botpress v12 container (`botpress/server:v12.30.x` pinned by digest) inside testcontainers-node, mounts the built `dist/` of `@spendguard/botpress-integration`, seeds a minimal bot via Botpress admin API, then triggers conversation events that fire `beforeAiGeneration` / `afterAiGeneration` hooks.

```ts
// tests/integration-v12.test.ts
import { GenericContainer, Wait } from "testcontainers";
import { setupMockSidecar } from "./_mockSidecar.js";

const BOTPRESS_IMAGE = "botpress/server:v12.30.x@sha256:..."; // pinned digest

let botpress: StartedTestContainer;
let mockSidecar: MockSidecarHandle;

beforeAll(async () => {
  mockSidecar = await setupMockSidecar(); // msw on a known port
  botpress = await new GenericContainer(BOTPRESS_IMAGE)
    .withExposedPorts(3000)
    .withBindMounts([{
      source: path.resolve("./dist"),
      target: "/botpress/integrations/spendguard",
      mode: "ro",
    }])
    .withEnvironment({
      SPENDGUARD_SIDECAR_URL: mockSidecar.url,
      SPENDGUARD_TENANT_ID: "test-tenant",
    })
    .withWaitStrategy(Wait.forLogMessage(/Botpress is listening/))
    .start();
  // POST bot + integration config via admin API
  await seedBot(botpress.getMappedPort(3000));
});

afterAll(async () => {
  await botpress?.stop();
  await mockSidecar?.close();
});

test("I01 hook fires reserve before model call", async () => { /* ... */ });
test("I02 deny short-circuits the generation", async () => { /* ... */ });
test("I03 success commits real usage", async () => { /* ... */ });
test("I04 validateConfiguration emits sidecar probe at install", async () => { /* ... */ });
```

Botpress image is ~800 MB; CI uses GH Actions cache for the docker layer so subsequent runs skip the pull.

### Slice 5 — Demo mode + docs + publish job (M)

**Files:** `deploy/demo/Makefile`, `deploy/demo/botpress/compose.botpress.yaml`, `deploy/demo/botpress/bot/sample_bot.json`, `deploy/demo/botpress/seed.sh`, `deploy/demo/verify_step_botpress.sql`, `deploy/demo/demo/run_demo.py`, `docs/site/docs/integrations/botpress.md`, `.github/workflows/botpress-integration-publish.yml`, `README.md`, `integrations/botpress/CHANGELOG.md`.

Makefile branch:

```
else ifeq ($(DEMO_MODE),botpress_real)
	@echo "[demo] DEMO_MODE=botpress_real → Botpress v12 + integration + sidecar"
	$(COMPOSE) -f compose.yaml -f botpress/compose.botpress.yaml up -d --build \
	    postgres pki-init bundles-init canonical-seed-init manifest-init \
	    endpoint-catalog ledger canonical-ingest tokenizer sidecar \
	    botpress-server botpress-seed
```

`compose.botpress.yaml` mounts `botpress/server:v12.30.x` pinned by digest, mounts `integrations/botpress/dist/` read-only at `/botpress/integrations/spendguard`, mounts the sidecar HTTP companion CA into Botpress's trust store, and points `SPENDGUARD_SIDECAR_URL` at the sidecar's loopback companion port.

`bot/sample_bot.json` defines a simple OpenAI-gpt-4o-mini bot with the SpendGuard integration enabled and a starter intent.

`seed.sh` POSTs the bot definition and integration configuration to Botpress's admin API and sends three probe conversation messages.

Demo driver `run_botpress_real_mode` (~150 LOC):
1. POST a conversation message to Botpress admin API that triggers the bot's AI generation. Assert HTTP 200, sidecar audit row reserved + committed.
2. POST a message that, with `force_hard_cap=1` in `decision_context`, triggers DENY. Assert 4xx from Botpress, DENY decision audited, **no upstream HTTP** (verified via the counting stub in front of `api.openai.com`).
3. POST a streaming-mode message. Assert success, end-of-stream commit row.

Docs page covers: "Why SpendGuard for Botpress", "Install (self-hosted v12)", "Install (Botpress Cloud — marketplace pending)", decision matrix vs egress-proxy, limitations (no workflow-node gating beyond AI hook), and the install-time `validateConfiguration` probe.

`botpress-integration-publish.yml` workflow runs on `botpress-integration-v*` tags; runs `pnpm build`, `pnpm test`, `pnpm publish --provenance` via npm OIDC Trusted Publisher (mirrors D05 / D04 pattern). Marketplace push to Botpress Cloud is documented as a future follow-up (the sideload path is the v1 invariant).

README gains one row: `Botpress (v12 self-host + Cloud) | TypeScript integration | npm i @spendguard/botpress-integration && botpress integrations push`.

## 3. Backwards compatibility

| Surface | Action |
|---------|--------|
| Existing `examples/` / `sdk/python/` / `sdk/typescript/` | Untouched. |
| `compose.yaml` for other demo modes | Unchanged. Botpress services live in an overlay file, opt-in per DEMO_MODE branch. |
| Existing PyPI extras of `spendguard-sdk` | N/A — D32 is TS, not Python. |
| Existing npm `@spendguard/sdk` | Unchanged. D32 is a peer-dep consumer only. |
| Existing DB schemas | Unchanged. Botpress uses its own embedded SQLite by default in v12 self-host; demo mode keeps that to avoid Postgres collision. |
| `@spendguard/sdk` v0.1.0 surface | Unchanged. D32 reads only the locked §4 surface from D05. |

## 4. Failure modes (must be tested)

| Mode | Expected | Test |
|------|----------|------|
| `@botpress/sdk` < 0.7 | Build fails at peer-dep check; integration entry module re-throws clear error if loaded | `tests/integration-v12.test.ts::test_peer_dep_floor` |
| `SPENDGUARD_SIDECAR_URL` unset + config missing `sidecarUrl` | Zod validation error at integration register | `tests/lifecycle.test.ts::test_missing_url` |
| Sidecar DENY | Botpress `RuntimeError` (`code: BUDGET_DENIED`); **no upstream HTTP** | `tests/beforeAiGeneration.test.ts::test_deny_no_upstream` + integration suite I02 |
| Sidecar DEGRADE | Botpress `RuntimeError` (`code: BUDGET_DEGRADED`) | `tests/reservation.test.ts::test_degrade_fail_closed` |
| `SPENDGUARD_BOTPRESS_FAIL_OPEN=1` + DEGRADE | beforeAi hook returns ALLOW + WARN + no commit row | `tests/reservation.test.ts::test_fail_open_skips_commit` |
| `event.payload.usage` missing on afterAi | Estimator-snapshot commit + WARN | `tests/afterAiGeneration.test.ts::test_no_usage_estimator_fallback` |
| `validateConfiguration` with bad sidecar | Botpress integration register fails with named error | `tests/lifecycle.test.ts::test_validate_bad_sidecar` |
| Conversation cancelled mid-generation | `releaseFailure` fires with `outcome=CANCELLED` | `tests/afterAiGeneration.test.ts::test_cancel_releases` |
| Hook re-entrancy (two messages same conversation) | Distinct reservations, no shared `_spendguardHandle` | `tests/beforeAiGeneration.test.ts::test_reentrant_safety` |

## 5. Code skeleton — total LOC budget

| File | Impl LOC | Test LOC |
|------|----------|----------|
| `reservation.ts` | ~260 | covered |
| `hooks/beforeAiGeneration.ts` | ~80 | covered |
| `hooks/afterAiGeneration.ts` | ~120 | covered |
| `config.ts` | ~50 | — |
| `lifecycle/validateConfiguration.ts` | ~60 | covered |
| `lifecycle/register.ts` | ~30 | — |
| `adapter/binding.ts` | ~90 | covered |
| `adapter/usage.ts` | ~50 | covered |
| `adapter/errors.ts` | ~40 | covered |
| `index.ts` | ~30 | — |
| `tests/*.test.ts` (unit) | — | ~520 |
| `tests/integration-v12.test.ts` | — | ~200 |
| `_mockSidecar.ts` + `_fixtures.ts` | — | ~120 |
| Demo driver + verify SQL + seed | ~150 + ~80 + ~50 | — |
| **Total** | **~960 + 280 demo** | **~840** |

## 6. Out of scope

Everything in `design.md` §3. Plus: no changes to `sdk/typescript/` or `sdk/python/`. Plus: no proto changes. Plus: no control-plane API changes. Plus: no edits to D09 SLICE 1 HTTP companion contract (D32 reuses, does not extend). Plus: no Botpress channel plugins. Plus: no per-workflow-node gating beyond the AI generation hook.
