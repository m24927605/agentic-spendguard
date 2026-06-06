# D05 — Implementation

This document specifies the directory layout, file responsibilities, and code skeleton for `@spendguard/sdk`. Pair with `design.md` (public surface contract) and `tests.md` (verification).

## 1. Repo layout

```
sdk/typescript/
├── package.json
├── tsconfig.json
├── tsconfig.build.json          # used by tsup; declaration files only
├── tsup.config.ts
├── biome.json
├── vitest.config.ts
├── pnpm-lock.yaml
├── README.md
├── LICENSE_NOTICES.md
├── CHANGELOG.md
├── Makefile                     # parity with sdk/python/Makefile (proto / lint / test / build)
├── scripts/
│   ├── proto.ts                 # invoked by `pnpm run proto`
│   └── verify-size.ts           # invoked by `pnpm run size`
├── src/
│   ├── index.ts                 # re-exports the entire public surface
│   ├── client.ts                # SpendGuardClient
│   ├── errors.ts                # typed exception hierarchy
│   ├── ids.ts                   # uuid7, deriveIdempotencyKey, ...
│   ├── promptHash.ts            # HMAC-SHA256 tenant-keyed
│   ├── pricing.ts               # PricingLookup
│   ├── pricing/
│   │   └── demo.ts              # embedded snapshot of deploy/demo/init/pricing/seed.yaml
│   ├── runPlan.ts               # withRunPlan + currentRunPlan
│   ├── otel.ts                  # optional OTel emitter
│   ├── retry.ts                 # bounded retry helper
│   ├── decisionCache.ts         # in-process idempotency cache (LRU)
│   ├── env.ts                   # env-var resolution + helpers
│   ├── version.ts               # VERSION constant; generated from package.json at build time
│   └── _proto/
│       └── spendguard/
│           ├── common/v1/common.ts
│           ├── sidecar_adapter/v1/adapter.ts
│           ├── ledger/v1/ledger.ts
│           ├── output_predictor_plugin/v1/plugin.ts
│           └── tokenizer/v1/tokenizer.ts
├── tests/
│   ├── client.test.ts
│   ├── errors.test.ts
│   ├── ids.test.ts
│   ├── promptHash.test.ts
│   ├── pricing.test.ts
│   ├── runPlan.test.ts
│   ├── retry.test.ts
│   ├── decisionCache.test.ts
│   ├── crossLanguage.test.ts          # consumes sdk/fixtures/cross-language/
│   ├── treeShaking.test.ts
│   ├── _support/
│   │   ├── mockSidecar.ts             # @grpc/grpc-js server that runs over an ephemeral UDS
│   │   └── fixtures.ts
│   └── e2e/
│       └── reserveCommitRelease.test.ts
└── .github/                            # symlinked / mirrored into top-level .github via workflows
```

Top-level workflow lives at `/.github/workflows/sdk-ts-publish.yml` (not inside `sdk/typescript/`) per the repo convention.

## 2. `package.json` skeleton

```json
{
  "name": "@spendguard/sdk",
  "version": "0.1.0",
  "description": "SpendGuard SDK — runtime safety layer client for AI agent frameworks (TypeScript half; mirror of spendguard-sdk on PyPI).",
  "license": "Apache-2.0",
  "author": "Michael Chen <m24927605@gmail.com>",
  "homepage": "https://github.com/m24927605/agentic-spendguard",
  "repository": {
    "type": "git",
    "url": "https://github.com/m24927605/agentic-spendguard.git",
    "directory": "sdk/typescript"
  },
  "bugs": "https://github.com/m24927605/agentic-spendguard/issues",
  "keywords": ["llm", "agent", "spend", "budget", "spendguard", "asp", "guardrails"],
  "type": "module",
  "engines": { "node": ">=20.10" },
  "sideEffects": false,
  "files": ["dist/", "README.md", "LICENSE_NOTICES.md", "CHANGELOG.md"],
  "main": "./dist/index.js",
  "types": "./dist/index.d.ts",
  "exports": {
    ".": { "types": "./dist/index.d.ts", "import": "./dist/index.js" },
    "./client": { "types": "./dist/client.d.ts", "import": "./dist/client.js" },
    "./errors": { "types": "./dist/errors.d.ts", "import": "./dist/errors.js" },
    "./ids": { "types": "./dist/ids.d.ts", "import": "./dist/ids.js" },
    "./pricing": { "types": "./dist/pricing.d.ts", "import": "./dist/pricing.js" },
    "./pricing/demo": { "types": "./dist/pricing/demo.d.ts", "import": "./dist/pricing/demo.js" },
    "./prompt-hash": { "types": "./dist/promptHash.d.ts", "import": "./dist/promptHash.js" },
    "./run-plan": { "types": "./dist/runPlan.d.ts", "import": "./dist/runPlan.js" },
    "./proto": { "types": "./dist/_proto/index.d.ts", "import": "./dist/_proto/index.js" }
  },
  "scripts": {
    "proto": "tsx scripts/proto.ts",
    "build": "tsup",
    "dev": "tsup --watch",
    "lint": "biome check src tests scripts",
    "format": "biome format --write src tests scripts",
    "typecheck": "tsc --noEmit",
    "test": "vitest run",
    "test:watch": "vitest",
    "test:e2e": "vitest run tests/e2e",
    "size": "tsx scripts/verify-size.ts",
    "prepack": "pnpm run proto && pnpm run build && pnpm run size"
  },
  "dependencies": {
    "@grpc/grpc-js": "^1.11.0",
    "@protobuf-ts/runtime": "^2.9.4",
    "@protobuf-ts/runtime-rpc": "^2.9.4",
    "@protobuf-ts/grpc-transport": "^2.9.4"
  },
  "peerDependencies": {
    "@opentelemetry/api": "^1.9.0"
  },
  "peerDependenciesMeta": {
    "@opentelemetry/api": { "optional": true }
  },
  "devDependencies": {
    "@biomejs/biome": "^1.9.4",
    "@protobuf-ts/plugin": "^2.9.4",
    "@types/node": "^20.14.0",
    "tsup": "^8.3.0",
    "tsx": "^4.19.0",
    "typescript": "^5.6.0",
    "vitest": "^2.1.0",
    "yaml": "^2.5.0"
  },
  "publishConfig": {
    "access": "public",
    "provenance": true
  }
}
```

