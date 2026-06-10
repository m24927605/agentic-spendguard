import { BaseCallbackHandler } from '@langchain/core/callbacks/base';
import { Serialized } from '@langchain/core/load/serializable';
import { BaseMessage } from '@langchain/core/messages';
import { LLMResult } from '@langchain/core/outputs';
import { SpendGuardClient, PricingFreeze } from '@spendguard/sdk';
export { DecisionDenied, SidecarUnavailable, SpendGuardError } from '@spendguard/sdk';

/**
 * Constructor options for {@link SpendGuardCallbackHandler}.
 *
 * SLICE 2 surface (LOCKED) — additional ADDITIVE OPTIONAL fields land in
 * SLICE 3+ when `reserve` / `commitEstimated` are wired. Every post-SLICE-2
 * addition is backward-compatible (new optional fields only) so the
 * SLICE 2 type lock holds.
 *
 * SLICE 5 deviation #1 (scope-routing only): added optional `budgetId` so
 * demo + production consumers can pin the projected claim's `scopeId` to a
 * specific budget UUID without subclassing the handler. The fuller
 * `unitId` / `windowInstanceId` / `pricing` / `claimEstimator` surface
 * design.md §4 anticipates remains deferred — the TS SDK substrate's
 * public `UnitRef` does not currently expose `unit_id` (`sdk/typescript/
 * src/client.ts::mapUnitRef` hardcodes empty), so a unit override would
 * be dead code today. The next D04 hardening slice picks up the
 * SDK-side broadening + adapter wire-through together.
 */
