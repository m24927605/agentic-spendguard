// `deriveIdempotencyKey` — adapter-side helper that maps LangChain's
// `(runId, parentRunId)` shape onto the substrate's canonical
// `(tenantId, sessionId, runId, stepId, llmCallId, trigger)` key.
//
// SLICE 3 wires only the `LLM_CALL_PRE` trigger — that is the single boundary
// where the adapter calls `client.reserve()`. The mapping rule (LOCKED for
// SLICE 3 per docs/internal/slices/COV_D04_S3_reserve_commit_wiring.md):
//
//   - `sessionId`   := `parentRunId ?? runId`
//                      LangChain's `parentRunId` is the run-manager UUID of
//                      the outer chain / agent that fired this LLM call. When
//                      the LLM is invoked at the top level (no chain wrapper),
//                      `parentRunId` is `undefined` and the runId itself
//                      stands in — preserving the "this run is its own
//                      session" semantics the substrate idempotency cache
//                      expects.
//   - `runId`       := LangChain's `runId` (the LLM call's run-manager UUID)
//   - `stepId`      := constant `"llm_call"` (SLICE 3 lock — design.md §6.3
//                      anticipates richer step-id shaping in later slices)
//   - `llmCallId`   := LangChain's `runId` (LangChain's run-id IS the call id;
//                      see review-standards.md §3.2 — exact equality)
//   - `trigger`     := constant `"LLM_CALL_PRE"`
//
// Cross-language guarantee: for byte-identical `(tenantId, runId, parentRunId)`
// inputs, this helper returns the same `sg-…` key the substrate's Python
// adapter would derive via `derive_idempotency_key(...)` with the same field
// canonicalisation. Verified end-to-end by `tests/handler.test.ts` —
// "idempotencyKey derivation deterministic for same runId+parentRunId+tenantId".

import { deriveIdempotencyKey as sdkDeriveKey } from "@spendguard/sdk";

/**
 * Derive the canonical idempotency key for a LangChain LLM-call boundary.
 *
 * The same `(tenantId, runId, parentRunId)` triple — invoked from any number
 * of retry attempts within a single LangChain run — produces the same key,
 * so the sidecar's idempotency cache + the ledger's `UNIQUE` constraint
 * collapse onto the first decision.
 *
 * @param opts.tenantId    - SpendGuard tenant the run is billed to.
 * @param opts.runId       - LangChain run-manager UUID for the LLM call.
 * @param opts.parentRunId - LangChain parent run-manager UUID, or `undefined`
 *                           when the LLM is invoked at the top level.
 * @returns                  `"sg-"` + 32 hex chars (BLAKE2b-128 digest).
 */
export function deriveIdempotencyKey(opts: {
  tenantId: string;
  runId: string;
  parentRunId?: string;
}): string {
  return sdkDeriveKey({
    tenantId: opts.tenantId,
    sessionId: opts.parentRunId ?? opts.runId,
    runId: opts.runId,
    stepId: "llm_call",
    llmCallId: opts.runId,
    trigger: "LLM_CALL_PRE",
  });
}
