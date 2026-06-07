import { z, RuntimeError, Integration } from '@botpress/sdk';

declare const VERSION = "0.1.0";

/**
 * LOCKED v1 configuration schema. Adding a field is a v0.2 minor; removing
 * one is a v1.0 major.
 *
 * design.md §5 + review-standards.md §2.1:
 *   - `sidecarUrl`               — D09 HTTP companion endpoint URL.
 *   - `spendguardBudgetId`       — UUID of the SpendGuard budget to charge.
 *   - `spendguardWindowInstanceId` — UUID of the SpendGuard window instance.
 *   - `upstreamProvider`         — enum LOCKED at `openai | anthropic | bedrock`.
 *   - `tenantId`                 — operator tenant identifier (overrides
 *                                  the per-bot default; see §5 conversation
 *                                  mapping).
 *   - `tlsCertPath` / `tlsKeyPath` / `tlsRootCaPath` — optional mTLS
 *                                  material paths. Resolved at runtime by
 *                                  `src/reservation.ts` (see §3.3 D09 mTLS
 *                                  contract).
 */
declare const ConfigurationSchema: z.ZodObject<{
    sidecarUrl: z.ZodString;
    spendguardBudgetId: z.ZodString;
    spendguardWindowInstanceId: z.ZodString;
    upstreamProvider: z.ZodEnum<["openai", "anthropic", "bedrock"]>;
    tenantId: z.ZodString;
    tlsCertPath: z.ZodOptional<z.ZodString>;
    tlsKeyPath: z.ZodOptional<z.ZodString>;
    tlsRootCaPath: z.ZodOptional<z.ZodString>;
}, "strip", z.ZodTypeAny, {
    sidecarUrl: string;
    spendguardBudgetId: string;
    spendguardWindowInstanceId: string;
    upstreamProvider: "openai" | "anthropic" | "bedrock";
    tenantId: string;
    tlsCertPath?: string | undefined;
    tlsKeyPath?: string | undefined;
    tlsRootCaPath?: string | undefined;
}, {
    sidecarUrl: string;
    spendguardBudgetId: string;
    spendguardWindowInstanceId: string;
    upstreamProvider: "openai" | "anthropic" | "bedrock";
    tenantId: string;
    tlsCertPath?: string | undefined;
    tlsKeyPath?: string | undefined;
    tlsRootCaPath?: string | undefined;
}>;
type Configuration = z.infer<typeof ConfigurationSchema>;
/**
 * Local config error — kept local rather than re-exported from
 * `@spendguard/sdk` to avoid an unnecessary peer-dep surface widening for a
 * pure-validation concern.
 *
 * The error's `code` mirrors the Botpress `RuntimeError` code that
 * `src/adapter/errors.ts` will translate it to (`BUDGET_CONFIG`), so a
 * defensive consumer catching this directly sees the same code in both
 * directions.
 */
declare class SpendGuardConfigError extends Error {
    readonly code: "BUDGET_CONFIG";
    constructor(message: string);
}

/**
 * Inputs the reservation sees per Botpress AI hook call. Built by
 * `src/adapter/binding.ts` from the Botpress hook input — `data.conversationId`
 * / `ctx.botId` / `data.model` / `data.input.messages` / `data.input.maxTokens`.
 */
interface BotpressCallContext {
    readonly botId: string;
    readonly conversationId: string;
    readonly userId: string;
    readonly model: string;
    readonly messages: ReadonlyArray<{
        role: string;
        content: string;
    }>;
    readonly maxTokens: number;
    /** Optional run identifier (Botpress message id); generated if absent. */
    readonly runId?: string;
}
/**
 * State carried from `reserve` → `commitSuccess` / `releaseFailure` for one
 * AI hook call. Stashed on `data._spendguardHandle` between
 * `beforeAiGeneration` and `afterAiGeneration` (review-standards.md §3.11).
 *
 * Readonly + plain-object so it survives the synchronous stash on the
 * Botpress hook payload object without surprising the runtime's
 * JSON-serialisation pass.
 */
