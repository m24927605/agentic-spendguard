// COV_D38_02 — typecheck-only consumer of the LOCKED §5 surface (A4.3).
//
// This file is compiled by `pnpm run typecheck` (tsconfig.tests.json) and
// never executed: it proves a downstream consumer can construct
// `SpendGuardProcessor` against the published option shape and mount it on
// a REAL typed `@mastra/core` Agent via the V5-pinned `inputProcessors`
// constructor key — including a model-router-string model, the path D06
// cannot reach.

import { Agent } from "@mastra/core/agent";
import type { Processor } from "@mastra/core/processors";
import type { SpendGuardClient } from "@spendguard/sdk";
import {
  type ClaimEstimator,
  type ClaimEstimatorInput,
  SpendGuardProcessor,
  type SpendGuardProcessorOptions,
} from "../../src/index.js";

// The consumer owns the client lifecycle (design §5 options doc) — a real
// consumer constructs `new SpendGuardClient({...})` + `connect()`; the
// typecheck consumer only needs the type.
declare const client: SpendGuardClient;

// Custom claim projection — exercises the ClaimEstimator + input types.
const estimator: ClaimEstimator = (input: ClaimEstimatorInput) => [
  {
    scopeId: "33333333-3333-4333-8333-333333333333",
    amountAtomic: String(Math.max(1, Math.ceil(input.stepText.length / 4)) * 1_000),
    unit: { unit: "USD_MICROS", denomination: 1 },
  },
];

// Day-1 unitId threading (HARDEN_D05_UR): typical operator source is the
// SPENDGUARD_UNIT_ID env var at construction.
const unitId = process.env.SPENDGUARD_UNIT_ID;

const options: SpendGuardProcessorOptions = {
  client,
  tenantId: "11111111-1111-4111-8111-111111111111",
  budgetId: "22222222-2222-4222-8222-222222222222",
  ...(unitId !== undefined ? { unitId } : {}),
  route: "mastra-llm",
  defaultBudgetMicrosCap: 5_000_000n,
  claimEstimator: estimator,
  runIdProvider: () => "sample-run-1",
};

export const guard = new SpendGuardProcessor(options);

// TP-02 companion: the processor satisfies the INSTALLED Processor type.
export const asProcessor: Processor = guard;

// V5 pin in consumer position: model-router-string Agent + inputProcessors.
export const agent = new Agent({
  id: "sample-spendguard-agent",
  name: "sample-spendguard-agent",
  instructions: "You are a sample agent.",
  model: "openai/gpt-4o-mini",
  inputProcessors: [guard],
});
