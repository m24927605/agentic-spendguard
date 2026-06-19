import * as sdk from '@botpress/sdk';
import { z, RuntimeError } from '@botpress/sdk';

type Configuration$1 = {
    /** HTTPS companion URL (plaintext http:// allowed only for loopback) */
    sidecarUrl: string;
    /** UUID of the SpendGuard budget to charge */
    spendguardBudgetId: string;
    /** UUID of the SpendGuard window instance */
    spendguardWindowInstanceId: string;
    /** Upstream provider Botpress dispatches to */
    upstreamProvider: "openai" | "anthropic" | "bedrock";
    /** Operator tenant identifier */
    tenantId: string;
    /** Path to SVID cert PEM */
    tlsCertPath?: string;
    /** Path to SVID key PEM */
    tlsKeyPath?: string;
    /** Path to sidecar CA PEM */
    tlsRootCaPath?: string;
};

type Configurations = {};

type Input$1 = {
    /** Model to use; defaults to the first listed model */
    model?: {
        /** Provider-qualified model id, e.g. openai:gpt-4o-mini */
        id: string;
        /** Human-facing model name */
        name: string;
    };
    /** Prompt messages */
    messages: Array<{
        /** Message role */
        role: "system" | "user" | "assistant" | "tool";
        /** Message text content */
        content: string;
    }>;
    /** Optional system prompt */
    systemPrompt?: string;
    /** Operator-declared output cap; drives the SpendGuard reserve estimate */
    maxTokens?: number;
    /** Sampling temperature */
    temperature?: number;
    /** Nucleus sampling cutoff */
    topP?: number;
    /** Stop sequences */
    stopSequences?: string[];
    /** Opaque end-user id forwarded upstream */
    userId?: string;
};

type Output$1 = {
    /** Provider response id */
    id: string;
    /** Upstream provider that served the call */
    provider: string;
    /** Model id that served the call */
    model: string;
    /** Generated choices */
    choices: Array<{
        /** Always assistant for generated content */
        role: "assistant";
        /** Content type — text only in v1 */
        type: "text";
        /** Generated text */
        content: string;
        /** Choice index */
        index: number;
        /** Why generation stopped */
        stopReason: "stop" | "max_tokens" | "content_filter" | "other";
    }>;
    /** Real token usage committed to SpendGuard */
    usage: {
        /** Prompt / input token count */
        inputTokens: number;
        /** Completion / output token count */
        outputTokens: number;
    };
    /** Botpress billing envelope */
    botpress: {
        /** Cost in USD as reported to Botpress billing */
        cost: number;
    };
};

type GenerateContent = {
    "input": Input$1;
    "output": Output$1;
};

type Input = {};

type Output = {
    /** Models this integration can route to */
    models: Array<{
        /** Provider-qualified model id, e.g. openai:gpt-4o-mini */
        id: string;
        /** Human-facing model name */
        name: string;
    }>;
};

type ListLanguageModels = {
    "input": Input;
    "output": Output;
};

type Actions = {
    "generateContent": GenerateContent;
    "listLanguageModels": ListLanguageModels;
};

type Channels = {};

type Events = {};

type States = {};

type ModelRef$1 = {
    /** Provider-qualified model id, e.g. openai:gpt-4o-mini */
    id: string;
    /** Human-facing model name */
    name: string;
};

type Entities = {
    "modelRef": ModelRef$1;
};

type TIntegration$1 = {
    name: "spendguard";
    version: "0.1.0";
    user: {
        "tags": {};
        "creation": {
            "enabled": false;
            "requiredTags": [];
        };
    };
    configuration: Configuration$1;
    configurations: Configurations;
    actions: Actions;
    channels: Channels;
    events: Events;
    states: States;
    entities: Entities;
};

