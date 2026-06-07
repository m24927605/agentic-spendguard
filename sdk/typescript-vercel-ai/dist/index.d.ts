import { SpendGuardClient } from '@spendguard/sdk';
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

declare const VERSION: "0.1.0";

export { type SpendGuardMiddlewareOptions, VERSION, createSpendGuardMiddleware };