## 3. Type declarations (locked from `design.md` §4)

### 3.1 `src/_proto/index.ts` (hand-written barrel)

```ts
export * as common from "./spendguard/common/v1/common.js";
export * as adapter from "./spendguard/sidecar_adapter/v1/adapter.js";
export * as ledger from "./spendguard/ledger/v1/ledger.js";
export * as tokenizer from "./spendguard/tokenizer/v1/tokenizer.js";
```

### 3.2 `src/client.ts` — type-only public surface

```ts
import { credentials, ChannelCredentials, Metadata } from "@grpc/grpc-js";
import { GrpcTransport } from "@protobuf-ts/grpc-transport";
import { adapter as A, common as C } from "./_proto/index.js";
import {
  SpendGuardError,
  HandshakeError,
  SidecarUnavailable,
  DecisionDenied,
  DecisionStopped,
  DecisionSkipped,
  ApprovalRequired,
  ApprovalDeniedError,
  ApprovalLapsedError,
  ApprovalBundleHotReloadedError,
  SpendGuardConfigError,
} from "./errors.js";
import { currentRunPlan } from "./runPlan.js";
import { computePromptHash } from "./promptHash.js";
import { DecisionCache } from "./decisionCache.js";
import { resolveEnvConfig } from "./env.js";
import { wrapSpan } from "./otel.js";
import { runWithRetry } from "./retry.js";
import { VERSION } from "./version.js";

export const DEFAULT_DECISION_TIMEOUT_MS = 250;
export const DEFAULT_HANDSHAKE_TIMEOUT_MS = 2000;
export const DEFAULT_PUBLISH_TIMEOUT_MS = 150;
export const DEFAULT_TRACE_TIMEOUT_MS = 500;

export interface SpendGuardClientOptions {
  socketPath?: string;
  tenantId?: string;
  runtimeKind?: string;
  runtimeVersion?: string;
  sdkVersion?: string;
  protocolVersion?: number;
  capabilityLevel?: number;
  workloadInstanceId?: string;
  decisionTimeoutMs?: number;
  handshakeTimeoutMs?: number;
  publishTimeoutMs?: number;
  traceTimeoutMs?: number;
  onSpan?: (span: SpanRecord) => void;
  otelTracer?: import("@opentelemetry/api").Tracer;
  runtime?: "uds-grpc"; // forward-reserved: | "fetch"
  disabled?: boolean;   // overrides SPENDGUARD_DISABLE
}

export interface HandshakeOutcome {
  sessionId: string;
  sidecarVersion: string;
  schemaBundleId: string;
  schemaBundleHash: Uint8Array;
  contractBundleId: string;
  contractBundleHash: Uint8Array;
  capabilityRequired: number;
  signingKeyId: string;
  announcementSignature: Uint8Array;
}

export interface DecisionOutcome {
  decisionId: string;
  auditDecisionEventId: string;
  decision: "CONTINUE" | "DEGRADE";
  mutationPatchJson: string;
  effectHash: Uint8Array;
  ledgerTransactionId: string;
  reservationIds: readonly string[];
  ttlExpiresAtSeconds: number;
  reasonCodes: readonly string[];
  matchedRuleIds: readonly string[];
}

export interface ReleaseOutcome {
  auditEventSignature: Uint8Array;
  ledgerTransactionId: string;
  releasedReservationIds: readonly string[];
}

export interface BudgetClaim {
  scopeId: string;
  amountAtomic: string; // NUMERIC(38,0) decimal string
  unit: UnitRef;
}

export interface UnitRef {
  unit: string;     // "USD_MICROS" / "TOKENS" / ...
  denomination: number;
}

export interface ClaimEstimate {
  tokenizerTier?: "T1" | "T2" | "T3" | "";
  tokenizerVersionId?: string;
  inputTokens?: number | bigint;
  predictedATokens?: number | bigint;
  predictedBTokens?: number | bigint;
  predictedCTokens?: number | bigint;
  reservedStrategy?: "A" | "B" | "C" | "";
  predictionStrategyUsed?: "A" | "B" | "C" | "";
  predictionPolicyUsed?: string;
  predictionConfidence?: number;
  predictionSampleSize?: number | bigint;
  coldStartLayerUsed?: "L1" | "L2" | "L3" | "L4" | "";
  classifierVersion?: string;
  fingerprintVersion?: string;
  promptClassFingerprint?: string;
  runProjectionAtDecisionAtomic?: number | bigint;
  runPredictedRemainingSteps?: number;
  runStepsCompletedSoFar?: number | bigint;
  runCodeTriggered?: string;
  model?: string;
  promptClass?: string;
}

export interface ReserveRequest {
  trigger: "RUN_PRE" | "AGENT_STEP_PRE" | "LLM_CALL_PRE" | "TOOL_CALL_PRE";
  runId: string;
  stepId: string;
  llmCallId: string;
  toolCallId?: string;
  decisionId: string;
  route: string;
  projectedClaims: BudgetClaim[];
  idempotencyKey: string;
  traceparent?: string;
  tracestate?: string;
  parentRunId?: string;
  budgetGrantJti?: string;
  projectedP50Atomic?: string;
  projectedP90Atomic?: string;
  projectedP95Atomic?: string;
  projectedP99Atomic?: string;
  projectedUnit?: UnitRef;
  promptText?: string;
  decisionContextJson?: Record<string, unknown>;
  claimEstimate?: ClaimEstimate;
}

export interface CommitEstimatedRequest {
  runId: string;
  stepId: string;
  llmCallId: string;
  decisionId: string;
  reservationId: string;
  estimatedAmountAtomic: string;
  unit: UnitRef;
  pricing: PricingFreeze;
  providerEventId: string;
  outcome: "SUCCESS" | "PROVIDER_ERROR" | "CLIENT_TIMEOUT" | "RUN_ABORTED";
  actualInputTokens?: number;
  actualOutputTokens?: number;
  deltaBRatio?: number;
  deltaCRatio?: number;
  traceparent?: string;
  tracestate?: string;
  providerResponseMetadata?: string;
}

export interface ReleaseRequest {
  reservationId: string;
  idempotencyKey: string;
  reasonCodes?: readonly string[];
  workloadInstanceId?: string;
  tenantId?: string;
}

export interface QueryBudgetRequest {
  scopeId: string;
  asOfSeconds?: number;
}

export interface QueryBudgetResult {
  availableAtomic: string;
  reservedAtomic: string;
  committedAtomic: string;
  unit: UnitRef;
  asOfSeconds: number;
}

export interface PricingFreeze {
  pricingVersion: string;
  pricingHash: Uint8Array;
}

export interface PublishOutcomeRequest {
  decisionId: string;
  effectHash: Uint8Array;
  outcome:
    | "APPLIED"
    | "APPLIED_NOOP"
    | "APPLY_FAILED"
    | "APPROVAL_GRANTED"
    | "APPROVAL_DENIED"
    | "APPROVAL_TIMED_OUT";
  adapterError?: string;
}

export interface ApplyFailedRequest {
  decisionId: string;
  effectHash: Uint8Array;
  adapterError: string;
}

export interface ResumeAfterApprovalRequest {
  approvalId: string;
  tenantId: string;
  decisionId: string;
  workloadInstanceId?: string;
}

export interface EmitLlmCallPostRequest extends CommitEstimatedRequest {
  // alias type; emitLlmCallPost is the lower-level entry point that
  // commitEstimated wraps.
}

export interface SpanRecord {
  name: string;
  startTimeMs: number;
  durationMs: number;
  attributes: Record<string, string | number | boolean | undefined>;
  error?: Error;
}

export class SpendGuardClient implements AsyncDisposable {
  // ... (implementation; see §4 below)
}
```