interface ReservationHandle {
    readonly decisionId: string;
    readonly reservationId: string;
    readonly llmCallId: string;
    readonly runId: string;
    readonly stepId: string;
    /** Snapshot of the projected claim used at reserve time. Estimator
     * fallback in afterAiGeneration commits this amount when
     * `event.payload.usage` is missing. */
    readonly estimatorSnapshot: {
        readonly amountAtomic: string;
        readonly inputTokens: number;
        readonly outputTokens: number;
    };
    /** Conversation id for cross-hook correlation in the audit chain. */
    readonly conversationId: string;
}
/** Sidecar returned DENY — fail-closed. Translated to Botpress
 *  `RuntimeError("BUDGET_DENIED")` by `src/adapter/errors.ts`. */
declare class DecisionDenied extends Error {
    readonly code: "BUDGET_DENIED";
    readonly reasonCodes: ReadonlyArray<string>;
    constructor(message: string, reasonCodes?: ReadonlyArray<string>);
}
/** Sidecar returned DEGRADE or transport-level failure — fail-closed by
 *  default. Translated to Botpress `RuntimeError("BUDGET_DEGRADED")`. Dev
 *  escape: `SPENDGUARD_BOTPRESS_FAIL_OPEN=1`. */
declare class SidecarUnavailable extends Error {
    readonly code: "BUDGET_DEGRADED";
    constructor(message: string);
}

/**
 * Reservation / commit / release delegate for the Botpress integration.
 *
 * Composition-only (review-standards.md §2.5 / §3 cross-cutting). The
 * Botpress hook signature lives in `src/hooks/*`; this class owns the
 * SpendGuard lifecycle and is reusable across hooks if a future Botpress
 * SDK exposes additional pre/post slots (review-standards.md §7 reviewer
 * override note).
 */
declare class SpendGuardReservation {
    private readonly cfg;
    private readonly failOpenDev;
    /** Per-instance HTTP client overrides — used by the unit test
     *  `_mockSidecar.ts` to drive the wire path without a real network
     *  socket. Production runtime uses the global `fetch`. */
    private readonly fetchImpl;
    private readonly reserveDeadlineMs;
    private readonly commitDeadlineMs;
    constructor(config: Partial<Configuration>, opts?: {
        readonly fetchImpl?: typeof globalThis.fetch;
        readonly reserveDeadlineMs?: number;
        readonly commitDeadlineMs?: number;
        /** Override the env-var fail-open check (test convenience). */
        readonly failOpenDevOverride?: boolean;
    });
    /**
     * Reserve projected spend with the sidecar.
     *
     * ALLOW → returns `ReservationHandle`; DENY → throws `DecisionDenied`;
     * DEGRADE → throws `SidecarUnavailable` unless dev fail-open is set, in
     * which case returns a sentinel handle (empty `reservationId`) and the
     * commit / release path no-ops to keep the call moving without leaking
     * a phantom reservation row.
     */
    reserve(ctx: BotpressCallContext): Promise<ReservationHandle>;
    /**
     * Commit successful generation with real provider usage. Falls back to
     * the estimator snapshot when `realUsage` is undefined and logs a WARN
     * (INV-5 secondary, design.md §7 question 3).
     */
    commitSuccess(handle: ReservationHandle, realUsage: {
        inputTokens: number;
        outputTokens: number;
    } | undefined, providerEventId: string): Promise<void>;
    /**
     * Release reservation on failure / cancellation. Swallows release-RPC
     * errors (TTL sweep is the durable backstop) but logs a WARN. Classifies
     * cancellation-shaped errors as `CANCELLED` outcome via the same regex
     * pattern as the LiteLLM callback (`_classify_failure`).
     */
    releaseFailure(handle: ReservationHandle, exc: unknown): Promise<void>;
    private warnEstimatorFallback;
    private postJson;
}

