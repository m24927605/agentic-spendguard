// `SpendGuardCallbackHandler` — the public LangChain.js callback handler.
//
// SLICE 2 shipped the skeleton (class shape + name + inflight Map + throw
// stubs). SLICE 3 wires the three hooks against the substrate's `reserve`
// and `commitEstimated` RPCs:
//
//   - `handleChatModelStart` derives the canonical idempotency key from
//     `(tenantId, runId, parentRunId)` via `./ids.ts`, builds a
//     `ReserveRequest` with `trigger="LLM_CALL_PRE"`, projects a coarse
//     token-count claim from the chat messages, and dispatches
//     `client.reserve(...)`. On success the `(decisionId, reservationId)`
//     pair is stashed in `inflight[runId]` for the matching POST/ERROR hook.
//     On `DecisionDenied` (or subclass — `DecisionStopped`, `ApprovalRequired`)
//     the error rethrows so LangChain halts the run. On `SidecarUnavailable`
//     (or any other substrate error) the handler swallows + logs so a sidecar
//     outage does NOT block the LLM call — design.md §3.6 / review-standards
//     §6.2 ("operational degradation, not enforcement").
//   - `handleLLMEnd` reads + deletes the inflight entry, extracts the
//     `(promptTokens, completionTokens, totalTokens)` triple from
//     `output.llmOutput.tokenUsage` (handling both LangChain camelCase and
//     OpenAI-passthrough snake_case shapes), and emits a SUCCESS commit via
//     `client.commitEstimated(...)` with `outcomeKind="SUCCESS"`.
//     A missing inflight entry is a warn-and-return — review-standards §3.11
//     fixes "unknown runId" as a no-op (idempotent re-delivery).
//   - `handleLLMError` symmetrically emits a FAILURE commit with the error's
//     `.message` threaded onto `actualErrorMessage`.
//
// SLICE 3 deliberately uses the SLICE 2 LOCKED options surface
// (`{ client, tenantId?, defaultBudgetMicrosCap? }`) — the richer
// `claimEstimator` / `callSignatureFn` / `unit` / `pricing` fields the
// design.md §4 surface anticipates are deferred to a later slice. The
// adapter projects sensible defaults instead so the wiring is end-to-end
// testable without expanding the public surface mid-flight.

import { BaseCallbackHandler } from "@langchain/core/callbacks/base";
import type { Serialized } from "@langchain/core/load/serializable";
import type { BaseMessage, MessageContent } from "@langchain/core/messages";
import type { LLMResult } from "@langchain/core/outputs";
import {
  type BudgetClaim,
  type CommitEstimatedRequest,
  DecisionDenied,
  type DecisionOutcome,
  type PricingFreeze,
  type ReserveRequest,
  type SpendGuardClient,
  type UnitRef,
} from "@spendguard/sdk";
import { deriveIdempotencyKey } from "./ids.js";
import type { SpendGuardCallbackHandlerOptions } from "./options.js";

/**
 * In-flight correlation record. Written by `handleChatModelStart`,
 * consumed + deleted by `handleLLMEnd` / `handleLLMError`. Keyed by
 * LangChain's `runId` (the run-manager UUID, design.md §6.3).
 */
interface InflightReservation {
  decisionId: string;
  reservationId: string;
}

// ── Defaults the SLICE 2 options surface deliberately omits ────────────────
//
// These constants project sensible defaults for the SLICE 3 reserve/commit
// path WITHOUT expanding the LOCKED options surface. The richer field set
// (claimEstimator, callSignatureFn, unit, pricing, route override) lands in
// a later slice when the public surface grows; until then the adapter
// behaves as a "lowest-friction default" so consumers get end-to-end
// guardrails by handing in nothing more than a configured `SpendGuardClient`.

/** Default route label surfaced on `ReserveRequest.route`. */
const DEFAULT_ROUTE = "langchain-llm";

/** Default budget unit — micro-dollars, the substrate's canonical money unit. */
const DEFAULT_UNIT: UnitRef = { unit: "USD_MICROS", denomination: 1 };

/**
 * Empty pricing freeze. SLICE 3 has no pricing-version visibility on the
 * adapter side; commits ride with a blank freeze and the sidecar's
 * server-side defaults take over. A later slice lifts the consumer-provided
 * `PricingFreeze` onto the options surface.
 */
const EMPTY_PRICING: PricingFreeze = {
  pricingVersion: "",
  pricingHash: new Uint8Array(0),
};

/**
 * Constant `stepId` for the SLICE 3 LLM-call boundary. Matched against the
 * value baked into `./ids.ts:deriveIdempotencyKey`, so the idempotency key
 * the adapter ships matches what the substrate would re-derive from the
 * canonical fields.
 */
const STEP_ID_LLM_CALL = "llm_call";

/**
 * Rough character → token ratio for projecting a pre-call budget claim from
 * raw chat-message text. Mirrors the substrate's own coarse fallback
 * heuristic. The adapter does NOT invoke a real tokenizer here — the
 * authoritative claim numbers land on the SUCCESS commit via the provider's
 * own `tokenUsage` payload.
 */