### 3.3 Subpath barrels

Each subpath (`./ids`, `./errors`, …) is a tiny file that re-exports from its source module. Goal: tree-shaking-friendly imports.

```ts
// src/ids.ts already exports its symbols; dist/ids.js is a leaf module.
// No additional barrel needed for single-source subpaths.
```

For `./pricing/demo`:

```ts
// src/pricing/demo.ts (committed; generated by scripts/embedPricingSnapshot.ts at release time)
import type { PriceTable } from "../pricing.js";
export const DEMO_PRICING_VERSION = "v2026.05.09-1";
export const DEMO_PRICING: PriceTable = new Map<string, number>([
  // Generated from deploy/demo/init/pricing/seed.yaml at release time.
  // ["openai|gpt-4o-mini|input", 0.15], ...
]);
```

## 4. `SpendGuardClient` implementation skeleton

```ts
export class SpendGuardClient implements AsyncDisposable {
  private readonly opts: Required<Omit<SpendGuardClientOptions, "otelTracer" | "onSpan" | "runtime">> & {
    otelTracer?: SpendGuardClientOptions["otelTracer"];
    onSpan?: SpendGuardClientOptions["onSpan"];
  };
  private transport: GrpcTransport | null = null;
  private client: A.SidecarAdapterClient | null = null;
  private handshakeResult: HandshakeOutcome | null = null;
  private handshakeLock: Promise<HandshakeOutcome> | null = null;
  private readonly decisionCache = new DecisionCache({ maxEntries: 1024 });

  constructor(rawOpts: SpendGuardClientOptions = {}) {
    const env = resolveEnvConfig();
    const socketPath = rawOpts.socketPath ?? env.socketPath;
    const tenantId = rawOpts.tenantId ?? env.tenantId;
    if (!socketPath) {
      throw new SpendGuardConfigError(
        "socketPath is required (or set SPENDGUARD_SIDECAR_UDS)",
      );
    }
    if (!tenantId) {
      throw new SpendGuardConfigError(
        "tenantId is required (or set SPENDGUARD_TENANT_ID)",
      );
    }
    if (rawOpts.otelTracer && rawOpts.onSpan) {
      throw new SpendGuardConfigError(
        "otelTracer and onSpan are mutually exclusive",
      );
    }
    this.opts = {
      socketPath,
      tenantId,
      runtimeKind: rawOpts.runtimeKind ?? "",
      runtimeVersion: rawOpts.runtimeVersion ?? "",
      sdkVersion: rawOpts.sdkVersion ?? VERSION,
      protocolVersion: rawOpts.protocolVersion ?? 1,
      capabilityLevel: rawOpts.capabilityLevel ?? 0x40,
      workloadInstanceId: rawOpts.workloadInstanceId ?? env.workloadInstanceId ?? "",
      decisionTimeoutMs: rawOpts.decisionTimeoutMs ?? env.decisionTimeoutMs ?? DEFAULT_DECISION_TIMEOUT_MS,
      handshakeTimeoutMs: rawOpts.handshakeTimeoutMs ?? env.handshakeTimeoutMs ?? DEFAULT_HANDSHAKE_TIMEOUT_MS,
      publishTimeoutMs: rawOpts.publishTimeoutMs ?? DEFAULT_PUBLISH_TIMEOUT_MS,
      traceTimeoutMs: rawOpts.traceTimeoutMs ?? DEFAULT_TRACE_TIMEOUT_MS,
      otelTracer: rawOpts.otelTracer,
      onSpan: rawOpts.onSpan,
      disabled: rawOpts.disabled ?? env.disabled,
    };
  }

  async connect(): Promise<void> {
    if (this.opts.disabled) return;
    if (this.client) return;
    // grpc-js's `unix:` URI scheme. Set `grpc.default_authority` to "localhost"
    // — otherwise tonic rejects the URL-encoded UDS path as authority and
    // resets streams with PROTOCOL_ERROR. Same bug as Python SDK at
    // sdk/python/src/spendguard/client.py:240-251.
    const target = `unix:${this.opts.socketPath}`;
    this.transport = new GrpcTransport({
      host: target,
      channelCredentials: credentials.createInsecure(),
      channelOptions: {
        "grpc.default_authority": "localhost",
      },
    });
    this.client = new A.SidecarAdapterClient(this.transport);
  }

  async close(): Promise<void> {
    const t = this.transport;
    this.transport = null;
    this.client = null;
    if (t) {
      try {
        t.close();
      } catch {
        /* idempotent */
      }
    }
  }

  async [Symbol.asyncDispose](): Promise<void> {
    await this.close();
  }

  get tenantId(): string {
    return this.opts.tenantId;
  }

  get sessionId(): string {
    if (!this.handshakeResult) {
      throw new HandshakeError(
        "handshake() has not completed; sessionId is not yet known",
      );
    }
    return this.handshakeResult.sessionId;
  }

  get handshakeOutcome(): HandshakeOutcome {
    if (!this.handshakeResult) {
      throw new HandshakeError("handshake() has not completed");
    }
    return this.handshakeResult;
  }

  async handshake(args: { workloadInstanceId?: string } = {}): Promise<HandshakeOutcome> {
    if (this.opts.disabled) {
      return makeDisabledHandshake();
    }
    if (this.handshakeResult) return this.handshakeResult;
    if (this.handshakeLock) return this.handshakeLock;
    this.handshakeLock = this.doHandshake(args).finally(() => {
      this.handshakeLock = null;
    });
    return this.handshakeLock;
  }

  private async doHandshake(args: { workloadInstanceId?: string }): Promise<HandshakeOutcome> {
    if (!this.client) await this.connect();
    const req: A.HandshakeRequest = {
      sdkVersion: this.opts.sdkVersion,
      runtimeKind: this.opts.runtimeKind,
      runtimeVersion: this.opts.runtimeVersion,
      capabilityLevel: this.opts.capabilityLevel,
      tenantIdAssertion: this.opts.tenantId,
      workloadInstanceId: args.workloadInstanceId ?? this.opts.workloadInstanceId,
      protocolVersion: this.opts.protocolVersion,
    };
    const res = await this.callWithRetry(
      "handshake",
      () => this.client!.handshake(req, { deadline: ms(this.opts.handshakeTimeoutMs) }).response,
    );
    if (res.protocolVersion !== this.opts.protocolVersion) {
      throw new HandshakeError(
        `protocol version mismatch: adapter=${this.opts.protocolVersion} sidecar=${res.protocolVersion}`,
      );
    }
    const outcome: HandshakeOutcome = {
      sessionId: res.sessionId,
      sidecarVersion: res.sidecarVersion,
      schemaBundleId: res.schemaBundle?.schemaBundleId ?? "",
      schemaBundleHash: res.schemaBundle?.schemaBundleHash ?? new Uint8Array(),
      contractBundleId: res.contractBundle?.bundleId ?? "",
      contractBundleHash: res.contractBundle?.bundleHash ?? new Uint8Array(),
      capabilityRequired: Number(res.capabilityRequired ?? 0),
      signingKeyId: res.signingKeyId,
      announcementSignature: res.announcementSignature,
    };
    if (outcome.capabilityRequired > this.opts.capabilityLevel) {
      throw new HandshakeError(
        `sidecar requires capability ${hex(outcome.capabilityRequired)} but adapter advertised ${hex(this.opts.capabilityLevel)}`,
      );
    }
    this.handshakeResult = outcome;
    return outcome;
  }

  async reserve(req: ReserveRequest): Promise<DecisionOutcome> {
    if (this.opts.disabled) return makeDisabledDecision(req);
    const cached = this.decisionCache.get(req.idempotencyKey);
    if (cached) return cached;
    return wrapSpan(this.opts, "spendguard.reserve", { decisionId: req.decisionId, trigger: req.trigger }, async () => {
      const decisionReq = await this.buildDecisionRequest(req);
      const res = await this.callWithRetry(
        "reserve",
        () => this.client!.requestDecision(decisionReq, { deadline: ms(this.opts.decisionTimeoutMs) }).response,
      );
      const outcome = this.mapDecisionResponse(res);
      this.decisionCache.set(req.idempotencyKey, outcome);
      return outcome;
    });
  }

  // alias — identical fn ref so `requestDecision === reserve` is true at runtime
  readonly requestDecision = this.reserve.bind(this);

  async commitEstimated(req: CommitEstimatedRequest): Promise<void> {
    if (this.opts.disabled) return;
    return wrapSpan(this.opts, "spendguard.commitEstimated", { decisionId: req.decisionId }, async () => {
      const event = this.buildLlmCallPostEvent(req);
      await this.emitLlmCallPostEvent(event);
    });
  }

  async release(req: ReleaseRequest): Promise<ReleaseOutcome> {
    if (this.opts.disabled) return makeDisabledRelease(req);
    return wrapSpan(this.opts, "spendguard.release", { reservationId: req.reservationId }, async () => {
      const grpcReq: A.ReleaseReservationRequest = {
        reservationId: req.reservationId,
        idempotencyKey: req.idempotencyKey,
        reasonCodes: [...(req.reasonCodes ?? [])],
        tenantId: req.tenantId ?? "",
        workloadInstanceId: req.workloadInstanceId ?? "",
        sessionId: this.sessionId,
      };
      const res = await this.callWithRetry(
        "release",
        () => this.client!.releaseReservation(grpcReq, { deadline: ms(this.opts.publishTimeoutMs) }).response,
      );
      return {
        auditEventSignature: res.auditEventSignature,
        ledgerTransactionId: res.ledgerTransactionId,
        releasedReservationIds: res.releasedReservationIds,
      };
    });
  }

  async queryBudget(_req: QueryBudgetRequest): Promise<QueryBudgetResult> {
    // v0.1.0 placeholder — wire path defined, sidecar RPC not yet
    // implemented. Adapters call this method and catch this error to
    // surface a clear "feature not yet available" upstream rather than
    // a gRPC NOT_FOUND at random.
    throw new SpendGuardError(
      "queryBudget is not yet wired in sidecar v1; tracked at https://github.com/m24927605/agentic-spendguard/issues (TBD)",
    );
  }

  // ── lower-level methods ────────────────────────────────────────────

  async confirmPublishOutcome(req: PublishOutcomeRequest): Promise<string> {
    if (this.opts.disabled) return "disabled-noop";
    const grpcReq: A.PublishOutcomeRequest = {
      sessionId: this.sessionId,
      decisionId: req.decisionId,
      effectHash: req.effectHash,
      outcome: mapOutcomeName(req.outcome),
      adapterError: req.adapterError ?? "",
    };
    const res = await this.callWithRetry(
      "confirmPublishOutcome",
      () => this.client!.confirmPublishOutcome(grpcReq, { deadline: ms(this.opts.publishTimeoutMs) }).response,
    );
    if (res.error?.code) {
      throw new SpendGuardError(
        `sidecar publish error code=${res.error.code} message=${res.error.message}`,
      );
    }
    return res.auditOutcomeEventId;
  }

  async resumeAfterApproval(req: ResumeAfterApprovalRequest): Promise<DecisionOutcome> {
    // mirrors Python `resume_after_approval`. See errors.ts for the
    // typed exception mapping of denied / lapsed / bundle-hot-reloaded
    // responses. (Implementation omitted from skeleton; mirrors the
    // Python `client.py` resume_after_approval branch logic line-for-line.)
    throw new Error("not implemented in skeleton — see implementation slice S05_04");
  }

  async safeConfirmApplyFailed(req: ApplyFailedRequest): Promise<void> {
    try {
      await this.confirmPublishOutcome({
        decisionId: req.decisionId,
        effectHash: req.effectHash,
        outcome: "APPLY_FAILED",
        adapterError: req.adapterError.slice(0, 1024),
      });
    } catch (err) {
      // Never shadow the caller's original exception.
      if (this.opts.onSpan) {
        this.opts.onSpan({
          name: "spendguard.safeConfirmApplyFailed.warning",
          startTimeMs: Date.now(),
          durationMs: 0,
          attributes: { decisionId: req.decisionId },
          error: err instanceof Error ? err : new Error(String(err)),
        });
      }
    }
  }

  async emitLlmCallPost(req: EmitLlmCallPostRequest): Promise<void> {
    return this.commitEstimated(req);
  }

  // ── private helpers ────────────────────────────────────────────────

  private async buildDecisionRequest(req: ReserveRequest): Promise<A.DecisionRequest> {
    const plan = currentRunPlan();
    const plannedStepsHint = plan ? plan.plannedCalls + (plan.plannedTools ?? 0) : 0;
    const runtimeMetadata = await this.buildRuntimeMetadata(req);
    return {
      sessionId: this.sessionId,
      trigger: triggerEnumOf(req.trigger),
      trace: buildTraceContext(req.traceparent, req.tracestate),
      ids: {
        runId: req.runId,
        stepId: req.stepId,
        llmCallId: req.llmCallId,
        toolCallId: req.toolCallId ?? "",
        decisionId: req.decisionId,
      },
      route: req.route,
      inputs: {
        projectedClaims: req.projectedClaims,
        projectedP50Atomic: req.projectedP50Atomic ?? "",
        projectedP90Atomic: req.projectedP90Atomic ?? "",
        projectedP95Atomic: req.projectedP95Atomic ?? "",
        projectedP99Atomic: req.projectedP99Atomic ?? "",
        projectedUnit: req.projectedUnit,
        runtimeMetadata,
        claimEstimate: req.claimEstimate ? mapClaimEstimate(req.claimEstimate) : undefined,
      },
      parentRunId: req.parentRunId ?? "",
      budgetGrantJti: req.budgetGrantJti ?? "",
      idempotency: { key: req.idempotencyKey, requestHash: new Uint8Array() },
      plannedStepsHint,
    };
  }

  private async buildRuntimeMetadata(req: ReserveRequest): Promise<Record<string, unknown> | undefined> {
    if (req.promptText === undefined && !req.decisionContextJson) return undefined;
    const meta: Record<string, unknown> = {};
    if (req.promptText !== undefined) {
      meta.prompt_hash = computePromptHash(req.promptText, this.opts.tenantId);
    }
    if (req.decisionContextJson) {
      for (const [k, v] of Object.entries(req.decisionContextJson)) {
        if (!(k in meta)) meta[k] = v;
      }
    }
    return meta;
  }

  private mapDecisionResponse(res: A.DecisionResponse): DecisionOutcome {
    const name = decisionEnumName(res.decision);
    if (name === "CONTINUE" || name === "DEGRADE") {
      return {
        decisionId: res.decisionId,
        auditDecisionEventId: res.auditDecisionEventId,
        decision: name,
        mutationPatchJson: res.mutationPatchJson,
        effectHash: res.effectHash,
        ledgerTransactionId: res.ledgerTransactionId,
        reservationIds: res.reservationIds,
        ttlExpiresAtSeconds: Number(res.ttlExpiresAt?.seconds ?? 0n),
        reasonCodes: res.reasonCodes,
        matchedRuleIds: res.matchedRuleIds,
      };
    }
    if (name === "STOP" || name === "STOP_RUN_PROJECTION") {
      throw new DecisionStopped(
        `sidecar ${name} terminal=${res.terminal} reasons=${JSON.stringify(res.reasonCodes)}`,
        {
          decisionId: res.decisionId,
          reasonCodes: [...res.reasonCodes],
          auditDecisionEventId: res.auditDecisionEventId,
          matchedRuleIds: [...res.matchedRuleIds],
        },
      );
    }
    if (name === "SKIP") {
      throw new DecisionSkipped(
        `sidecar SKIP reasons=${JSON.stringify(res.reasonCodes)}`,
        {
          decisionId: res.decisionId,
          reasonCodes: [...res.reasonCodes],
          auditDecisionEventId: res.auditDecisionEventId,
          matchedRuleIds: [...res.matchedRuleIds],
        },
      );
    }
    if (name === "REQUIRE_APPROVAL") {
      throw new ApprovalRequired(
        `sidecar REQUIRE_APPROVAL approval_request_id=${res.approvalRequestId}`,
        {
          decisionId: res.decisionId,
          approvalRequestId: res.approvalRequestId,
          approverRole: res.approverRole,
          reasonCodes: [...res.reasonCodes],
          auditDecisionEventId: res.auditDecisionEventId,
          matchedRuleIds: [...res.matchedRuleIds],
          tenantId: this.opts.tenantId,
        },
      );
    }
    throw new DecisionDenied(`sidecar returned unknown decision=${res.decision}`, {
      decisionId: res.decisionId,
      reasonCodes: [...res.reasonCodes],
      auditDecisionEventId: res.auditDecisionEventId,
      matchedRuleIds: [...res.matchedRuleIds],
    });
  }

  private async callWithRetry<T>(op: string, fn: () => Promise<T>): Promise<T> {
    return runWithRetry({ maxAttempts: 2, baseBackoffMs: 25, jitterMs: 25 }, async () => {
      try {
        return await fn();
      } catch (err) {
        throw classifyRpcError(err, op);
      }
    });
  }

  private buildLlmCallPostEvent(req: CommitEstimatedRequest): A.TraceEvent {
    // build TraceEvent.LLM_CALL_POST per proto
    // (implementation omitted; mirrors Python emit_llm_call_post)
    throw new Error("see slice S05_04");
  }

  private async emitLlmCallPostEvent(_event: A.TraceEvent): Promise<void> {
    // bidi stream — open, send one event, await one ack, close. Same
    // pattern as Python `emit_llm_call_post`. Implementation lives in
    // slice S05_04.
    throw new Error("see slice S05_04");
  }
}

function ms(timeoutMs: number): Date {
  return new Date(Date.now() + timeoutMs);
}

function hex(n: number): string {
  return `0x${n.toString(16)}`;
}

function makeDisabledHandshake(): HandshakeOutcome { /* … */ throw new Error("see slice S05_03"); }
function makeDisabledDecision(_req: ReserveRequest): DecisionOutcome { /* … */ throw new Error("see slice S05_03"); }
function makeDisabledRelease(_req: ReleaseRequest): ReleaseOutcome { /* … */ throw new Error("see slice S05_03"); }
function classifyRpcError(err: unknown, op: string): SpendGuardError { /* … */ throw new Error("see slice S05_03"); }
function buildTraceContext(_tp: string | undefined, _ts: string | undefined): C.TraceContext { /* … */ throw new Error(""); }
function triggerEnumOf(_t: string): A.DecisionRequest_Trigger { /* … */ throw new Error(""); }
function decisionEnumName(_d: number): string { /* … */ throw new Error(""); }
function mapOutcomeName(_o: string): A.PublishOutcomeRequest_Outcome { /* … */ throw new Error(""); }
function mapClaimEstimate(_e: ClaimEstimate): A.ClaimEstimate { /* … */ throw new Error(""); }
```

