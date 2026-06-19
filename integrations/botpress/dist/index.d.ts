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
    model?: {
        /** Provider-qualified model id, e.g. gpt-4o-mini */
        id: string;
    };
    /**
     * Reasoning effort level to use for models that support reasoning. Specifying "none" will indicate the LLM to not use reasoning (for models that support optional reasoning). A "dynamic" effort will indicate the provider to automatically determine the reasoning effort (if supported by the provider). If not provided the model will not use reasoning for models with optional reasoning or use the default reasoning effort specified by the provider for reasoning-only models.
     * Note: A higher reasoning effort will incur in higher output token charges from the LLM provider.
     */
    reasoningEffort?: "low" | "medium" | "high" | "dynamic" | "none";
    /** Optional system prompt to guide the model */
    systemPrompt?: string;
    /** Array of messages for the model to process */
    messages: Array<{
        role: "user" | "assistant";
        type?: "text" | "tool_calls" | "tool_result" | "multipart";
        /** Required if `type` is "tool_calls" */
        toolCalls?: Array<{
            id: string;
            type: "function";
            function: {
                name: string;
                arguments: /** Some LLMs may generate invalid JSON for a tool call, so this will be `null` when it happens. */ {
                    [key: string]: any;
                } | null;
            };
        }>;
        /** Required if `type` is "tool_result" */
        toolResultCallId?: string;
        content: string | Array<{
            type: "text" | "image";
            /** Indicates the MIME type of the content. If not provided it will be detected from the content-type header of the provided URL. */
            mimeType?: string;
            /** Required if part type is "text" */
            text?: string;
            /** Required if part type is "image" */
            url?: string;
        }> | null;
    }>;
    /** Response format expected from the model. If "json_object" is chosen, you must instruct the model to generate JSON either via the system prompt or a user message. */
    responseFormat?: "text" | "json_object";
    /** Maximum number of tokens allowed in the generated response */
    maxTokens?: number;
    /** Sampling temperature for the model. Higher values result in more random outputs. */
    temperature?: number;
    /** Top-p sampling parameter. Limits sampling to the smallest set of tokens with a cumulative probability above the threshold. */
    topP?: number;
    /** Sequences where the model should stop generating further tokens. */
    stopSequences?: string[];
    /** List of tools available for the model to use */
    tools?: Array<{
        type: "function";
        function: {
            /** Function name */
            name: string;
            description?: string;
            /** JSON schema of the function arguments */
            argumentsSchema?: {};
        };
    }>;
    /** The chosen tool to use for content generation */
    toolChoice?: {
        type?: "auto" | "specific" | "any" | "none" | "";
        /** Required if `type` is "specific" */
        functionName?: string;
    };
    /** Unique identifier of the user that sent the prompt */
    userId?: string;
    /** Set to `true` to output debug information to the bot logs */
    debug?: boolean;
    /** Contextual metadata about the prompt */
    meta?: {
        /** Source of the prompt, e.g. agent/:id/:version cards/ai-generate, cards/ai-task, nodes/autonomous, etc. */
        promptSource?: string;
        promptCategory?: string;
        /** Name of the integration that originally received the message that initiated this action */
        integrationName?: string;
    };
};

type Output$1 = {
    /** Response ID from LLM provider */
    id: string;
    /** LLM provider name */
    provider: string;
    /** The name of the LLM model that was used */
    model: string;
    /** Array of generated message choices from the model */
    choices: Array<{
        type?: "text" | "tool_calls" | "tool_result" | "multipart";
        /** Required if `type` is "tool_calls" */
        toolCalls?: Array<{
            id: string;
            type: "function";
            function: {
                name: string;
                arguments: /** Some LLMs may generate invalid JSON for a tool call, so this will be `null` when it happens. */ {
                    [key: string]: any;
                } | null;
            };
        }>;
        /** Required if `type` is "tool_result" */
        toolResultCallId?: string;
        content: string | Array<{
            type: "text" | "image";
            /** Indicates the MIME type of the content. If not provided it will be detected from the content-type header of the provided URL. */
            mimeType?: string;
            /** Required if part type is "text" */
            text?: string;
            /** Required if part type is "image" */
            url?: string;
        }> | null;
        role: "assistant";
        index: number;
        stopReason: "stop" | "max_tokens" | "tool_calls" | "content_filter" | "other";
    }>;
    /** A breakdown of token usage and cost information */
    usage: {
        /** Number of input tokens used by the model */
        inputTokens: number;
        /** Cost of the input tokens received by the model, in U.S. dollars */
        inputCost: number;
        /** Number of output tokens used by the model */
        outputTokens: number;
        /** Cost of the output tokens generated by the model, in U.S. dollars */
        outputCost: number;
    };
    /** Metadata added by Botpress */
    botpress: {
        /** Total cost of the content generation, in U.S. dollars */
        cost: number;
    };
};