type TIntegration = sdk.DefaultIntegration<TIntegration$1>;
declare class Integration extends sdk.Integration<TIntegration> {
}

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
/**
 * Base object schema (no cross-field refinements). Botpress's
 * `IntegrationDefinition` config + action schemas must be a plain
 * `ZodObject` / `ZodRecord` (`z.ZuiObjectSchema`); a `.refine()` produces a
 * `ZodEffects`, which the definition's `SchemaDefinition` constraint rejects.
 *
 * `integration.definition.ts` consumes THIS schema so `bp build` codegen sees
 * a well-typed config object. The cross-field invariants (transport-security +
 * all-or-none mTLS) are re-enforced fail-closed at runtime by
 * `assertRequiredConfig` inside `SpendGuardReservation`'s constructor, so the
 * deeper checks are never skipped on the live path.
 */
declare const ConfigurationObjectSchema: z.ZodObject<{
    sidecarUrl: z.ZodEffects<z.ZodString, string, string>;
    spendguardBudgetId: z.ZodString;
    spendguardWindowInstanceId: z.ZodString;
    upstreamProvider: z.ZodEnum<["openai", "anthropic", "bedrock"]>;
    tenantId: z.ZodString;
    tlsCertPath: z.ZodOptional<z.ZodString>;
    tlsKeyPath: z.ZodOptional<z.ZodString>;
    tlsRootCaPath: z.ZodOptional<z.ZodString>;
}, "strip", {
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
declare const ConfigurationSchema: z.ZodEffects<z.ZodObject<{
    sidecarUrl: z.ZodEffects<z.ZodString, string, string>;
    spendguardBudgetId: z.ZodString;
    spendguardWindowInstanceId: z.ZodString;
    upstreamProvider: z.ZodEnum<["openai", "anthropic", "bedrock"]>;
    tenantId: z.ZodString;
    tlsCertPath: z.ZodOptional<z.ZodString>;
    tlsKeyPath: z.ZodOptional<z.ZodString>;
    tlsRootCaPath: z.ZodOptional<z.ZodString>;
}, "strip", {
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
}>, {
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
 * Inputs the reservation sees per `generateContent` action call. Built by
 * `src/adapter/binding.ts` from the action input + handler ctx — `ctx.botId` /
 * `input.model` / `input.messages` / `input.systemPrompt` / `input.maxTokens`.
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
 * State carried from `reserve` → `commitSuccess` / `releaseFailure` within one
 * `generateContent` action invocation. Held in a local variable across the
 * reserve -> forward -> commit sequence (src/llm/generateContent.ts); there is
 * no cross-call stash — the whole lifecycle runs inside a single action call.
 *
 * Readonly + plain-object for cheap structural equality in tests.
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
    /** Whether a caller injected a transport (`fetchImpl`). When true the
     *  caller owns TLS and the mTLS dispatcher is not built — this is the
     *  test/seam path (mirrors the Kong Go plugin's `httpStubClient` bypass). */
    private readonly transportInjected;
    /** Resolved + traversal-cleaned mTLS PEM paths, or null when no client
     *  certificate is configured. */
    private readonly tlsPaths;
    /** Memoised undici mTLS dispatcher promise. Built lazily on first request
     *  so a missing `undici` fails the call (fail closed) rather than the whole
     *  process at import time. */
    private dispatcherPromise;
    constructor(config: Partial<Configuration>, opts?: {
        readonly fetchImpl?: typeof globalThis.fetch;
        readonly reserveDeadlineMs?: number;
        readonly commitDeadlineMs?: number;
        /** Override the env-var fail-open check (test convenience). */
        readonly failOpenDevOverride?: boolean;
    });
    /**
     * Build (and memoise) the undici mTLS dispatcher from the configured PEM
     * material. FAIL CLOSED: if `undici` cannot be loaded, the returned promise
     * rejects, so the dependent `postJson` throws and the reserve/commit path
     * fails closed rather than dialing without a client certificate.
     */
    private mtlsDispatcher;
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

/** A reference to one upstream model the integration exposes. Mirrors the
 *  llm interface `modelRef` entity (id + human-facing name). */
declare const ModelRefSchema: z.ZodObject<{
    id: z.ZodString;
    name: z.ZodString;
}, "strip", {
    id: string;
    name: string;
}, {
    id: string;
    name: string;
}>;
type ModelRef = z.infer<typeof ModelRefSchema>;
/** One message in the prompt. Botpress normalises tool/assistant/user roles
 *  into this shape before invoking generateContent. `content` is the text
 *  the SpendGuard prompt-hash is computed over. */
declare const MessageSchema: z.ZodObject<{
    role: z.ZodEnum<["system", "user", "assistant", "tool"]>;
    content: z.ZodString;
}, "strip", {
    role: "system" | "user" | "assistant" | "tool";
    content: string;
}, {
    role: "system" | "user" | "assistant" | "tool";
    content: string;
}>;
type Message = z.infer<typeof MessageSchema>;
/** Per-call token usage echoed back from the upstream provider. */
declare const UsageSchema: z.ZodObject<{
    inputTokens: z.ZodNumber;
    outputTokens: z.ZodNumber;
}, "strip", {
    inputTokens: number;
    outputTokens: number;
}, {
    inputTokens: number;
    outputTokens: number;
}>;
type Usage = z.infer<typeof UsageSchema>;
/** One generated choice. */
declare const ChoiceSchema: z.ZodObject<{
    role: z.ZodLiteral<"assistant">;
    type: z.ZodLiteral<"text">;
    content: z.ZodString;
    index: z.ZodNumber;
    stopReason: z.ZodEnum<["stop", "max_tokens", "content_filter", "other"]>;
}, "strip", {
    role: "assistant";
    content: string;
    type: "text";
    index: number;
    stopReason: "stop" | "max_tokens" | "content_filter" | "other";
}, {
    role: "assistant";
    content: string;
    type: "text";
    index: number;
    stopReason: "stop" | "max_tokens" | "content_filter" | "other";
}>;
type Choice = z.infer<typeof ChoiceSchema>;
declare const GenerateContentInputSchema: z.ZodObject<{
    model: z.ZodOptional<z.ZodObject<{
        id: z.ZodString;
        name: z.ZodString;
    }, "strip", {
        id: string;
        name: string;
    }, {
        id: string;
        name: string;
    }>>;
    messages: z.ZodArray<z.ZodObject<{
        role: z.ZodEnum<["system", "user", "assistant", "tool"]>;
        content: z.ZodString;
    }, "strip", {
        role: "system" | "user" | "assistant" | "tool";
        content: string;
    }, {
        role: "system" | "user" | "assistant" | "tool";
        content: string;
    }>, "many">;
    systemPrompt: z.ZodOptional<z.ZodString>;
    maxTokens: z.ZodOptional<z.ZodNumber>;
    temperature: z.ZodOptional<z.ZodNumber>;
    topP: z.ZodOptional<z.ZodNumber>;
    stopSequences: z.ZodOptional<z.ZodArray<z.ZodString, "many">>;
    userId: z.ZodOptional<z.ZodString>;
}, "strip", {
    messages: {
        role: "system" | "user" | "assistant" | "tool";
        content: string;
    }[];
    model?: {
        id: string;
        name: string;
    } | undefined;
    systemPrompt?: string | undefined;
    maxTokens?: number | undefined;
    temperature?: number | undefined;
    topP?: number | undefined;
    stopSequences?: string[] | undefined;
    userId?: string | undefined;
}, {
    messages: {
        role: "system" | "user" | "assistant" | "tool";
        content: string;
    }[];
    model?: {
        id: string;
        name: string;
    } | undefined;
    systemPrompt?: string | undefined;
    maxTokens?: number | undefined;
    temperature?: number | undefined;
    topP?: number | undefined;
    stopSequences?: string[] | undefined;
    userId?: string | undefined;
}>;
type GenerateContentInput = z.infer<typeof GenerateContentInputSchema>;
declare const GenerateContentOutputSchema: z.ZodObject<{
    id: z.ZodString;
    provider: z.ZodString;
    model: z.ZodString;
    choices: z.ZodArray<z.ZodObject<{
        role: z.ZodLiteral<"assistant">;
        type: z.ZodLiteral<"text">;
        content: z.ZodString;
        index: z.ZodNumber;
        stopReason: z.ZodEnum<["stop", "max_tokens", "content_filter", "other"]>;
    }, "strip", {
        role: "assistant";
        content: string;
        type: "text";
        index: number;
        stopReason: "stop" | "max_tokens" | "content_filter" | "other";
    }, {
        role: "assistant";
        content: string;
        type: "text";
        index: number;
        stopReason: "stop" | "max_tokens" | "content_filter" | "other";
    }>, "many">;
    usage: z.ZodObject<{
        inputTokens: z.ZodNumber;
        outputTokens: z.ZodNumber;
    }, "strip", {
        inputTokens: number;
        outputTokens: number;
    }, {
        inputTokens: number;
        outputTokens: number;
    }>;
    botpress: z.ZodObject<{
        cost: z.ZodNumber;
    }, "strip", {
        cost: number;
    }, {
        cost: number;
    }>;
}, "strip", {
    id: string;
    model: string;
    provider: string;
    choices: {
        role: "assistant";
        content: string;
        type: "text";
        index: number;
        stopReason: "stop" | "max_tokens" | "content_filter" | "other";
    }[];
    usage: {
        inputTokens: number;
        outputTokens: number;
    };
    botpress: {
        cost: number;
    };
}, {
    id: string;
    model: string;
    provider: string;
    choices: {
        role: "assistant";
        content: string;
        type: "text";
        index: number;
        stopReason: "stop" | "max_tokens" | "content_filter" | "other";
    }[];
    usage: {
        inputTokens: number;
        outputTokens: number;
    };
    botpress: {
        cost: number;
    };
}>;
type GenerateContentOutput = z.infer<typeof GenerateContentOutputSchema>;
declare const ListLanguageModelsInputSchema: z.ZodObject<{}, "strip", {}, {}>;
type ListLanguageModelsInput = z.infer<typeof ListLanguageModelsInputSchema>;
declare const ListLanguageModelsOutputSchema: z.ZodObject<{
    models: z.ZodArray<z.ZodObject<{
        id: z.ZodString;
        name: z.ZodString;
    }, "strip", {
        id: string;
        name: string;
    }, {
        id: string;
        name: string;
    }>, "many">;
}, "strip", {
    models: {
        id: string;
        name: string;
    }[];
}, {
    models: {
        id: string;
        name: string;
    }[];
}>;
type ListLanguageModelsOutput = z.infer<typeof ListLanguageModelsOutputSchema>;

type schemas_Choice = Choice;
declare const schemas_ChoiceSchema: typeof ChoiceSchema;
type schemas_GenerateContentInput = GenerateContentInput;
declare const schemas_GenerateContentInputSchema: typeof GenerateContentInputSchema;
type schemas_GenerateContentOutput = GenerateContentOutput;
declare const schemas_GenerateContentOutputSchema: typeof GenerateContentOutputSchema;
type schemas_ListLanguageModelsInput = ListLanguageModelsInput;
declare const schemas_ListLanguageModelsInputSchema: typeof ListLanguageModelsInputSchema;
type schemas_ListLanguageModelsOutput = ListLanguageModelsOutput;
declare const schemas_ListLanguageModelsOutputSchema: typeof ListLanguageModelsOutputSchema;
type schemas_Message = Message;
declare const schemas_MessageSchema: typeof MessageSchema;
type schemas_ModelRef = ModelRef;
declare const schemas_ModelRefSchema: typeof ModelRefSchema;
type schemas_Usage = Usage;
declare const schemas_UsageSchema: typeof UsageSchema;
declare namespace schemas {
  export { type schemas_Choice as Choice, schemas_ChoiceSchema as ChoiceSchema, type schemas_GenerateContentInput as GenerateContentInput, schemas_GenerateContentInputSchema as GenerateContentInputSchema, type schemas_GenerateContentOutput as GenerateContentOutput, schemas_GenerateContentOutputSchema as GenerateContentOutputSchema, type schemas_ListLanguageModelsInput as ListLanguageModelsInput, schemas_ListLanguageModelsInputSchema as ListLanguageModelsInputSchema, type schemas_ListLanguageModelsOutput as ListLanguageModelsOutput, schemas_ListLanguageModelsOutputSchema as ListLanguageModelsOutputSchema, type schemas_Message as Message, schemas_MessageSchema as MessageSchema, type schemas_ModelRef as ModelRef, schemas_ModelRefSchema as ModelRefSchema, type schemas_Usage as Usage, schemas_UsageSchema as UsageSchema };
}

/** Minimal Botpress integration handler context the binding depends on. */
interface BotpressActionCtx {
    readonly botId: string;
    readonly integrationId?: string;
}
/** Resolve the model id to forward + reserve under: explicit input model id,
 *  else the provider default. */
declare function resolveModel(input: GenerateContentInput, configuration: Configuration): string;
/** Resolve the output-token cap that drives both the SpendGuard reserve
 *  estimate and the upstream `max_tokens` field. */
declare function resolveMaxTokens(input: GenerateContentInput): number;
/**
 * Build a `BotpressCallContext` from the `generateContent` action input + the
 * operator-supplied configuration + the handler `ctx`. The system prompt, when
 * present, is prepended to the message list so the SpendGuard prompt-hash and
 * token estimate cover it.
 */
declare function toBindingFromActionInput(args: {
    readonly input: GenerateContentInput;
    readonly configuration: Configuration;
    readonly ctx: BotpressActionCtx;
}): BotpressCallContext;
/**
 * Pick the binding tenant id. Configuration `tenantId` is Zod-validated as
 * non-empty in production; the binding stays tolerant of synthetic test
 * configurations and falls back to the bot id.
 */
declare function pickTenantId(configuration: Configuration, botId: string): string;

type SpendGuardCode = "BUDGET_DENIED" | "BUDGET_DEGRADED" | "BUDGET_CONFIG";
/**
 * Translate any SpendGuard-flavoured error to a Botpress `RuntimeError`.
 * Unrecognised inputs flow through as a `BUDGET_CONFIG` runtime error with the
 * original message preserved — this mirrors the Python LiteLLM callback's
 * "unknown-error-is-config" fallback
 * (sdk/python/src/spendguard/integrations/litellm.py:806-820).
 */
declare function toRuntimeError(err: unknown): RuntimeError;
/**
 * Read the SpendGuard code a translated RuntimeError carries (from its
 * metadata bag). Returns `undefined` if the RuntimeError was not minted by
 * `toRuntimeError`. Lets tests assert the translation without depending on
 * Botpress's internal numeric `code`.
 */
declare function runtimeErrorCode(rt: RuntimeError): SpendGuardCode | undefined;
/**
 * Inspect the `code` a SpendGuard-typed source error carries. Lets the unit
 * tests assert the error -> code mapping at the source.
 */
declare function codeFor(err: DecisionDenied | SidecarUnavailable | SpendGuardConfigError): SpendGuardCode;

/** Resolved per-call forward request — the normalised inputs every provider
 *  branch consumes. */
interface ForwardRequest {
    readonly provider: Configuration["upstreamProvider"];
    readonly model: string;
    readonly messages: ReadonlyArray<Message>;
    readonly systemPrompt: string | undefined;
    readonly maxTokens: number;
    readonly temperature: number | undefined;
    readonly topP: number | undefined;
    readonly stopSequences: ReadonlyArray<string> | undefined;
    readonly userId: string | undefined;
}
/** Normalised forward result — the provider's completion + real usage. */
interface ForwardResult {
    readonly id: string;
    readonly model: string;
    readonly content: string;
    readonly stopReason: "stop" | "max_tokens" | "content_filter" | "other";
    readonly inputTokens: number;
    readonly outputTokens: number;
}
/** Pluggable forward seam. Injected by tests; defaults to `defaultForward`. */
type ForwardFn = (req: ForwardRequest) => Promise<ForwardResult>;
/** Raised when the upstream provider call itself fails (network / 5xx / auth).
 *  Distinct from the SpendGuard budget errors so the runtime can release the
 *  reservation and surface a provider-flavoured RuntimeError. */
declare class ProviderForwardError extends Error {
    constructor(message: string);
}
/** Build the normalised `ForwardRequest` from the action input + config. The
 *  resolved `model` prefers the explicit `input.model.id`, then the
 *  provider-default model. `maxTokens` mirrors the SpendGuard reserve estimate
 *  so the upstream cap and the reserved budget agree. */
declare function toForwardRequest(input: GenerateContentInput, config: Configuration, resolvedModel: string, resolvedMaxTokens: number): ForwardRequest;
/** Map a `ForwardResult` into the action's `GenerateContentOutput`. `cost` is
 *  left to the caller (it depends on the SpendGuard pricing freeze), so this
 *  takes the resolved USD cost as an argument. */
declare function toGenerateContentOutput(result: ForwardResult, provider: string, cost: number): GenerateContentOutput;
/** Default OpenAI-compatible forward. Reads the provider API key from the
 *  environment and POSTs the chat-completions request. Used in production; the
 *  unit tier injects a stub `ForwardFn` instead. Anthropic uses its `messages`
 *  wire shape; openai/bedrock use the chat-completions shape. */
declare const defaultForward: ForwardFn;

/** Minimal logger surface — satisfied by the Botpress `IntegrationLogger`
 *  (`forBot().info/warn/error`) and by `console` in tests. */
interface MinimalLogger {
    warn(message: string): void;
}
interface GenerateContentArgs {
    readonly input: GenerateContentInput;
    readonly configuration: Configuration;
    readonly ctx: BotpressActionCtx;
    readonly logger?: MinimalLogger;
    /** Injected upstream forward; defaults to `defaultForward`. */
    readonly forward?: ForwardFn;
    /** Injected reservation (test seam); defaults to a fresh instance built from
     *  `configuration`. */
    readonly reservationOverride?: SpendGuardReservation;
    /** Resolved USD cost reported to Botpress billing. Defaults to 0 when no
     *  pricing freeze is wired (the SpendGuard sidecar is the source of truth
     *  for the ledgered cost; Botpress billing is advisory). */
    readonly costResolver?: (usage: {
        inputTokens: number;
        outputTokens: number;
    }) => number;
}
/**
 * Execute the SpendGuard-gated `generateContent` action.
 *
 * @throws RuntimeError on DENY / DEGRADE / config error / provider error /
 *         commit failure — always after releasing any held reservation.
 */
declare function runGenerateContent(args: GenerateContentArgs): Promise<GenerateContentOutput>;

/** Return the models SpendGuard can route to for the configured provider. */
declare function runListLanguageModels(configuration: Configuration): ListLanguageModelsOutput;

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

declare const _default: Integration;

export { type BotpressActionCtx, type BotpressCallContext, type Configuration, ConfigurationObjectSchema, ConfigurationSchema, DecisionDenied, type ForwardFn, type ForwardRequest, type ForwardResult, type GenerateContentArgs, type MinimalLogger, ProviderForwardError, type ReservationHandle, SidecarUnavailable, type SpendGuardCode, SpendGuardConfigError, SpendGuardReservation, VERSION, codeFor, _default as default, defaultForward, schemas as llmSchemas, pickTenantId, resolveMaxTokens, resolveModel, runGenerateContent, runListLanguageModels, runtimeErrorCode, toBindingFromActionInput, toForwardRequest, toGenerateContentOutput, toRuntimeError, validateConfiguration };