const CHARS_PER_TOKEN_HEURISTIC = 4;

/**
 * Number of micro-dollars projected per estimated token at PRE time. Used
 * only when the consumer has not provided a `defaultBudgetMicrosCap` —
 * a $0.001-per-token coarse "is there any budget left at all" probe.
 */
const DEFAULT_MICROS_PER_TOKEN = 1_000n;

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
export class SpendGuardCallbackHandler extends BaseCallbackHandler {
  /**
   * Stable serialization name. Matches LangChain.js snake_case handler
   * convention (`tracer_langchain`, `langfuse_handler`, …).
   */
  name = "spendguard_callback_handler";

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
  override raiseError = true;
  override awaitHandlers = true;

  /** Substrate client handed in by the consumer; never mutated. */
  private readonly client: SpendGuardClient;

  /** Consumer-supplied options snapshot; treated as immutable. */
  private readonly opts: SpendGuardCallbackHandlerOptions;

  /**
   * PRE → POST correlation Map keyed by LangChain's `runId`. Written by
   * `handleChatModelStart`, read + deleted by `handleLLMEnd` /
   * `handleLLMError`.
   */
  private readonly inflight = new Map<string, InflightReservation>();

  constructor(options: SpendGuardCallbackHandlerOptions) {
    super();
    this.client = options.client;
    this.opts = options;
  }

  /**
   * Resolve the tenant id that goes onto reserve/commit requests. Consumer
   * override on the handler wins over the client's configured tenant.
   */
  private get effectiveTenantId(): string {
    return this.opts.tenantId ?? this.client.tenantId;
  }

  /**
   * Build a coarse pre-call `BudgetClaim` from the chat messages. The number
   * is intentionally a heuristic — the substrate cares that the claim shape
   * is well-formed; the authoritative spend lands on the POST commit.
   */
  private projectClaim(messages: BaseMessage[][]): BudgetClaim {
    let totalChars = 0;
    for (const turn of messages) {
      for (const msg of turn) {
        totalChars += measureContentChars(msg.content);
      }
    }
    const estimatedTokens = BigInt(Math.max(1, Math.ceil(totalChars / CHARS_PER_TOKEN_HEURISTIC)));
    const cap = this.opts.defaultBudgetMicrosCap;
    const amountMicros =
      cap !== undefined && cap > 0n ? cap : estimatedTokens * DEFAULT_MICROS_PER_TOKEN;
    // HARDEN_D05_UR — thread caller-supplied unitId onto the wire UnitRef.
    // Omitted unitId keeps the pre-HARDEN_D05_UR wire shape (substrate
    // `mapUnitRef` coerces to "").
    const unit: UnitRef = this.opts.unitId
      ? { ...DEFAULT_UNIT, unitId: this.opts.unitId }
      : DEFAULT_UNIT;
    return {
      scopeId: this.opts.budgetId ?? this.effectiveTenantId,
      amountAtomic: amountMicros.toString(),
      unit,
    };
  }

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
  override async handleChatModelStart(
    _llm: Serialized,
    messages: BaseMessage[][],
    runId: string,
    parentRunId?: string,
    _extraParams?: Record<string, unknown>,
    _tags?: string[],
    metadata?: Record<string, unknown>,
    name?: string,
  ): Promise<void> {
    const tenantId = this.effectiveTenantId;
    const idempotencyKey = deriveIdempotencyKey({
      tenantId,
      runId,
      ...(parentRunId !== undefined ? { parentRunId } : {}),
    });
    const traceparent = readTraceparent(metadata);
    const req: ReserveRequest = {
      trigger: "LLM_CALL_PRE",
      runId,
      stepId: STEP_ID_LLM_CALL,
      llmCallId: runId,
      decisionId: runId,
      route: name ?? DEFAULT_ROUTE,
      projectedClaims: [this.projectClaim(messages)],
      idempotencyKey,
      ...(traceparent !== undefined ? { traceparent } : {}),
      ...(parentRunId !== undefined ? { parentRunId } : {}),
    };

    let outcome: DecisionOutcome;
    try {
      outcome = await this.client.reserve(req);
    } catch (err) {
      // `DecisionDenied` covers DENY + STOP + APPROVAL_REQUIRED (subclasses)
      // — those MUST propagate so the LangChain run halts before the
      // provider request fires (review-standards §4.1, §4.7).
      if (err instanceof DecisionDenied) {
        throw err;
      }
      // Anything else — `SidecarUnavailable`, transport hiccups, ack
      // rejections — is operational. Log + return; do NOT block the LLM
      // call. No inflight entry is stashed, so the matching POST will
      // also no-op (warn).
      const reason = err instanceof Error ? err.message : String(err);
      console.warn(
        `[spendguard:langchain] reserve() failed for runId=${runId}; ` +
          `LLM call proceeds without budget gate (${reason})`,
      );
      return;
    }

    this.inflight.set(runId, {
      decisionId: outcome.decisionId,
      reservationId: outcome.reservationIds[0] ?? "",
    });
  }

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
  override async handleLLMEnd(
    output: LLMResult,
    runId: string,
    _parentRunId?: string,
    _tags?: string[],
  ): Promise<void> {
    const pending = this.inflight.get(runId);
    if (pending === undefined) {
      console.warn(
        `[spendguard:langchain] handleLLMEnd: no inflight entry for runId=${runId} (reserve may have failed or commit was already delivered)`,
      );
      return;
    }
    this.inflight.delete(runId);

    const usage = extractTokenUsage(output);
    if (usage === undefined) {
      console.warn(
        `[spendguard:langchain] handleLLMEnd: no tokenUsage in LLMResult for runId=${runId}; committing with actual tokens = 0`,
      );
    }
    const req: CommitEstimatedRequest = {
      runId,
      stepId: STEP_ID_LLM_CALL,
      llmCallId: runId,
      decisionId: pending.decisionId,
      reservationId: pending.reservationId,
      estimatedAmountAtomic: "0",
      unit: DEFAULT_UNIT,
      pricing: EMPTY_PRICING,
      providerEventId: "",
      outcome: "SUCCESS",
      outcomeKind: "SUCCESS",
      actualInputTokensWire: String(usage?.promptTokens ?? 0),
      actualOutputTokensWire: String(usage?.completionTokens ?? 0),
    };
    await this.client.commitEstimated(req);
  }