/**
 * Minimal Botpress hook input shape the adapter depends on.
 *
 * The real Botpress hook input is `{ ctx, client, data, configuration }`.
 * We only pull from `ctx` (botId) and `data` (everything else). The
 * configuration is threaded separately because it has already been Zod-
 * validated by Botpress before the hook fires.
 */
interface BotpressHookInput {
    ctx: {
        readonly botId: string;
    };
    data: {
        readonly conversationId?: string;
        readonly userId?: string;
        readonly model?: string;
        readonly maxTokens?: number;
        readonly input?: {
            readonly messages?: ReadonlyArray<{
                role?: string;
                content?: string;
            }>;
        };
        /** Sometimes the messages live at the top of `data` rather than
         *  under `data.input`. Both shapes are observed across Botpress 0.7.x
         *  patch versions. The binding code prefers `data.input.messages` and
         *  falls back to `data.messages`. */
        readonly messages?: ReadonlyArray<{
            role?: string;
            content?: string;
        }>;
    };
}
/**
 * Build a `BotpressCallContext` from the Botpress hook input + the
 * operator-supplied configuration. The tenant id falls back to the bot id
 * when `configuration.tenantId` is empty (review-standards.md §3 AD01),
 * but Zod's `min(1)` on `tenantId` means the empty-string path is only
 * reachable via direct (test-only) construction.
 */
declare function toBindingFromHookInput(args: {
    readonly input: BotpressHookInput;
    readonly configuration: Configuration;
}): BotpressCallContext;
/**
 * Convenience function — pick the binding tenant id. Promoted to a named
 * helper because AD01 / AD02 test it as a unit and the precedence rule is
 * load-bearing. (Configuration `tenantId` is Zod-validated as non-empty in
 * production, but the binding layer remains tolerant of synthetic test
 * configurations.)
 */
declare function pickTenantId(configuration: Configuration, botId: string): string;

/** Loose-shape Botpress afterAiGeneration `data` we extract usage from. */
interface BotpressAfterHookData {
    /** Botpress 0.7 normalised shape. */
    readonly payload?: {
        readonly usage?: {
            readonly inputTokens?: number;
            readonly outputTokens?: number;
        };
    };
    /** Some 0.7.x patch versions emit usage at the top of `data`. */
    readonly usage?: {
        readonly inputTokens?: number;
        readonly outputTokens?: number;
    };
    /** Upstream-event-style payload occasionally surfaces the provider's
     *  raw response under `data.response.usage`. We sniff this last so the
     *  per-provider fallback path stays additive. */
    readonly response?: {
        readonly usage?: {
            readonly input_tokens?: number;
            readonly output_tokens?: number;
            readonly prompt_tokens?: number;
            readonly completion_tokens?: number;
        };
    };
    /** Provider event id for dedup at canonical ingest. */
    readonly providerEventId?: string;
}
/**
 * Extract `{ inputTokens, outputTokens }` from Botpress's afterAiGeneration
 * data. Returns `undefined` when none of the recognised shapes is present.
 */
declare function extractUsageFromBotpressEvent(data: BotpressAfterHookData | undefined): {
    inputTokens: number;
    outputTokens: number;
} | undefined;
/** Convenience: snapshot → usage shape for the estimator fallback path
 *  (review-standards.md §3.10 / INV-5 secondary). */
declare function snapshotToUsage(snapshot: ReservationHandle["estimatorSnapshot"]): {
    inputTokens: number;
    outputTokens: number;
};
/** Convenience: pull `providerEventId` from the loose afterAiGeneration
 *  data. Empty string is the wire-stable "missing" sentinel that lines
 *  up with the Kong-shaped trace payload's `provider_event_id` field. */
declare function pickProviderEventId(data: BotpressAfterHookData | undefined): string;

