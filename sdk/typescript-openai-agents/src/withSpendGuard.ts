// `withSpendGuard` — composition factory. Wraps an `@openai/agents` `Model`
// so every `getResponse(request)` runs through the SLICE 2 PRE/POST
// bracket from `./core.ts`.
//
// design.md §7 locked decision #1: composition is the primary surface;
// `SpendGuardAgentsModel` (subclass) is secondary. Both delegate to the
// same `bracketedGetResponse(...)` so no bracket drifts between them
// (reviewer gate 1.2).
//
// The returned object satisfies the `Model` interface contract from
// `@openai/agents` (single-arg `getResponse(request)`, plus the optional
// `getStreamedResponse(request)` async-iterable and `getRetryAdvice(...)`
// hooks). The stream path is **pass-through with no PRE/POST gating**
// (reviewer gate 1.5 — JSDoc + tests verify). Per-chunk gating is the v0.2
// follow-on (design §3 non-goals).

import type { Model, ModelRequest } from "@openai/agents";
import { bracketedGetResponse } from "./core.js";
import type { SpendGuardAgentsOptions } from "./options.js";

/**
 * Wrap an `@openai/agents` `Model` with SpendGuard PRE/POST guardrails.
 *
 * Returns a new `Model` whose `getResponse(request)` calls flow through:
 *
 *   1. `client.reserve({ trigger: "LLM_CALL_PRE", ... })` — built from the
 *      active `runContext()` and a deterministic
 *      `(decisionId, llmCallId)` derived from the request input. DENY /
 *      STOP / SKIP / APPROVAL → typed error → inner NEVER reached.
 *   2. `inner.getResponse(request)` — request passed verbatim.
 *   3. `client.commitEstimated({ outcome: "SUCCESS", ... })` with
 *      `totalTokens` from the inner response usage. Provider error →
 *      commit with `outcome: "PROVIDER_ERROR"` first, then re-throw.
 *
 * Pre-condition: caller MUST be inside an active `runContext()` scope. The
 * adapter throws when called outside one — there is no implicit run id.
 *
 * Pass-through hooks:
 *   - `getStreamedResponse(request)`: pass-through to inner; **NO PRE/POST**
 *     gating in v0.1.x. Per-chunk gating tracked in POST_D08 / v0.2.
 *   - `getRetryAdvice(args)`: delegates to inner when defined; returns
 *     `undefined` otherwise.
 *
 * @param inner - The model to wrap. Typically `OpenAIChatCompletionsModel`
 *   or `OpenAIResponsesModel` from `@openai/agents/openai`, or any
 *   custom-provider `Model` instance.
 * @param opts - Locked options surface — see {@link SpendGuardAgentsOptions}.
 * @returns A `Model`-shaped object suitable for an `Agent({ model })` slot.
 *
 * @throws TypeError when `opts.client` or `opts.tenantId` is missing /
 *   invalid. Throws synchronously at the factory call so misconfiguration
 *   surfaces before the first call.
 *
 * @example
 * ```ts
 * import { Agent, Runner } from "@openai/agents";
 * import { OpenAIChatCompletionsModel } from "@openai/agents/openai";
 * import { withSpendGuard, runContext } from "@spendguard/openai-agents";
 * import { SpendGuardClient, newUuid7 } from "@spendguard/sdk";
 *
 * const client = new SpendGuardClient({ ... });
 * await client.connect();
 * await client.handshake();
 *
 * const inner = new OpenAIChatCompletionsModel({ model: "gpt-4o-mini" });
 * const guarded = withSpendGuard(inner, { client, tenantId: "tenant-prod" });
 * const agent = new Agent({ name: "demo", model: guarded });
 *
 * const runId = newUuid7();
 * await runContext({ runId }, () => Runner.run(agent, "Say hi"));
 * ```
 */
export function withSpendGuard<M extends Model>(inner: M, opts: SpendGuardAgentsOptions): Model {
  validateOpts(opts);
  // `as { model?: string }` — the `Model` interface itself does not declare
  // a `.model` field, but every provider impl shipped under
  // `@openai/agents/openai` carries one. Read defensively; SLICE 3's
  // default estimator routes on this string.
  const innerModelName = (inner as { model?: string }).model ?? "";

  const wrapped: Model = {
    async getResponse(request: ModelRequest) {
      return bracketedGetResponse(inner, request, opts, innerModelName);
    },

    /**
     * Stream pass-through. v0.1.x scope: NO PRE/POST gating around the
     * stream. POST_D08 / v0.2 will add per-chunk gating when the substrate's
     * `LLM_STREAM_DELTA` trigger ships. Documented in reviewer gate 1.5.
     */
    getStreamedResponse(request: ModelRequest) {
      return inner.getStreamedResponse(request);
    },
  };

  // `getRetryAdvice` is optional on the `Model` interface. Forward when the
  // inner model defines it; leave undefined otherwise so the Agents Runner
  // falls back to its built-in retry classifier.
  if (typeof inner.getRetryAdvice === "function") {
    wrapped.getRetryAdvice = inner.getRetryAdvice.bind(inner);
  }

  return wrapped;
}

// ── Internal helpers ───────────────────────────────────────────────────────

function validateOpts(opts: SpendGuardAgentsOptions): void {
  if (opts === null || typeof opts !== "object") {
    throw new TypeError("withSpendGuard: opts must be an object");
  }
  if (!opts.client) {
    throw new TypeError("withSpendGuard: opts.client is required");
  }
  if (typeof opts.tenantId !== "string" || opts.tenantId.length === 0) {
    throw new TypeError("withSpendGuard: opts.tenantId is required (non-empty string)");
  }
}