## 5. `src/errors.ts`

```ts
export class SpendGuardError extends Error {
  override name = "SpendGuardError";
}

export class HandshakeError extends SpendGuardError {
  override name = "HandshakeError";
}

export class SidecarUnavailable extends SpendGuardError {
  override name = "SidecarUnavailable";
  readonly statusCode = 503 as const;
  constructor(message: string, opts?: { cause?: unknown }) {
    super(message);
    if (opts?.cause !== undefined) (this as { cause?: unknown }).cause = opts.cause;
  }
}

export interface DecisionDeniedInit {
  decisionId: string;
  reasonCodes?: string[];
  auditDecisionEventId?: string;
  matchedRuleIds?: string[];
}

export class DecisionDenied extends SpendGuardError {
  override name = "DecisionDenied";
  readonly statusCode = 403 as const;
  readonly decisionId: string;
  readonly reasonCodes: string[];
  readonly auditDecisionEventId?: string;
  readonly matchedRuleIds: string[];
  constructor(message: string, init: DecisionDeniedInit) {
    super(message);
    this.decisionId = init.decisionId;
    this.reasonCodes = init.reasonCodes ?? [];
    this.auditDecisionEventId = init.auditDecisionEventId;
    this.matchedRuleIds = init.matchedRuleIds ?? [];
  }
}

export class DecisionStopped extends DecisionDenied { override name = "DecisionStopped"; }
export class DecisionSkipped extends DecisionDenied { override name = "DecisionSkipped"; }

export interface ApprovalRequiredInit extends DecisionDeniedInit {
  approvalRequestId: string;
  approverRole?: string;
  tenantId?: string;
}

export class ApprovalRequired extends DecisionDenied {
  override name = "ApprovalRequired";
  readonly approvalRequestId: string;
  readonly approverRole?: string;
  readonly tenantId?: string;
  constructor(message: string, init: ApprovalRequiredInit) {
    super(message, init);
    this.approvalRequestId = init.approvalRequestId;
    this.approverRole = init.approverRole;
    this.tenantId = init.tenantId;
  }
  async resume(client: { resumeAfterApproval(req: { approvalId: string; tenantId: string; decisionId: string }): Promise<unknown> }): Promise<unknown> {
    return client.resumeAfterApproval({
      approvalId: this.approvalRequestId,
      tenantId: this.tenantId ?? "",
      decisionId: this.decisionId,
    });
  }
}

export class ApprovalDeniedError extends DecisionDenied {
  override name = "ApprovalDeniedError";
  readonly approverSubject?: string;
  readonly approverReason?: string;
  constructor(message: string, init: DecisionDeniedInit & { approverSubject?: string; approverReason?: string }) {
    super(message, { ...init, reasonCodes: ["approval_denied", ...(init.reasonCodes ?? [])] });
    this.approverSubject = init.approverSubject;
    this.approverReason = init.approverReason;
  }
}

export class ApprovalLapsedError extends DecisionDenied {
  override name = "ApprovalLapsedError";
  readonly state: "pending" | "expired" | "cancelled" | "unknown";
  constructor(message: string, init: DecisionDeniedInit & { state: "pending" | "expired" | "cancelled" | "unknown" }) {
    super(message, { ...init, reasonCodes: [`approval_lapsed_${init.state}`, ...(init.reasonCodes ?? [])] });
    this.state = init.state;
  }
}

export class ApprovalBundleHotReloadedError extends SpendGuardError {
  override name = "ApprovalBundleHotReloadedError";
  readonly originalBundleHash: string;
  readonly currentBundleHash: string;
  constructor(message: string, init: { originalBundleHash: string; currentBundleHash: string }) {
    super(message);
    this.originalBundleHash = init.originalBundleHash;
    this.currentBundleHash = init.currentBundleHash;
  }
}

export class MutationApplyFailed extends SpendGuardError { override name = "MutationApplyFailed"; }
export class SpendGuardConfigError extends SpendGuardError { override name = "SpendGuardConfigError"; }
```

