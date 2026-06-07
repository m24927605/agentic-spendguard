// `SpendGuardAgentsModel` — class form of the SpendGuard wrapper. Sibling
// to `withSpendGuard(...)` per design.md §7 locked decision #1 (composition
// primary, subclass secondary). Both delegate to the same
// `bracketedGetResponse(...)` shared core so the bracket NEVER drifts
// between the two surfaces — reviewer gate 1.2.
//
// Some consumers (factories that need an `instanceof` check, or codebases
// that prefer to subclass-extend with additional fields like a request-id
// tracker) reach for the subclass form. Behaviour parity is enforced by
// the slice-2 test suite — every test that exercises the factory ALLOW /
// DENY / streaming-passthrough path has a sibling that exercises the
// subclass.

import type { Model, ModelRequest, ModelResponse } from "@openai/agents";
import { bracketedGetResponse } from "./core.js";
import type { SpendGuardAgentsOptions } from "./options.js";

/**
 * Class form of {@link withSpendGuard}. Implements `@openai/agents`'s
 * `Model` interface and runs every `getResponse(request)` through the
 * SLICE 2 PRE/POST bracket from `./core.ts`.
 *
 * Prefer {@link withSpendGuard} for new code (composition is the primary
 * surface); the subclass form exists for codebases that prefer subclass
 * factories or need an `instanceof` check.
 *
 * @example
 * ```ts
 * import { Agent, Runner } from "@openai/agents";
 * import { OpenAIChatCompletionsModel } from "@openai/agents/openai";
 * import { SpendGuardAgentsModel, runContext } from "@spendguard/openai-agents";
 *
 * const inner = new OpenAIChatCompletionsModel({ model: "gpt-4o-mini" });
 * const guarded = new SpendGuardAgentsModel({
 *   inner,
 *   client,
 *   tenantId: "tenant-prod",
 * });
 * const agent = new Agent({ name: "demo", model: guarded });
 * ```
 */
export class SpendGuardAgentsModel implements Model {
  private readonly inner: Model;
  private readonly opts: SpendGuardAgentsOptions;
  private readonly innerModelName: string;

  /**
   * Construct a `SpendGuardAgentsModel`. Throws `TypeError` synchronously
   * when `inner` / `opts.client` / `opts.tenantId` are missing — surfaces
   * misconfiguration at construction rather than on the first call.
   */
  constructor(opts: SpendGuardAgentsOptions & { inner: Model }) {
    if (opts === null || typeof opts !== "object") {
      throw new TypeError("SpendGuardAgentsModel: opts must be an object");
    }
    if (!opts.inner) {
      throw new TypeError("SpendGuardAgentsModel: opts.inner is required");
    }
    if (!opts.client) {
      throw new TypeError("SpendGuardAgentsModel: opts.client is required");
    }
    if (typeof opts.tenantId !== "string" || opts.tenantId.length === 0) {
      throw new TypeError("SpendGuardAgentsModel: opts.tenantId is required (non-empty string)");
    }
    this.inner = opts.inner;
    // Strip the `inner` field off the options the bracket sees; only the
    // factory shape (client / tenantId / budgetId) flows through.
    const { inner: _strip, ...rest } = opts;
    this.opts = rest;
    this.innerModelName = (opts.inner as { model?: string }).model ?? "";
  }

  /**
   * Run the PRE/POST bracket around the inner model's `getResponse(...)`.
   *
   * @throws DecisionDenied / DecisionStopped / ApprovalRequired on a
   *   non-CONTINUE substrate outcome — `inner.getResponse` is NEVER
   *   invoked. Caller may `.resume()` on `ApprovalRequired`.
   * @throws SidecarUnavailable when the sidecar is unreachable — the
   *   adapter does NOT swallow this at v0.1.x; the Runner caller decides
   *   whether to halt or treat the outage as a degrade.
   */
  async getResponse(request: ModelRequest): Promise<ModelResponse> {
    return bracketedGetResponse(this.inner, request, this.opts, this.innerModelName);
  }

  /**
   * Stream pass-through. v0.1.x scope: NO PRE/POST gating. POST_D08 /
   * v0.2 will land per-chunk gating once the substrate's
   * `LLM_STREAM_DELTA` trigger ships.
   */
  getStreamedResponse(request: ModelRequest): ReturnType<Model["getStreamedResponse"]> {
    return this.inner.getStreamedResponse(request);
  }

  /**
   * Forward `getRetryAdvice` to the inner model. The optional retry-advice
   * hook is consulted by the Agents Runner when an LLM call fails; the
   * adapter has no opinion of its own on retry policy at v0.1.x.
   */
  getRetryAdvice(
    args: Parameters<NonNullable<Model["getRetryAdvice"]>>[0],
  ): ReturnType<NonNullable<Model["getRetryAdvice"]>> {
    if (typeof this.inner.getRetryAdvice === "function") {
      return this.inner.getRetryAdvice(args);
    }
    return undefined;
  }
}
