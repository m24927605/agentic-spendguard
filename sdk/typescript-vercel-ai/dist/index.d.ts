import { SpendGuardClient, PricingFreeze } from '@spendguard/sdk';
export { DecisionDenied, SidecarUnavailable, SpendGuardError } from '@spendguard/sdk';
import { LanguageModelV1Middleware } from 'ai';

/**
 * Constructor options for {@link createSpendGuardMiddleware}.
 *
 * SLICE 2/3 surface (LOCKED) — additional ADDITIVE OPTIONAL fields land in
 * SLICE 4+ when the commit / release paths are wired. Every post-SLICE-3
 * addition is backward-compatible (new optional fields only) so consumers
 * who pin against this minimal shape never break.
 *
 * @example
 * ```ts
 * import { createSpendGuardMiddleware } from "@spendguard/vercel-ai";
 * import { wrapLanguageModel } from "ai";
 * import { openai } from "@ai-sdk/openai";
 *
 * const middleware = createSpendGuardMiddleware({
 *   client,
 *   tenantId: "tenant-prod",
 * });
 * const model = wrapLanguageModel({
 *   model: openai("gpt-4o-mini"),
 *   middleware,
 * });
 * ```
 */
interface SpendGuardMiddlewareOptions {
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
     * Mirrors `pydantic_ai.py::SpendGuardModel.__init__`'s `tenant_id` arg —
     * the adapter does not infer a tenant from the client (the substrate
     * `SpendGuardClient` *does* expose `tenantId`, but D06's design.md §4
     * locks the middleware option as REQUIRED to keep the public surface
     * explicit; cross-tenant misconfiguration is harder to silently mint when
     * the field is mandatory).
     */
    tenantId: string;
    /**
     * Optional budget scope override (UUID) used as the projected claim's
     * `scopeId`. When unset, SLICE 3 falls back to `tenantId` as the scopeId
     * — same default discipline as D04 SLICE 3 / SLICE 5.
     *
     * Production consumers route to a team-specific budget by setting this
     * per middleware instance. The richer `windowInstanceId` / `unit` /
     * `pricing` fields the design.md §4 surface anticipates land in SLICE 4+;
     * see file-level JSDoc for the deferral rationale.
     */
    budgetId?: string;
    /**
     * Canonical-truth UUID of the ledger unit row. When set, threads to
     * `BudgetClaim.unit.unitId` on the wire so the sidecar ledger can
     * resolve the budget claim. Most operators source this from the
     * `SPENDGUARD_UNIT_ID` env var at middleware construction time.
     *
     * Omitting leaves the wire field empty and the ledger will reject the
     * reserve with `INVALID_REQUEST: claim[N].unit.unit_id empty` —
     * recipe-style integrations (no ledger reserve) MAY omit. NB: this is
     * the ledger UUID, distinct from the free-form unit slug — they are
     * NOT interchangeable.
     *
     * Additive optional field shipped under HARDEN_D05_UR (the SDK-side
     * `UnitRef.unitId` broadening landed in SLICE 1; this option threads
     * it through the middleware's `transformParams` reserve path).
     */
    unitId?: string;
    /**
     * Canonical-truth UUID of the ledger window-instance row. When set,
     * threads to `BudgetClaim.window_instance_id` on the wire. Most
     * operators source this from the `SPENDGUARD_WINDOW_INSTANCE_ID` env
     * var at middleware construction time.
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
     * Demo/test-only escape hatch: when set (string-form integer), the
     * projected claim's `amountAtomic` uses this value INSTEAD of the
     * chars/4 heuristic. Mirrors the Python litellm callback's
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
 * Construct a Vercel AI SDK middleware that enforces SpendGuard budget
 * guardrails on every wrapped model call.
 *
 * Compose via `wrapLanguageModel({ model, middleware })`. Every
 * `generateText` / `streamText` invocation flows through:
 *   1. `transformParams` → `client.reserve(LLM_CALL_PRE)` (this slice).
 *   2. `wrapGenerate` → `client.commitEstimated(SUCCESS)` / `release` on
 *      failure (SLICE 4).
 *   3. `wrapStream` → TransformStream-based commit-after-finish (SLICE 5).
 *
 * SLICE 2/3 ships steps (1) only. `wrapGenerate` / `wrapStream` throw a
 * clear "SLICE N not implemented" signal so a consumer who calls into a
 * SLICE-2/3 build of the package gets a pointed error instead of silent
 * skip.
 *
 * @param opts Locked options surface. The minimum required fields are
 *             `client` (a configured `SpendGuardClient`) and `tenantId`
 *             (the tenant the call bills against). `budgetId` is optional
 *             and overrides the default tenant-scoped budget routing.
 *
 * @example
 * ```ts
 * import { createSpendGuardMiddleware } from "@spendguard/vercel-ai";
 * import { wrapLanguageModel, generateText } from "ai";
 * import { openai } from "@ai-sdk/openai";
 *
 * const client = new SpendGuardClient({ ... });
 * await client.connect();
 * await client.handshake();
 *
 * const middleware = createSpendGuardMiddleware({
 *   client,
 *   tenantId: "tenant-prod",
 * });
 * const model = wrapLanguageModel({
 *   model: openai("gpt-4o-mini"),
 *   middleware,
 * });
 * const { text } = await generateText({ model, prompt: "Hello" });
 * ```
 *
 * @throws DecisionDenied (and `DecisionStopped` / `ApprovalRequired`
 *   subclasses) from `transformParams` when the substrate denies the
 *   reserve. The AI SDK caller sees the typed error directly.
 */
declare function createSpendGuardMiddleware(opts: SpendGuardMiddlewareOptions): LanguageModelV1Middleware;

declare const VERSION: "0.2.0";

export { type SpendGuardMiddlewareOptions, VERSION, createSpendGuardMiddleware };
