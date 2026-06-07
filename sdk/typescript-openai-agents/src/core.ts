// `bracketedGetResponse` — shared PRE/POST bracket the `withSpendGuard`
// factory and the `SpendGuardAgentsModel` class both delegate to. ONE
// implementation, two call shapes (composition + subclass) per design.md §7
// locked decision #1. Two copies would risk drift between the two surfaces.
//
// Behaviour contract (design.md §5 verbatim):
//   1. Read `runId` from `currentRunContext()`. Outside any active context →
//      throw — `currentRunContext()` itself is the source of the error.
//      Reviewer gate 1.1 ("inner.getResponse only fires AFTER reserve
//      resolves") trivially holds because the throw fires before reserve
//      is even built.
//   2. `signature = deriveAgentSignature(input, systemInstructions)`.
//   3. Derive `(llmCallId, decisionId)` from the signature via
//      `@spendguard/sdk::deriveUuidFromSignature(...)` with distinct scope
//      tags. The same signature feeds both UUIDs, guaranteeing cross-
//      language parity (review-standards.md §2.2 / §2.3).
//   4. `stepId = "<runId>:oai-call:<sig-prefix>"` — short, human-grepable
//      step boundary that survives the substrate's idempotency canonical-
//      key normalization (the prefix is `sig.slice(0,16)`).
//   5. `idempotencyKey` from D05 `deriveIdempotencyKey(...)` with
//      `trigger="LLM_CALL_PRE"` — review-standards.md §2.4 byte-parity.
//   6. `client.reserve({...})`. DENY / STOP / SKIP / APPROVAL → D05 throws
//      typed error → inner NEVER reached (reviewer gate 1.1, 1.3, 1.4).
//      `ApprovalRequired` propagates so the Runner caller can `.resume()`
//      and re-enter the run.
//   7. CONTINUE / DEGRADE → `inner.getResponse(request)`. The request is
//      passed verbatim — DEGRADE mutation application is LOCKED OUT of
//      v0.1.x (design §3 non-goals).
//   8. POST commit via `client.commitEstimated({...})` with
//      `extractUsage(response).totalTokens` → string-coerced for the wire
//      int64 field (review-standards.md §6.4 BudgetClaim shape).
//      `outcomeKind` is omitted at SLICE 2 — `outcome="SUCCESS"` alone
//      suffices for the single-event path the substrate has shipped since
//      D05 SLICE 4. Provider error → POST commit fires with
//      `outcome="PROVIDER_ERROR"`, then the error rethrows.
//   9. Return inner response unchanged (no wrapping, no usage rewrite).
//
// Anti-scope (do NOT add):
//   - stream-per-chunk gating (LOCKED OUT, v0.2 minor — design §3).
//   - DEGRADE mutation patch application (LOCKED OUT — design §3).
//   - browser support (UDS only — D05 §6).
//   - default claimEstimator from `inner.model` (SLICE 3 owns it — the
//     fixture parity gate ports the Python `_default_estimator.MODEL_BASELINE_TOKENS`
//     table; we stub `defaultClaimEstimator` here so SLICE 2 has a safe
//     `[]` projection for the reserve).

import type { Model, ModelRequest, ModelResponse } from "@openai/agents";
import {
  type BudgetClaim,
  type CommitEstimatedRequest,
  type DecisionOutcome,
  type PricingFreeze,
  type ReserveRequest,
  type UnitRef,
  deriveIdempotencyKey,
  deriveUuidFromSignature,
} from "@spendguard/sdk";
import type { SpendGuardAgentsOptions } from "./options.js";
import { currentRunContext } from "./runContext.js";
import { deriveAgentSignature } from "./signature.js";
import { extractUsage } from "./usage.js";

// ── Defaults the SLICE 2 options surface deliberately omits ────────────────
//
// Mirror D04 SLICE 3 / D06 SLICE 3: pick sensible defaults for fields the
// LOCKED options surface does not yet expose so the SLICE 2 wiring is
// end-to-end testable without expanding the public type.

/** Default route label surfaced on `ReserveRequest.route`. */
const DEFAULT_ROUTE = "llm.call";

/** Default budget unit — micro-dollars, the substrate's canonical money unit. */
const DEFAULT_UNIT: UnitRef = { unit: "USD_MICROS", denomination: 1 };

/**
 * Empty pricing freeze. SLICE 2 has no pricing-version visibility on the
 * adapter side; commits ride with a blank freeze and the sidecar's
 * server-side defaults take over. A later slice lifts the consumer-provided
 * `PricingFreeze` onto the options surface (parity with D06 SLICE 3).
 */
const EMPTY_PRICING: PricingFreeze = {
  pricingVersion: "",
  pricingHash: new Uint8Array(0),
};

/** Trigger constant; only LLM_CALL_PRE for SLICE 2's reserve. */
const TRIGGER_PRE = "LLM_CALL_PRE" as const;

/** Scope tags for the substrate `deriveUuidFromSignature(...)` derivations. */
const SCOPE_DECISION_ID = "decision_id";
const SCOPE_LLM_CALL_ID = "llm_call_id";

/**
 * Run the PRE/POST bracket around an inner `Model.getResponse(...)` call.
 *
 * @param inner - The `Model` being wrapped. NEVER invoked before
 *   `client.reserve(...)` resolves CONTINUE / DEGRADE. Reviewer gate 1.1.
 * @param request - The `ModelRequest` the OpenAI Agents Runner has built
 *   for this call. Passed verbatim to `inner.getResponse(...)` — no
 *   field rewriting.
 * @param opts - The SLICE 2 LOCKED options surface — see
 *   `SpendGuardAgentsOptions`.
 * @param innerModelName - Best-effort inner model id read from the
 *   `inner.model` field by the caller. SLICE 2 uses it only for `claims`
 *   projection telemetry / labelling; SLICE 3 will route it through the
 *   default `claimEstimator` derived from the Python parity table.
 */
