// src/identity.ts — §6.3 identity derivation. ALL derivation delegates to
// the `@spendguard/sdk` substrate (design §6.3 / §11.6 — P0 hash-reuse
// gate, review-standards §4). This module — and every other module in the
// package — contains ZERO node-crypto / noble-hashes imports (the literal
// import specifiers are deliberately not spelled out here so the
// COV_D38_04 hashReuse grep gate stays comment-proof).
//
// Skeleton copied from implementation.md §3.1 (not re-derived).

import { deriveIdempotencyKey, deriveUuidFromSignature } from "@spendguard/sdk";

export const STEP_ID_LLM_CALL = "llm_call";
const LLM_CALL_ID_SCOPE = "mastra_llm_call_id";

export interface StepIdentity {
  runId: string;
  llmCallId: string;
  decisionId: string;
  idempotencyKey: string;
}

export function deriveStepIdentity(args: {
  tenantId: string;
  stepText: string;
  /** From opts.runIdProvider / Mastra hook context (V3); undefined → content-derived. */
  externalRunId?: string;
}): StepIdentity {
  const signature = `v1|${args.tenantId}|${args.stepText}`;
  const llmCallId = deriveUuidFromSignature(signature, { scope: LLM_CALL_ID_SCOPE });
  // [VERIFY-AT-IMPL: V3] PINNED (COV_D38_02, @mastra/core 1.41.0): the
  // installed hook args expose NO Mastra run id (`ProcessInputStepArgs`
  // carries stepNumber/messageId/state but no run identifier), so the
  // only external source is `opts.runIdProvider` — absent that, the run id
  // is content-derived (= llmCallId), exactly the §6.3 LOCKED chain.
  const runId = args.externalRunId ?? llmCallId;
  return {
    runId,
    llmCallId,
    decisionId: llmCallId,
    idempotencyKey: deriveIdempotencyKey({
      tenantId: args.tenantId,
      sessionId: runId,
      runId,
      stepId: STEP_ID_LLM_CALL,
      llmCallId,
      trigger: "LLM_CALL_PRE",
    }),
  };
}
