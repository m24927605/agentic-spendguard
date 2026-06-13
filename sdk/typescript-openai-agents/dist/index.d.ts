import { Model, ModelRequest, ModelResponse } from '@openai/agents';
import { SpendGuardClient, PricingFreeze } from '@spendguard/sdk';
export { ApprovalRequired, DecisionDenied, DecisionStopped, SidecarUnavailable, SpendGuardError } from '@spendguard/sdk';
export { RunContext, currentRunContext, runContext } from './runContext.js';

/**
 * Locked options surface for {@link withSpendGuard} and
 * {@link SpendGuardAgentsModel}.
 *
 * SLICE 2 surface (LOCKED) — additional ADDITIVE OPTIONAL fields land in
 * SLICE 3+ when the cross-language fixture and the real-demo wiring need
 * them. Every post-SLICE-2 addition is backward-compatible (new optional
 * fields only) so consumers who pin against this minimal shape never break.
 *
 * @example
 * ```ts
 * import { withSpendGuard, runContext } from "@spendguard/openai-agents";
 * import { Agent, Runner } from "@openai/agents";
 * import { SpendGuardClient, newUuid7 } from "@spendguard/sdk";
 *
 * const client = new SpendGuardClient({ ... });
 * await client.connect();
 * await client.handshake();
 *
 * const guarded = withSpendGuard(innerModel, {
 *   client,
 *   tenantId: "tenant-prod",
 * });
 * const agent = new Agent({ name: "demo", model: guarded });
 *
 * const runId = newUuid7();
 * await runContext({ runId }, () => Runner.run(agent, "hello"));
 * ```
 */
interface SpendGuardAgentsOptions {
    /**
     * Configured `SpendGuardClient` instance from `@spendguard/sdk`. The
     * adapter does NOT own the client lifecycle — the consumer constructs it,
     * calls `connect()` / `handshake()`, and is responsible for `close()`.
     */
    client: SpendGuardClient;
    /**
     * Tenant id the call is billed to. Forwarded to the substrate as the
     * `reserve()` claim scope and as the first field of the idempotency-key
     * canonical tuple.
     *
     * Mirrors the D06 vercel-ai middleware's `tenantId` locking discipline —
     * cross-tenant misconfiguration is harder to silently mint when the field
     * is mandatory even though `SpendGuardClient` *does* expose a configured
     * `tenantId` of its own.
     */
    tenantId: string;
    /**
     * Optional budget scope override (UUID) used as the projected claim's
     * `scopeId`. When unset, SLICE 2 falls back to `tenantId` as the scopeId —
     * same default discipline as D04 SLICE 3 / D06 SLICE 3.
     *
     * Production consumers route to a team-specific budget by setting this
     * per adapter instance. The richer `windowInstanceId` / `unit` /
     * `pricing` fields the design.md §4 surface anticipates land in SLICE 4+;
     * see file-level JSDoc for the deferral rationale.
     */
    budgetId?: string;
    /**
     * Canonical-truth UUID of the ledger unit row. When set, threads to
     * `BudgetClaim.unit.unitId` on the wire so the sidecar ledger can
     * resolve the budget claim. Most operators source this from the
     * `SPENDGUARD_UNIT_ID` env var at adapter construction time.
     *
     * Omitting leaves the wire field empty and the ledger will reject the
     * reserve with `INVALID_REQUEST: claim[N].unit.unit_id empty` —
     * recipe-style integrations (no ledger reserve) MAY omit. NB: this is
     * the ledger UUID, distinct from the free-form unit slug — they are
     * NOT interchangeable.
     *
     * Additive optional field shipped under HARDEN_D05_UR (the SDK-side
     * `UnitRef.unitId` broadening landed in SLICE 1; this option threads
     * it through the bracket's reserve path).
     */
    unitId?: string;
    /**
     * Canonical-truth UUID of the ledger window-instance row. When set,
     * threads to `BudgetClaim.window_instance_id` on the wire. Most
     * operators source this from the `SPENDGUARD_WINDOW_INSTANCE_ID` env
     * var at construction time.
     *
     * Omitting leaves the wire field empty and the ledger will reject the
     * reserve with `INVALID_REQUEST: claim[N].window_instance_id empty` —
     * recipe-style integrations (no ledger reserve) MAY omit.
     *
     * Additive optional field shipped under HARDEN_D05_WI (mirror of the
     * HARDEN_D05_UR `unitId` broadening).
     */
    windowInstanceId?: string;
    /**
     * Demo/test-only escape hatch: when set (string-form integer), every
     * projected claim's `amountAtomic` uses this value INSTEAD of the
     * per-model baseline table. Mirrors the Python litellm callback's
     * `spendguard_estimate_override` convention so demo DENY steps can
     * blow past a seeded hard-cap deterministically. Production
     * consumers MUST NOT set this — pricing-table estimation is the
     * supported path.
     */
    estimateOverrideAtomic?: string;
    /**
     * Pricing freeze tuple the commit path repeats back to the ledger.
     * Must match the reservation's freeze (the demo sources it from
     * `SPENDGUARD_PRICING_VERSION` + `SPENDGUARD_PRICE_SNAPSHOT_HASH_HEX`
     * + `SPENDGUARD_FX_RATE_VERSION` + `SPENDGUARD_UNIT_CONVERSION_VERSION`,
     * the same convention as the Python demos). Omitting sends the empty
     * tuple — fine when the ledger's reservation also carries the empty
     * tuple, rejected otherwise. Shipped under HARDEN_D05_WI.
     */
    pricing?: PricingFreeze;
}

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
declare function withSpendGuard<M extends Model>(inner: M, opts: SpendGuardAgentsOptions): Model;

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
declare class SpendGuardAgentsModel implements Model {
    private readonly inner;
    private readonly opts;
    private readonly innerModelName;
    /**
     * Construct a `SpendGuardAgentsModel`. Throws `TypeError` synchronously
     * when `inner` / `opts.client` / `opts.tenantId` are missing — surfaces
     * misconfiguration at construction rather than on the first call.
     */
    constructor(opts: SpendGuardAgentsOptions & {
        inner: Model;
    });
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
    getResponse(request: ModelRequest): Promise<ModelResponse>;
    /**
     * Stream pass-through. v0.1.x scope: NO PRE/POST gating. POST_D08 /
     * v0.2 will land per-chunk gating once the substrate's
     * `LLM_STREAM_DELTA` trigger ships.
     */
    getStreamedResponse(request: ModelRequest): ReturnType<Model["getStreamedResponse"]>;
    /**
     * Forward `getRetryAdvice` to the inner model. The optional retry-advice
     * hook is consulted by the Agents Runner when an LLM call fails; the
     * adapter has no opinion of its own on retry policy at v0.1.x.
     */
    getRetryAdvice(args: Parameters<NonNullable<Model["getRetryAdvice"]>>[0]): ReturnType<NonNullable<Model["getRetryAdvice"]>>;
}