interface SpendGuardCallbackHandlerOptions {
    /**
     * Configured `SpendGuardClient` instance from `@spendguard/sdk`. The
     * adapter does NOT own the client lifecycle — the consumer constructs it,
     * calls `connect()` / `handshake()`, and is responsible for `close()`.
     */
    client: SpendGuardClient;
    /**
     * Optional tenant override forwarded to the substrate when set. Defaults
     * to whatever tenant the `client` was configured with at construction
     * time. SLICE 3 surfaces it on the `reserve` path.
     */
    tenantId?: string;
    /**
     * Optional default budget cap in atomic micros (USD micros if `unit` is
     * `USD_MICROS`). Used by SLICE 3's fallback `claimEstimator` when the
     * consumer does not provide a custom estimator. `bigint` to avoid the
     * Number.MAX_SAFE_INTEGER cliff at $9.007e9.
     */
    defaultBudgetMicrosCap?: bigint;
    /**
     * Optional budget ID (UUID) used as the projected claim's `scopeId`.
     * When unset, the handler falls back to `tenantId` as the scopeId
     * (SLICE 3 default). Production consumers route to the right
     * team-budget by setting this per handler instance.
     *
     * Additive optional field, SLICE 5 deviation #1 (scope-routing only;
     * see interface JSDoc above for the deferred `unitId` /
     * `windowInstanceId` / pricing surface scope).
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
     * it through the adapter's reserve path).
     */
    unitId?: string;
    /**
     * Canonical-truth UUID of the ledger window-instance row. When set,
     * threads to `BudgetClaim.window_instance_id` on the wire. Most
     * operators source this from the `SPENDGUARD_WINDOW_INSTANCE_ID` env
     * var at handler construction time.
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
 * SpendGuard adapter for LangChain.js.
 *
 * Drop-in via `callbacks: [handler]` on any `BaseChatModel`. SLICE 3 wires
 * the LangChain-protocol hooks against `SpendGuardClient.reserve` /
 * `commitEstimated`; SLICE 4 covers mock-sidecar tests, SLICE 5 ships the
 * runnable demo, SLICE 6 publishes the docs page.
 *
 * @example
 * ```ts
 * import { ChatOpenAI } from "@langchain/openai";
 * import { SpendGuardClient } from "@spendguard/sdk";
 * import { SpendGuardCallbackHandler } from "@spendguard/langchain";
 *
 * const client = new SpendGuardClient({ ... });
 * await client.connect();
 * await client.handshake();
 *
 * const handler = new SpendGuardCallbackHandler({ client });
 * await new ChatOpenAI({ model: "gpt-4o-mini", callbacks: [handler] })
 *   .invoke("hello");
 * ```
 */
declare class SpendGuardCallbackHandler extends BaseCallbackHandler {
    /**
     * Stable serialization name. Matches LangChain.js snake_case handler
     * convention (`tracer_langchain`, `langfuse_handler`, …).
     */
    name: string;
    /**
     * `raiseError = true` — review-standards.md §1.3 P0 LOCK. Without this, a
     * throw from `handleChatModelStart` is swallowed by `CallbackManager`
     * before it can halt `model.invoke()`; the budget gate would never block
     * the LLM call.
     *
     * `awaitHandlers = true` — review-standards.md §1.3 + @langchain/core
     * `base.js:118-120`: setting `raiseError` already forces awaiting (the
     * core code does `awaitHandlers = raiseError || ...`), but pinning it
     * explicitly here defends against future @langchain/core drift.
     */
    raiseError: boolean;
    awaitHandlers: boolean;
    /** Substrate client handed in by the consumer; never mutated. */
    private readonly client;
    /** Consumer-supplied options snapshot; treated as immutable. */
    private readonly opts;
    /**
     * PRE → POST correlation Map keyed by LangChain's `runId`. Written by
     * `handleChatModelStart`, read + deleted by `handleLLMEnd` /
     * `handleLLMError`.
     */
    private readonly inflight;
    constructor(options: SpendGuardCallbackHandlerOptions);
    /**
     * Resolve the tenant id that goes onto reserve/commit requests. Consumer
     * override on the handler wins over the client's configured tenant.
     */
    private get effectiveTenantId();
    /**
     * Build a coarse pre-call `BudgetClaim` from the chat messages. The number
     * is intentionally a heuristic — the substrate cares that the claim shape
     * is well-formed; the authoritative spend lands on the POST commit.
     */
    private projectClaim;
    /**
     * SLICE 3 wires `handleChatModelStart` against `client.reserve()`.
     *
     * Idempotency key is derived from `(tenantId, runId, parentRunId)` via
     * `./ids.ts:deriveIdempotencyKey`. On a `DecisionDenied` (or subclass —
     * `DecisionStopped`, `ApprovalRequired`) the error rethrows so the
     * LangChain `RunManager` propagates it through `model.invoke()`. On any
     * other substrate error (notably `SidecarUnavailable`) the handler logs
     * and returns without stashing inflight — the LLM call proceeds without
     * a budget gate, per the "operational degradation, not enforcement"
     * stance in design.md §3.6.
     *
     * @throws DecisionDenied (and subclasses) — propagates through
     *   `model.invoke()` and halts the run.
     */
    handleChatModelStart(_llm: Serialized, messages: BaseMessage[][], runId: string, parentRunId?: string, _extraParams?: Record<string, unknown>, _tags?: string[], metadata?: Record<string, unknown>, name?: string): Promise<void>;
    /**
     * SLICE 3 wires `handleLLMEnd` against `client.commitEstimated()`.
     *
     * Reads the inflight `(decisionId, reservationId)` keyed by `runId`,
     * deletes the entry, extracts the provider's reported
     * `(promptTokens, completionTokens)` from `output.llmOutput.tokenUsage`,
     * and emits a SUCCESS commit. Both LangChain-canonical camelCase
     * (`promptTokens`) and OpenAI-passthrough snake_case (`prompt_tokens`)
     * shapes are accepted — review-standards §9 cross-language parity.
     *
     * A missing inflight entry is a warn-and-return (review-standards §3.11)
     * — covers the substrate-degradation case where `reserve` failed and the
     * matching POST is just an idempotent re-delivery.
     */
    handleLLMEnd(output: LLMResult, runId: string, _parentRunId?: string, _tags?: string[]): Promise<void>;
    /**
     * SLICE 3 wires `handleLLMError` against `client.commitEstimated()` with
     * the PROVIDER_ERROR / FAILURE outcome shape. Mirrors `handleLLMEnd`'s
     * inflight-lookup discipline; the error's `.message` is threaded onto
     * `actualErrorMessage` so the substrate's outcome event carries the
     * provider's failure reason.
     */
    handleLLMError(err: Error, runId: string, _parentRunId?: string, _tags?: string[]): Promise<void>;
    /**
     * HARDEN_D05_WI — `client.commitEstimated(...)` wrapper that warns on
     * substrate failures so commit-side errors NEVER bubble back to the
     * consumer. The LLM call result has already been delivered (SUCCESS
     * path) or the original provider error is already propagating (FAILURE
     * path) — a commit-side throw at this point (with `raiseError = true`)
     * would corrupt that surface with an unrelated error. Sidecar TTL
     * reconciles any orphaned reservation via the audit chain. Mirrors the
     * vercel-ai / openai-agents `safeCommit` convention.
     */
    private safeCommit;
}

declare const VERSION: "0.1.0-pre";

export { SpendGuardCallbackHandler, type SpendGuardCallbackHandlerOptions, VERSION };