## 6. `src/ids.ts`

```ts
import { randomBytes, createHash } from "node:crypto";

export function newUuid7(): string {
  const tsMs = BigInt(Date.now()) & ((1n << 48n) - 1n);
  const randA = randomBytes(2).readUInt16BE(0) & 0x0fff;
  const randB = randomBytes(8);
  randB[0] = (randB[0] & 0x3f) | 0x80;
  const hi = (tsMs << 16n) | BigInt(randA);
  // assemble per RFC 9562 §5.7
  const u = Buffer.alloc(16);
  u.writeBigUInt64BE(hi, 0);
  // overwrite top nibble of byte 6 with version 7
  u[6] = (u[6] & 0x0f) | 0x70;
  randB.copy(u, 8);
  return [
    u.toString("hex", 0, 4),
    u.toString("hex", 4, 6),
    u.toString("hex", 6, 8),
    u.toString("hex", 8, 10),
    u.toString("hex", 10, 16),
  ].join("-");
}

export function deriveIdempotencyKey(args: {
  tenantId: string;
  sessionId: string;
  runId: string;
  stepId: string;
  llmCallId: string;
  trigger: string;
}): string {
  // BYTE-IDENTICAL to Python `derive_idempotency_key`: BLAKE2b digest_size=16,
  // canonical = "v1\x1ftenant\x1fsession\x1frun\x1fstep\x1fllm\x1ftrigger".
  const canonical = ["v1", args.tenantId, args.sessionId, args.runId, args.stepId, args.llmCallId, args.trigger]
    .join("");
  // Node 20+ exposes BLAKE2b-256 via crypto.createHash("blake2b512") (default 512-bit);
  // we need 128-bit. Use libsodium or a tiny WASM blake2b impl. Slice S05_06
  // vendors a 6 KB blake2b wasm or a pure-TS impl; tests gate cross-language
  // determinism.
  const digest = blake2b16(Buffer.from(canonical, "utf8"));
  return `sg-${digest.toString("hex")}`;
}

export function deriveUuidFromSignature(signature: string, args: { scope: string }): string {
  const buf = blake2b16(Buffer.from(`${args.scope}|${signature}`, "utf8"));
  buf[6] = (buf[6] & 0x0f) | 0x40;
  buf[8] = (buf[8] & 0x3f) | 0x80;
  return [
    buf.toString("hex", 0, 4),
    buf.toString("hex", 4, 6),
    buf.toString("hex", 6, 8),
    buf.toString("hex", 8, 10),
    buf.toString("hex", 10, 16),
  ].join("-");
}

export function defaultCallSignature(messages: unknown[], modelSettings?: unknown): string {
  const h = blake2b16Streaming();
  h.update(Buffer.from("v1:call:", "utf8"));
  for (let i = 0; i < messages.length; i++) {
    h.update(Buffer.from(`|msg${i}|`, "utf8"));
    const m = messages[i];
    if (m && typeof m === "object" && "toJSON" in m) {
      h.update(Buffer.from(JSON.stringify(m), "utf8"));
    } else {
      h.update(Buffer.from(safeRepr(m), "utf8"));
    }
  }
  h.update(Buffer.from("|settings|", "utf8"));
  if (modelSettings === undefined || modelSettings === null) {
    h.update(Buffer.from("none", "utf8"));
  } else {
    h.update(Buffer.from(JSON.stringify(modelSettings, Object.keys(modelSettings as object).sort()), "utf8"));
  }
  return h.digest().toString("hex");
}

export function workloadInstanceId(): string {
  return process.env.SPENDGUARD_WORKLOAD_INSTANCE_ID ?? "";
}

// internal — blake2b 128-bit; vendored via @noble/hashes (zero-dep, audited).
declare function blake2b16(buf: Buffer): Buffer;
declare function blake2b16Streaming(): { update(b: Buffer): void; digest(): Buffer };
declare function safeRepr(v: unknown): string;
```

