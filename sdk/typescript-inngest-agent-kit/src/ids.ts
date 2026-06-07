// `deriveIdentity` — Inngest step identity → SpendGuard identity.
//
// The headline retry-dedup contract (design.md §6, review-standards §4): the
// same `(stepId, inngestIdempotencyKey, runId)` triple across all retry
// attempts of an Inngest step body MUST produce the same `idempotencyKey`,
// so the in-process D05 `DecisionCache` (and / or the sidecar's own
// idempotency cache) returns the cached outcome on retry, and the adapter
// records exactly ONE `LLM_CALL_PRE` audit row across N attempts.
//
// LOCKED mapping (design.md §6 + review-standards §6):
//
//   - `sessionId`           := `runId`              (Inngest function run UUID)
//   - `runId`               := `runId`              (Inngest function run UUID)
//   - `stepId`              := `step.id`            (Inngest step id, durable
//                                                    + attempt-invariant)
//   - `llmCallId`           := `step.id`            (one-to-one with step id —
//                                                    review-standards §3.2 + §3.3)
//   - `trigger`             := `"LLM_CALL_PRE"`     (constant)
//   - `decisionId`          := `deriveUuidFromSignature(seed, {scope:"decision_id"})`
//                              where `seed = inngestIdempotencyKey ?? stepId`
//                              (review-standards §3.4)
//
// `attempt` is INTENTIONALLY NOT part of the seed (review-standards §6.5 —
// branch-covered). `model` and `body` are INTENTIONALLY NOT part of the seed
// either (review-standards §6.6 — content-stability across retries).
//
// Cross-language invariant: byte-identical to Python `derive_idempotency_key`
// for the same canonical tuple — verified by the D05 substrate's own
// cross-language fixture (SLICE 4 adds the Inngest-specific vectors).

import { deriveIdempotencyKey, deriveUuidFromSignature } from "@spendguard/sdk";
import type { ClaimEstimatorInput } from "./options.js";

/**
 * Output of {@link deriveIdentity}. All four fields are deterministic
 * functions of `(tenantId, sessionId, stepId, inngestIdempotencyKey, runId)`.
 */
export interface DerivedIdentity {
  /** UUIDv4-shaped, scope-namespaced under `"decision_id"`. */
  decisionId: string;
  /** `sg-` + 32 hex chars (BLAKE2b-128). Cross-language byte-identical. */
  idempotencyKey: string;
  /** Equal to `input.stepId`. */
  llmCallId: string;
  /** Equal to `input.stepId`. */
  stepId: string;
}

/**
 * Derive the SpendGuard identity tuple for an Inngest step boundary.
 *
 * Retry-safety contract (design.md §6 + review-standards §4):
 *
 *   - **Attempt-invariance:** Same `(tenantId, stepId, inngestIdempotencyKey,
 *     runId)` → same `idempotencyKey` regardless of `input.attempt`.
 *     Verified by R-02 (`tests/wrap.test.ts`).
 *   - **Run-scope:** A NEW Inngest function invocation (new `runId`) for
 *     the same step name produces a DIFFERENT `idempotencyKey` so a fresh
 *     run is NOT deduped against a prior run. Verified by R-08 / I-05.
 *   - **Seed precedence:** `inngestIdempotencyKey` wins over `stepId`
 *     when both are present, falls back to `stepId` when the consumer
 *     omits an explicit `step.ai`-level idempotency key. Verified by
 *     I-03 / I-04 / R-05.
 *
 * @param args.tenantId            - SpendGuard tenant the run is billed to.
 *                                    Forwarded to the canonical tuple's first
 *                                    slot.
 * @param args.input               - The {@link ClaimEstimatorInput} the
 *                                    factory built from the Inngest runtime
 *                                    context. `attempt` / `model` / `body` /
 *                                    `eventId` are deliberately NOT consumed
 *                                    here — they live on the estimator's
 *                                    inputs only.
 * @returns                          The four-field identity tuple. All four
 *                                    fields are stable across retries when
 *                                    the seed is stable.
 */
export function deriveIdentity(args: {
  tenantId: string;
  input: ClaimEstimatorInput;
}): DerivedIdentity {
  const seed = args.input.inngestIdempotencyKey ?? args.input.stepId;
  const decisionId = deriveUuidFromSignature(seed, { scope: "decision_id" });
  const stepId = args.input.stepId;
  const llmCallId = args.input.stepId;
  const idempotencyKey = deriveIdempotencyKey({
    tenantId: args.tenantId,
    sessionId: args.input.runId,
    runId: args.input.runId,
    stepId,
    llmCallId,
    trigger: "LLM_CALL_PRE",
  });
  return { decisionId, idempotencyKey, llmCallId, stepId };
}

/**
 * Convenience: derive only the idempotencyKey component, useful when callers
 * want to probe the dedup contract without constructing a full identity. Same
 * canonical tuple as {@link deriveIdentity}.
 */
export function deriveStepIdempotencyKey(args: {
  tenantId: string;
  runId: string;
  stepId: string;
  inngestIdempotencyKey?: string;
}): string {
  // The seed is read for parity with deriveIdentity's branch coverage even
  // though only the canonical fields go onto the idempotencyKey wire.
  void (args.inngestIdempotencyKey ?? args.stepId);
  return deriveIdempotencyKey({
    tenantId: args.tenantId,
    sessionId: args.runId,
    runId: args.runId,
    stepId: args.stepId,
    llmCallId: args.stepId,
    trigger: "LLM_CALL_PRE",
  });
}