type GenerateContent = {
    "input": Input$1;
    "output": Output$1;
};

type Input = {};

type Output = {
    models: Array<{
        /** Unique identifier of the large language model */
        id: string;
        name: string;
        description: string;
        tags: Array<"recommended" | "deprecated" | "general-purpose" | "low-cost" | "vision" | "coding" | "agents" | "function-calling" | "roleplay" | "storytelling" | "reasoning" | "preview" | "speech-to-text" | "image-generation" | "text-to-speech">;
        input: {
            maxTokens: number;
            /** Cost per 1 million tokens, in U.S. dollars */
            costPer1MTokens: number;
        };
        output: {
            maxTokens: number;
            /** Cost per 1 million tokens, in U.S. dollars */
            costPer1MTokens: number;
        };
    } & {
        /** Provider-qualified model id, e.g. gpt-4o-mini */
        id: string;
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
    /** Provider-qualified model id, e.g. gpt-4o-mini */
    id: string;
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

/** The `id` field of the `llm` interface `modelRef` entity. The interface
 *  declares `modelRef = z.object({ id: <string> }).catchall(z.never())` and
 *  references it from both action schemas via `z.ref("modelRef")`. We supply
 *  the concrete `id` schema here so `integration.definition.ts` can build the
 *  matching `modelRef` entity (mirroring the first-party OpenAI integration's
 *  `entities.modelRef.schema = z.object({ id: <languageModelId> })`). */
declare const LanguageModelIdSchema: z.ZodString;
/** A reference to one upstream model the integration exposes — the runtime
 *  projection of the `llm` interface `modelRef` entity (`{ id }`). Note the
 *  interface `modelRef` carries ONLY `id` (no `name`); the human-facing name
 *  lives on the `listLanguageModels` model rows, not on the ref. */
declare const ModelRefSchema: z.ZodObject<{
    id: z.ZodString;
}, "strip", {
    id: string;
}, {
    id: string;
}>;
type ModelRef = z.infer<typeof ModelRefSchema>;
/** One message in the internal prompt representation. The `llm` interface
 *  restricts message roles to `user | assistant`, but the SpendGuard prompt
 *  hash + token estimate also fold in the `systemPrompt` (mapped to a
 *  synthetic `system` message by the binding), so the internal role set is
 *  wider. `content` is the flattened text the prompt-hash is computed over. */
declare const MessageSchema: z.ZodObject<{
    role: z.ZodEnum<["system", "user", "assistant", "tool"]>;
    content: z.ZodString;
}, "strip", {
    role: "assistant" | "system" | "user" | "tool";
    content: string;
}, {
    role: "assistant" | "system" | "user" | "tool";
    content: string;
}>;
type Message = z.infer<typeof MessageSchema>;
/** Per-call token usage echoed back from the upstream provider (internal). The
 *  interface output additionally carries `inputCost` / `outputCost`; those are
 *  filled in by the action boundary in `src/index.ts`. */
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
/** One generated choice (internal). SpendGuard emits a single text choice; the
 *  interface output `stopReason` additionally allows `tool_calls`, which this
 *  text-only surface never produces. */
declare const ChoiceSchema: z.ZodObject<{
    role: z.ZodLiteral<"assistant">;
    type: z.ZodLiteral<"text">;
    content: z.ZodString;
    index: z.ZodNumber;
    stopReason: z.ZodEnum<["stop", "max_tokens", "content_filter", "other"]>;
}, "strip", {
    role: "assistant";
    type: "text";
    content: string;
    index: number;
    stopReason: "stop" | "max_tokens" | "content_filter" | "other";
}, {
    role: "assistant";
    type: "text";
    content: string;
    index: number;
    stopReason: "stop" | "max_tokens" | "content_filter" | "other";
}>;
type Choice = z.infer<typeof ChoiceSchema>;
/** Internal `generateContent` input — the simplified projection the SpendGuard
 *  pipeline consumes. `src/index.ts` builds this from the interface's richer
 *  action input. */
declare const GenerateContentInputSchema: z.ZodObject<{
    model: z.ZodOptional<z.ZodObject<{
        id: z.ZodString;
    }, "strip", {
        id: string;
    }, {
        id: string;
    }>>;
    messages: z.ZodArray<z.ZodObject<{
        role: z.ZodEnum<["system", "user", "assistant", "tool"]>;
        content: z.ZodString;
    }, "strip", {
        role: "assistant" | "system" | "user" | "tool";
        content: string;
    }, {
        role: "assistant" | "system" | "user" | "tool";
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
        role: "assistant" | "system" | "user" | "tool";
        content: string;
    }[];
    model?: {
        id: string;
    } | undefined;
    maxTokens?: number | undefined;
    systemPrompt?: string | undefined;
    temperature?: number | undefined;
    topP?: number | undefined;
    stopSequences?: string[] | undefined;
    userId?: string | undefined;
}, {
    messages: {
        role: "assistant" | "system" | "user" | "tool";
        content: string;
    }[];
    model?: {
        id: string;
    } | undefined;
    maxTokens?: number | undefined;
    systemPrompt?: string | undefined;
    temperature?: number | undefined;
    topP?: number | undefined;
    stopSequences?: string[] | undefined;
    userId?: string | undefined;
}>;
type GenerateContentInput = z.infer<typeof GenerateContentInputSchema>;
/** Internal `generateContent` output. Structurally a subset of the interface
 *  output (single text choice, no cost fields); `src/index.ts` widens it to
 *  the interface output by adding `usage.inputCost` / `usage.outputCost`. */
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
        type: "text";
        content: string;
        index: number;
        stopReason: "stop" | "max_tokens" | "content_filter" | "other";
    }, {
        role: "assistant";
        type: "text";
        content: string;
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
    provider: string;
    model: string;
    choices: {
        role: "assistant";
        type: "text";
        content: string;
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
    provider: string;
    model: string;
    choices: {
        role: "assistant";
        type: "text";
        content: string;
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
/** One row of the `listLanguageModels` catalog — matches the `llm` interface
 *  model shape (`modelRef` intersected with the model metadata Botpress Studio
 *  renders in the model picker). */
declare const LanguageModelSchema: z.ZodObject<{
    id: z.ZodString;
    name: z.ZodString;
    description: z.ZodString;
    tags: z.ZodArray<z.ZodEnum<["recommended", "deprecated", "general-purpose", "low-cost", "vision", "coding", "agents", "function-calling", "roleplay", "storytelling", "reasoning", "preview", "speech-to-text", "image-generation", "text-to-speech"]>, "many">;
    input: z.ZodObject<{
        maxTokens: z.ZodNumber;
        costPer1MTokens: z.ZodNumber;
    }, "strip", {
        maxTokens: number;
        costPer1MTokens: number;
    }, {
        maxTokens: number;
        costPer1MTokens: number;
    }>;
    output: z.ZodObject<{
        maxTokens: z.ZodNumber;
        costPer1MTokens: z.ZodNumber;
    }, "strip", {
        maxTokens: number;
        costPer1MTokens: number;
    }, {
        maxTokens: number;
        costPer1MTokens: number;
    }>;
}, "strip", {
    input: {
        maxTokens: number;
        costPer1MTokens: number;
    };
    id: string;
    name: string;
    description: string;
    tags: ("recommended" | "deprecated" | "general-purpose" | "low-cost" | "vision" | "coding" | "agents" | "function-calling" | "roleplay" | "storytelling" | "reasoning" | "preview" | "speech-to-text" | "image-generation" | "text-to-speech")[];
    output: {
        maxTokens: number;
        costPer1MTokens: number;
    };
}, {
    input: {
        maxTokens: number;
        costPer1MTokens: number;
    };
    id: string;
    name: string;
    description: string;
    tags: ("recommended" | "deprecated" | "general-purpose" | "low-cost" | "vision" | "coding" | "agents" | "function-calling" | "roleplay" | "storytelling" | "reasoning" | "preview" | "speech-to-text" | "image-generation" | "text-to-speech")[];
    output: {
        maxTokens: number;
        costPer1MTokens: number;
    };
}>;
type LanguageModel = z.infer<typeof LanguageModelSchema>;
declare const ListLanguageModelsInputSchema: z.ZodObject<{}, "strip", {}, {}>;
type ListLanguageModelsInput = z.infer<typeof ListLanguageModelsInputSchema>;
declare const ListLanguageModelsOutputSchema: z.ZodObject<{
    models: z.ZodArray<z.ZodObject<{
        id: z.ZodString;
        name: z.ZodString;
        description: z.ZodString;
        tags: z.ZodArray<z.ZodEnum<["recommended", "deprecated", "general-purpose", "low-cost", "vision", "coding", "agents", "function-calling", "roleplay", "storytelling", "reasoning", "preview", "speech-to-text", "image-generation", "text-to-speech"]>, "many">;
        input: z.ZodObject<{
            maxTokens: z.ZodNumber;
            costPer1MTokens: z.ZodNumber;
        }, "strip", {
            maxTokens: number;
            costPer1MTokens: number;
        }, {
            maxTokens: number;
            costPer1MTokens: number;
        }>;
        output: z.ZodObject<{
            maxTokens: z.ZodNumber;
            costPer1MTokens: z.ZodNumber;
        }, "strip", {
            maxTokens: number;
            costPer1MTokens: number;
        }, {
            maxTokens: number;
            costPer1MTokens: number;
        }>;
    }, "strip", {
        input: {
            maxTokens: number;
            costPer1MTokens: number;
        };
        id: string;
        name: string;
        description: string;
        tags: ("recommended" | "deprecated" | "general-purpose" | "low-cost" | "vision" | "coding" | "agents" | "function-calling" | "roleplay" | "storytelling" | "reasoning" | "preview" | "speech-to-text" | "image-generation" | "text-to-speech")[];
        output: {
            maxTokens: number;
            costPer1MTokens: number;
        };
    }, {
        input: {
            maxTokens: number;
            costPer1MTokens: number;
        };
        id: string;
        name: string;
        description: string;
        tags: ("recommended" | "deprecated" | "general-purpose" | "low-cost" | "vision" | "coding" | "agents" | "function-calling" | "roleplay" | "storytelling" | "reasoning" | "preview" | "speech-to-text" | "image-generation" | "text-to-speech")[];
        output: {
            maxTokens: number;
            costPer1MTokens: number;
        };
    }>, "many">;
}, "strip", {
    models: {
        input: {
            maxTokens: number;
            costPer1MTokens: number;
        };
        id: string;
        name: string;
        description: string;
        tags: ("recommended" | "deprecated" | "general-purpose" | "low-cost" | "vision" | "coding" | "agents" | "function-calling" | "roleplay" | "storytelling" | "reasoning" | "preview" | "speech-to-text" | "image-generation" | "text-to-speech")[];
        output: {
            maxTokens: number;
            costPer1MTokens: number;
        };
    }[];
}, {
    models: {
        input: {
            maxTokens: number;
            costPer1MTokens: number;
        };
        id: string;
        name: string;
        description: string;
        tags: ("recommended" | "deprecated" | "general-purpose" | "low-cost" | "vision" | "coding" | "agents" | "function-calling" | "roleplay" | "storytelling" | "reasoning" | "preview" | "speech-to-text" | "image-generation" | "text-to-speech")[];
        output: {
            maxTokens: number;
            costPer1MTokens: number;
        };
    }[];
}>;
type ListLanguageModelsOutput = z.infer<typeof ListLanguageModelsOutputSchema>;

type schemas_Choice = Choice;
declare const schemas_ChoiceSchema: typeof ChoiceSchema;
type schemas_GenerateContentInput = GenerateContentInput;
declare const schemas_GenerateContentInputSchema: typeof GenerateContentInputSchema;
type schemas_GenerateContentOutput = GenerateContentOutput;
declare const schemas_GenerateContentOutputSchema: typeof GenerateContentOutputSchema;
type schemas_LanguageModel = LanguageModel;
declare const schemas_LanguageModelIdSchema: typeof LanguageModelIdSchema;
declare const schemas_LanguageModelSchema: typeof LanguageModelSchema;
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
  export { type schemas_Choice as Choice, schemas_ChoiceSchema as ChoiceSchema, type schemas_GenerateContentInput as GenerateContentInput, schemas_GenerateContentInputSchema as GenerateContentInputSchema, type schemas_GenerateContentOutput as GenerateContentOutput, schemas_GenerateContentOutputSchema as GenerateContentOutputSchema, type schemas_LanguageModel as LanguageModel, schemas_LanguageModelIdSchema as LanguageModelIdSchema, schemas_LanguageModelSchema as LanguageModelSchema, type schemas_ListLanguageModelsInput as ListLanguageModelsInput, schemas_ListLanguageModelsInputSchema as ListLanguageModelsInputSchema, type schemas_ListLanguageModelsOutput as ListLanguageModelsOutput, schemas_ListLanguageModelsOutputSchema as ListLanguageModelsOutputSchema, type schemas_Message as Message, schemas_MessageSchema as MessageSchema, type schemas_ModelRef as ModelRef, schemas_ModelRefSchema as ModelRefSchema, type schemas_Usage as Usage, schemas_UsageSchema as UsageSchema };
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

/** The interface `generateContent` input, as generated by `bp build`. */
type InterfaceGenerateContentInput = Input$1;
/** The interface `generateContent` output, as generated by `bp build`. */
type InterfaceGenerateContentOutput = Output$1;
type InterfaceMessage = InterfaceGenerateContentInput["messages"][number];
type InterfaceContent = InterfaceMessage["content"];
/**
 * Flatten an interface message `content` (string | multipart array | null) to
 * the plain text the SpendGuard prompt-hash + token estimate operate over.
 * - `null`            -> "" (e.g. an assistant message that is purely tool
 *                        calls carries `content: null`).
 * - `string`          -> as-is.
 * - multipart array   -> the concatenation of the text parts; non-text parts
 *                        (images) contribute nothing to the text estimate.
 */
declare function flattenContent(content: InterfaceContent): string;
/**
 * Narrow the interface `generateContent` input to the internal
 * `GenerateContentInput` the SpendGuard pipeline consumes. The interface
 * `modelRef` carries only `{ id }`; the internal model ref mirrors it.
 */
declare function toInternalInput(input: InterfaceGenerateContentInput): GenerateContentInput;
/**
 * Widen the internal `GenerateContentOutput` to the interface output. The
 * interface `usage` requires per-token cost fields the SpendGuard pipeline does
 * not compute (the sidecar ledger is the source of truth for cost); they are
 * reported as 0 and the aggregate advisory `botpress.cost` is preserved.
 */
declare function toInterfaceOutput(output: GenerateContentOutput): InterfaceGenerateContentOutput;

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

export { type BotpressActionCtx, type BotpressCallContext, type Configuration, ConfigurationObjectSchema, ConfigurationSchema, DecisionDenied, type ForwardFn, type ForwardRequest, type ForwardResult, type GenerateContentArgs, type InterfaceGenerateContentInput, type InterfaceGenerateContentOutput, type MinimalLogger, ProviderForwardError, type ReservationHandle, SidecarUnavailable, type SpendGuardCode, SpendGuardConfigError, SpendGuardReservation, VERSION, codeFor, _default as default, defaultForward, flattenContent, schemas as llmSchemas, pickTenantId, resolveMaxTokens, resolveModel, runGenerateContent, runListLanguageModels, runtimeErrorCode, toBindingFromActionInput, toForwardRequest, toGenerateContentOutput, toInterfaceOutput, toInternalInput, toRuntimeError, validateConfiguration };