  /**
   * SLICE 3 wires `handleLLMError` against `client.commitEstimated()` with
   * the PROVIDER_ERROR / FAILURE outcome shape. Mirrors `handleLLMEnd`'s
   * inflight-lookup discipline; the error's `.message` is threaded onto
   * `actualErrorMessage` so the substrate's outcome event carries the
   * provider's failure reason.
   */
  override async handleLLMError(
    err: Error,
    runId: string,
    _parentRunId?: string,
    _tags?: string[],
  ): Promise<void> {
    const pending = this.inflight.get(runId);
    if (pending === undefined) {
      console.warn(
        `[spendguard:langchain] handleLLMError: no inflight entry for runId=${runId} (reserve may have failed or commit was already delivered)`,
      );
      return;
    }
    this.inflight.delete(runId);

    const req: CommitEstimatedRequest = {
      runId,
      stepId: STEP_ID_LLM_CALL,
      llmCallId: runId,
      decisionId: pending.decisionId,
      reservationId: pending.reservationId,
      estimatedAmountAtomic: "0",
      unit: DEFAULT_UNIT,
      pricing: EMPTY_PRICING,
      providerEventId: "",
      outcome: "PROVIDER_ERROR",
      outcomeKind: "FAILURE",
      actualErrorMessage: err.message,
    };
    await this.client.commitEstimated(req);
  }
}

// ── Internal helpers ──────────────────────────────────────────────────────

/**
 * Sum the visible character length of a LangChain `MessageContent`. Strings
 * count their `.length` directly; complex arrays sum each text part's
 * length and ignore non-text parts (images, tool-calls) so the heuristic
 * stays a coarse proxy without paying for binary payloads.
 */
function measureContentChars(content: MessageContent): number {
  if (typeof content === "string") {
    return content.length;
  }
  let total = 0;
  for (const part of content) {
    if ("text" in part && typeof part.text === "string") {
      total += part.text.length;
    }
  }
  return total;
}

/**
 * Pull a W3C `traceparent` header value out of LangChain's `metadata` bag.
 * LangChain itself does not require any specific shape here; the adapter
 * looks for the canonical lowercase key only (consumers who want to forward
 * a parent span set `metadata.traceparent` on the invoke).
 */
function readTraceparent(metadata?: Record<string, unknown>): string | undefined {
  if (metadata === undefined) return undefined;
  const tp = metadata.traceparent;
  return typeof tp === "string" ? tp : undefined;
}

/**
 * Token-usage triple extracted from a `LLMResult`. Adapter-internal; not
 * exported. Accepts both LangChain-canonical camelCase and the OpenAI
 * passthrough snake_case shape so the adapter handles `ChatOpenAI`'s
 * raw `llmOutput.tokenUsage` AND the post-normalisation shape any other
 * provider integration emits.
 */
interface ExtractedTokenUsage {
  promptTokens: number;
  completionTokens: number;
}

function extractTokenUsage(output: LLMResult): ExtractedTokenUsage | undefined {
  const tokenUsage = output.llmOutput?.tokenUsage ?? output.llmOutput?.token_usage;
  if (tokenUsage === undefined || tokenUsage === null || typeof tokenUsage !== "object") {
    return undefined;
  }
  const bag = tokenUsage as Record<string, unknown>;
  const prompt = readTokenCount(bag, ["promptTokens", "prompt_tokens"]);
  const completion = readTokenCount(bag, ["completionTokens", "completion_tokens"]);
  if (prompt === undefined && completion === undefined) {
    return undefined;
  }
  return {
    promptTokens: prompt ?? 0,
    completionTokens: completion ?? 0,
  };
}

function readTokenCount(bag: Record<string, unknown>, keys: readonly string[]): number | undefined {
  for (const key of keys) {
    const value = bag[key];
    if (typeof value === "number" && Number.isFinite(value)) {
      return value;
    }
  }
  return undefined;
}