`@noble/hashes` adds ~20 KB minified — acceptable within the 120 KB budget (§10 `design.md`). It is a dev-audited, zero-dep crypto lib used by major TS ecosystems; vendoring it avoids a `node:crypto` BLAKE2b-128 gap on older Node 20.10 versions.

## 7. `src/promptHash.ts`

```ts
import { createHmac } from "node:crypto";

const ASCII_WHITESPACE = " \t\n\f\r"; // match Rust `char::is_ascii_whitespace`

const UUID_RE = /^[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}$/;

function canonicalizeTenant(tenantId: string): string {
  if (UUID_RE.test(tenantId)) return tenantId.toLowerCase();
  return tenantId;
}

export function computePromptHash(promptText: string, tenantId: string): string {
  const tenant = canonicalizeTenant(tenantId);
  const trimmed = trim(promptText, ASCII_WHITESPACE);
  return createHmac("sha256", tenant).update(trimmed, "utf8").digest("hex");
}

function trim(s: string, set: string): string {
  let i = 0;
  while (i < s.length && set.includes(s[i])) i++;
  let j = s.length;
  while (j > i && set.includes(s[j - 1])) j--;
  return s.slice(i, j);
}
```

## 8. `src/pricing.ts`

```ts
export const USD_MICROS_PER_USD = 1_000_000;
export type PriceKey = readonly [provider: string, model: string, tokenKind: string];
export type PriceTable = ReadonlyMap<string, number>; // key = `${provider}|${model}|${kind}`

export class PricingLookup {
  private readonly table: PriceTable;
  private readonly defaultKind: string;
  constructor(table: PriceTable, opts?: { defaultKind?: string }) {
    this.table = table;
    this.defaultKind = opts?.defaultKind ?? "output";
  }
  pricePerMillion(provider: string, model: string, tokenKind: string): number | null {
    const v = this.table.get(`${provider}|${model}|${tokenKind}`);
    return v ?? null;
  }
  usdMicrosForCall(args: {
    provider: string;
    model: string;
    inputTokens?: number;
    outputTokens?: number;
    cachedInputTokens?: number;
  }): number {
    let usd = 0;
    const charge = (kind: string, count: number) => {
      if (count <= 0) return;
      const p = this.pricePerMillion(args.provider, args.model, kind)
        ?? this.pricePerMillion(args.provider, args.model, this.defaultKind)
        ?? 0;
      usd += (count * p) / 1_000_000;
    };
    charge("input", args.inputTokens ?? 0);
    charge("output", args.outputTokens ?? 0);
    charge("cached_input", args.cachedInputTokens ?? 0);
    // round up — never under-charge the customer
    return Math.max(1, Math.ceil(usd * USD_MICROS_PER_USD));
  }
}
```