/**
 * Compute the stable hex signature for an OpenAI Agents `ModelRequest`'s
 * `(input, systemInstructions)` pair.
 *
 * @param input - The `ModelRequest.input` field — either a raw string
 *   prompt (older Chat Completions style) or a list of `AgentInputItem`
 *   message objects (Responses API style). Both shapes are supported.
 * @param systemInstructions - The `ModelRequest.systemInstructions` field.
 *   Treated as `""` when `null` or `undefined` so two calls with no system
 *   prompt collapse to the same signature.
 * @returns 32-character lowercase hex string — BLAKE2b output truncated to
 *   16 bytes.
 *
 * @remarks
 * Python parity quirk: for string inputs we render a Python `repr(str)`-style
 * literal. For list-of-message inputs both languages serialize to JSON via
 * the canonical path described in module JSDoc. The cross-language fixture
 * (SLICE 3) gates the agreement.
 */
declare function deriveAgentSignature(input: unknown, systemInstructions: string | null | undefined): string;

/**
 * Extracted token totals from a `ModelResponse`. All fields are numbers in
 * canonical token units. Missing / unparseable usage degrades to `0`.
 */
interface ExtractedUsage {
    inputTokens: number;
    outputTokens: number;
    totalTokens: number;
}
/**
 * Pull canonical token counts from an OpenAI Agents `ModelResponse`.
 *
 * @param response - The response returned by `inner.getResponse(...)`. Only
 *   `.usage` is read; the rest of the response passes through verbatim.
 * @returns `{ inputTokens, outputTokens, totalTokens }` — each safe-zero
 *   on missing or malformed data.
 */
declare function extractUsage(response: ModelResponse | undefined | null): ExtractedUsage;

declare const VERSION: "0.1.0";

export { type ExtractedUsage, SpendGuardAgentsModel, type SpendGuardAgentsOptions, VERSION, deriveAgentSignature, extractUsage, withSpendGuard };