export async function bracketedGetResponse(
  inner: Model,
  request: ModelRequest,
  opts: SpendGuardAgentsOptions,
  innerModelName: string,
): Promise<ModelResponse> {
  const ctx = currentRunContext();

  const sig = deriveAgentSignature(request.input, request.systemInstructions);
  const llmCallId = deriveUuidFromSignature(sig, { scope: SCOPE_LLM_CALL_ID });
  const decisionId = deriveUuidFromSignature(sig, { scope: SCOPE_DECISION_ID });
  const stepId = `${ctx.runId}:oai-call:${sig.slice(0, 16)}`;

  const idempotencyKey = deriveIdempotencyKey({
    tenantId: opts.tenantId,
    sessionId: ctx.runId,
    runId: ctx.runId,
    stepId,
    llmCallId,
    trigger: TRIGGER_PRE,
  });

  const claims = projectClaimsSlice2(request, opts, innerModelName);
  const req: ReserveRequest = {
    trigger: TRIGGER_PRE,
    runId: ctx.runId,
    stepId,
    llmCallId,
    decisionId,
    route: DEFAULT_ROUTE,
    projectedClaims: claims,
    idempotencyKey,
  };

  // PRE — typed substrate errors propagate UNCHANGED. DENY / STOP / SKIP /
  // APPROVAL → inner.getResponse NEVER reached (reviewer gates 1.1 / 1.3 /
  // 1.4). SidecarUnavailable also propagates here at SLICE 2 — the future
  // `degrade=auto` mode is LOCKED OUT of v0.1.x. The OpenAI Agents Runner
  // caller catches both.
  const outcome: DecisionOutcome = await opts.client.reserve(req);

  // Inner call — request passed verbatim. The Agents SDK has shipped a
  // single-arg `getResponse(request)` since v0.3.0 (the spec's 7-positional
  // signature is from a pre-public draft); modern v0.11.x uses the same
  // single-arg shape, so this stays compatible across the locked peer range.
  let response: ModelResponse;
  let providerError: unknown;
  try {
    response = await inner.getResponse(request);
  } catch (err) {
    providerError = err;
    response = undefined as unknown as ModelResponse;
  }

  // POST — fire commit only when the substrate handed back at least one
  // reservation id. Empty `reservationIds` (no-op DEGRADE / probe outcome)
  // → skip commit, matching the D05 / D06 SLICE 4 commit-skip discipline.
  if (outcome.reservationIds.length > 0) {
    const reservationId = outcome.reservationIds[0] ?? "";
    if (providerError) {
      await safeCommit(opts, {
        runId: ctx.runId,
        stepId,
        llmCallId,
        decisionId: outcome.decisionId,
        reservationId,
        estimatedAmountAtomic: "0",
        unit: DEFAULT_UNIT,
        pricing: EMPTY_PRICING,
        providerEventId: "",
        outcome: "PROVIDER_ERROR",
      });
      throw providerError;
    }

    const usage = extractUsage(response);
    const providerEventId =
      (response as { responseId?: string; requestId?: string } | undefined)?.responseId ??
      (response as { requestId?: string } | undefined)?.requestId ??
      "";

    await safeCommit(opts, {
      runId: ctx.runId,
      stepId,
      llmCallId,
      decisionId: outcome.decisionId,
      reservationId,
      estimatedAmountAtomic: String(usage.totalTokens),
      unit: DEFAULT_UNIT,
      pricing: EMPTY_PRICING,
      providerEventId,
      outcome: "SUCCESS",
    });
  } else if (providerError) {
    // No reservation to commit against, but inner still failed — surface
    // the provider error so the Runner sees it.
    throw providerError;
  }

  return response;
}

/**
 * SLICE 2 placeholder claim projection. Returns a single coarse claim
 * keyed by the budgetId-or-tenantId scope and a `0` amount so the reserve
 * still threads a well-formed claim through the substrate while the
 * authoritative numbers land on the POST commit. SLICE 3 (default
 * estimator + cross-language fixture) replaces this with the Python
 * `_default_estimator.MODEL_BASELINE_TOKENS` table.
 */
function projectClaimsSlice2(
  _request: ModelRequest,
  opts: SpendGuardAgentsOptions,
  _innerModelName: string,
): BudgetClaim[] {
  return [
    {
      scopeId: opts.budgetId ?? opts.tenantId,
      amountAtomic: "0",
      unit: DEFAULT_UNIT,
    },
  ];
}

/**
 * Commit the POST event. Substrate errors on the POST path are warned-and-
 * returned, NOT swallowed silently — review-standards.md §10.2 "no
 * swallowing". The Runner caller has already received (or is about to
 * receive) the inner response, so a commit-side fault must not corrupt the
 * agent run.
 */
async function safeCommit(
  opts: SpendGuardAgentsOptions,
  req: CommitEstimatedRequest,
): Promise<void> {
  try {
    await opts.client.commitEstimated(req);
  } catch (err) {
    const reason = err instanceof Error ? err.message : String(err);
    console.warn(
      `[spendguard:openai-agents] commitEstimated() failed for runId=${req.runId}; ` +
        `inner response preserved (${reason})`,
    );
  }
}