## 9. `src/runPlan.ts`

```ts
import { AsyncLocalStorage } from "node:async_hooks";

export interface RunPlan {
  plannedCalls: number;
  plannedTools: number;
}

const storage = new AsyncLocalStorage<RunPlan>();

export function currentRunPlan(): RunPlan | null {
  return storage.getStore() ?? null;
}

export function withRunPlan<TArgs extends unknown[], TRet>(
  plan: { plannedCalls: number; plannedTools?: number },
  fn: (...args: TArgs) => TRet | Promise<TRet>,
): (...args: TArgs) => Promise<TRet> {
  if (!Number.isInteger(plan.plannedCalls) || plan.plannedCalls < 0) {
    throw new TypeError(`withRunPlan: plannedCalls must be a non-negative integer, got ${plan.plannedCalls}`);
  }
  const tools = plan.plannedTools ?? 0;
  if (!Number.isInteger(tools) || tools < 0) {
    throw new TypeError(`withRunPlan: plannedTools must be a non-negative integer, got ${plan.plannedTools}`);
  }
  const fullPlan: RunPlan = { plannedCalls: plan.plannedCalls, plannedTools: tools };
  return async (...args: TArgs): Promise<TRet> => {
    // Nested: outer wins per spec — defer to existing plan when present.
    const existing = storage.getStore();
    if (existing) return await fn(...args);
    return await storage.run(fullPlan, () => fn(...args));
  };
}
```