/**
 * Translate any SpendGuard-flavoured error to a Botpress `RuntimeError`
 * carrying a stable `code` field. Unrecognised inputs flow through as a
 * `BUDGET_CONFIG` runtime error with the original message preserved — this
 * mirrors the Python LiteLLM callback's "unknown-error-is-config" fallback
 * (sdk/python/src/spendguard/integrations/litellm.py:806-820).
 */
declare function toRuntimeError(err: unknown): RuntimeError;
/**
 * Inspect the `code` a runtime error carries, given a SpendGuard-typed
 * input. Lets the unit tests assert AD04-AD06 without depending on Botpress's
 * internal RuntimeError shape.
 */
declare function codeFor(err: DecisionDenied | SidecarUnavailable | SpendGuardConfigError): "BUDGET_DENIED" | "BUDGET_DEGRADED" | "BUDGET_CONFIG";

/** Mutable handle stash for the afterAiGeneration cross-hook handoff
 *  (review-standards.md §3.11 / §3.12). `data._spendguardHandle` is the
 *  only field we add to the Botpress hook payload object. */
interface SpendGuardHandleStash {
    _spendguardHandle?: ReservationHandle;
}
interface BeforeAiHookArgs {
    readonly input: BotpressHookInput;
    readonly configuration: Configuration;
    /** Optional reservation override — used by the unit tests to inject
     *  a `SpendGuardReservation` configured with a mock sidecar fetch. */
    readonly reservationOverride?: SpendGuardReservation;
}
/**
 * Execute the beforeAiGeneration logic with a typed signature. The
 * top-level Botpress hook wires this into the `new Integration({...hooks})`
 * registration in `src/index.ts`.
 *
 * @throws RuntimeError on DENY / DEGRADE / config error.
 */
declare function runBeforeAiGeneration(args: BeforeAiHookArgs): Promise<{
    data: BotpressHookInput["data"] & SpendGuardHandleStash;
}>;

interface AfterAiHookArgs {
    readonly input: BotpressHookInput & {
        readonly data: BotpressHookInput["data"] & SpendGuardHandleStash & {
            readonly _cancelled?: boolean;
        };
    };
    readonly configuration: Configuration;
    /** Optional reservation override for unit tests. */
    readonly reservationOverride?: SpendGuardReservation;
}
declare function runAfterAiGeneration(args: AfterAiHookArgs): Promise<{
    data: BotpressHookInput["data"] & SpendGuardHandleStash;
}>;

interface ValidateArgs {
    readonly configuration: Configuration;
    /** Override the reservation used for the probe — unit tests inject a
     *  mock-sidecar-bound reservation here. */
    readonly reservationOverride?: SpendGuardReservation;
}
/**
 * Issue a 1-token reserve + release roundtrip against the sidecar. On
 * success, returns silently. On any failure (DENY / DEGRADE / config /
 * transport), throws a translated Botpress `RuntimeError`.
 */
declare function validateConfiguration(args: ValidateArgs): Promise<void>;

declare const _default: Integration<{
    sidecarUrl: string;
    spendguardBudgetId: string;
    spendguardWindowInstanceId: string;
    upstreamProvider: "openai" | "anthropic" | "bedrock";
    tenantId: string;
    tlsCertPath?: string | undefined;
    tlsKeyPath?: string | undefined;
    tlsRootCaPath?: string | undefined;
}>;

export { type BotpressCallContext, type BotpressHookInput, type Configuration, ConfigurationSchema, DecisionDenied, type ReservationHandle, SidecarUnavailable, SpendGuardConfigError, type SpendGuardHandleStash, SpendGuardReservation, VERSION, codeFor, _default as default, extractUsageFromBotpressEvent, pickProviderEventId, pickTenantId, runAfterAiGeneration, runBeforeAiGeneration, snapshotToUsage, toBindingFromHookInput, toRuntimeError, validateConfiguration };