## 10. `src/decisionCache.ts`

A tiny LRU keyed by `idempotencyKey`. Default 1024 entries, TTL 5 minutes. The cache prevents same-process retries from issuing duplicate `reserve` calls. The sidecar has its own idempotency cache; the local one is a latency optimisation, not a correctness gate.

```ts
export class DecisionCache {
  // standard LRU; implementation omitted
}
```

## 11. `src/retry.ts`

Bounded retry, applies to `UNAVAILABLE` / `DEADLINE_EXCEEDED` / `CANCELLED` per the same buckets Python's `_classify_rpc_error` uses. Idempotency-key required (see `design.md` §6.5).

## 12. `src/otel.ts`

Span wrapper. If `otelTracer` is set, opens an OTel span. Else if `onSpan` is set, records to the callback. Else no-op.

## 13. `tsup.config.ts`

```ts
import { defineConfig } from "tsup";

export default defineConfig({
  entry: {
    index: "src/index.ts",
    client: "src/client.ts",
    errors: "src/errors.ts",
    ids: "src/ids.ts",
    pricing: "src/pricing.ts",
    "pricing/demo": "src/pricing/demo.ts",
    promptHash: "src/promptHash.ts",
    runPlan: "src/runPlan.ts",
    "_proto/index": "src/_proto/index.ts",
  },
  format: ["esm"],
  dts: true,
  splitting: false,        // we manage subpath splits via `entry`
  sourcemap: true,
  clean: true,
  target: "node20",
  treeshake: true,
});
```

## 14. `biome.json`

```json
{
  "$schema": "https://biomejs.dev/schemas/1.9.4/schema.json",
  "files": { "include": ["src", "tests", "scripts"] },
  "linter": {
    "enabled": true,
    "rules": {
      "recommended": true,
      "style": { "noNonNullAssertion": "off" },
      "suspicious": { "noExplicitAny": "warn" }
    }
  },
  "formatter": { "enabled": true, "indentStyle": "space", "indentWidth": 2, "lineWidth": 100 },
  "organizeImports": { "enabled": true }
}
```

## 15. `vitest.config.ts`

```ts
import { defineConfig } from "vitest/config";

export default defineConfig({
  test: {
    environment: "node",
    include: ["tests/**/*.test.ts"],
    coverage: { provider: "v8", reporter: ["text", "lcov"] },
    pool: "forks",          // UDS server in mocks needs distinct sockets
    testTimeout: 10_000,
    hookTimeout: 10_000,
  },
});
```

## 16. `scripts/proto.ts`

Reads `proto/spendguard/**/*.proto`, runs `protoc` via `@protobuf-ts/plugin`, writes to `src/_proto/`. Deterministic output, idempotent.

## 17. `scripts/verify-size.ts`

Builds the package, measures `dist/index.js` minified + gzipped, asserts against the budgets in `design.md` §10. Fails the build if any budget is exceeded.

## 18. `.github/workflows/sdk-ts-publish.yml`

```yaml
name: Publish @spendguard/sdk to npm

on:
  release:
    types: [published]
  workflow_dispatch:

jobs:
  publish:
    if: startsWith(github.ref, 'refs/tags/ts-sdk-v')
    runs-on: ubuntu-latest
    environment:
      name: npm
      url: https://www.npmjs.com/package/@spendguard/sdk/v/${{ github.ref_name }}
    permissions:
      id-token: write       # OIDC for npm provenance
      contents: read
    steps:
      - uses: actions/checkout@v4
      - uses: actions/setup-node@v4
        with: { node-version: "20.x", registry-url: "https://registry.npmjs.org" }
      - uses: pnpm/action-setup@v4
        with: { version: 9.x }
      - name: Install
        working-directory: sdk/typescript
        run: pnpm install --frozen-lockfile
      - name: Verify proto codegen is committed
        working-directory: sdk/typescript
        run: pnpm run proto && git diff --exit-code src/_proto
      - name: Lint + typecheck + test
        working-directory: sdk/typescript
        run: |
          pnpm run lint
          pnpm run typecheck
          pnpm run test
      - name: Build
        working-directory: sdk/typescript
        run: pnpm run build
      - name: Verify bundle size budget
        working-directory: sdk/typescript
        run: pnpm run size
      - name: Publish (provenance)
        working-directory: sdk/typescript
        run: npm publish --provenance --access public
        env:
          NODE_AUTH_TOKEN: ${{ secrets.NPM_AUTOMATION_TOKEN }}
          # Trusted Publisher OIDC flow: when configured in npm, the
          # automation token is not used; OIDC provenance attestation is
          # automatic. Token kept as a fallback during the initial setup
          # window per npm's published Trusted Publisher rollout schedule.
```

## 19. Open implementation TODOs explicitly out of v0.1.0

- ProviderReport commit path (`LlmCallPostPayload.provider_reported_amount_atomic`): deferred; only CommitEstimated wired in v0.1.0.
- `IssueBudgetGrant` / `RevokeBudgetGrant` / `ConsumeBudgetGrant`: forward-reserved on the proto, not surfaced as TS methods in v0.1.0. Added in v0.2 when a downstream adapter needs sub-agent budget grants.
- `StreamDrainSignal`: forward-reserved, not surfaced as TS method in v0.1.0. Adapters running long-lived processes will need it in v0.2.
- `queryBudget` server wire (see `design.md` §9 decision 4).
- Tokenizer (see `design.md` §9 decision 8).

Each TODO becomes a GitHub issue at v0.1.0 release time and is referenced from the JSDoc on the corresponding TS surface.
