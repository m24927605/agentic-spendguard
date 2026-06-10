import { BaseCallbackHandler } from '@langchain/core/callbacks/base';
import { DecisionDenied, deriveIdempotencyKey as deriveIdempotencyKey$1 } from '@spendguard/sdk';
export { DecisionDenied, SidecarUnavailable, SpendGuardError } from '@spendguard/sdk';

// src/handler.ts
function deriveIdempotencyKey(opts) {
  return deriveIdempotencyKey$1({
    tenantId: opts.tenantId,
    sessionId: opts.parentRunId ?? opts.runId,
    runId: opts.runId,
    stepId: "llm_call",
    llmCallId: opts.runId,
    trigger: "LLM_CALL_PRE"
  });
}

// src/handler.ts
var DEFAULT_ROUTE = "langchain-llm";
var DEFAULT_UNIT = { unit: "USD_MICROS", denomination: 1 };
var EMPTY_PRICING = {
  pricingVersion: "",
  pricingHash: new Uint8Array(0)
};
var STEP_ID_LLM_CALL = "llm_call";
var CHARS_PER_TOKEN_HEURISTIC = 4;
var DEFAULT_MICROS_PER_TOKEN = 1000n;
var SpendGuardCallbackHandler = class extends BaseCallbackHandler {
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
  raiseError = true;
  awaitHandlers = true;
  /** Substrate client handed in by the consumer; never mutated. */
  client;
  /** Consumer-supplied options snapshot; treated as immutable. */
  opts;
  /**
   * PRE → POST correlation Map keyed by LangChain's `runId`. Written by
   * `handleChatModelStart`, read + deleted by `handleLLMEnd` /
   * `handleLLMError`.
   */
  inflight = /* @__PURE__ */ new Map();
  constructor(options) {
    super();
    this.client = options.client;
    this.opts = options;
  }
  /**
   * Resolve the tenant id that goes onto reserve/commit requests. Consumer
   * override on the handler wins over the client's configured tenant.
   */
  get effectiveTenantId() {
    return this.opts.tenantId ?? this.client.tenantId;
  }
  /**
   * Build a coarse pre-call `BudgetClaim` from the chat messages. The number
   * is intentionally a heuristic — the substrate cares that the claim shape
   * is well-formed; the authoritative spend lands on the POST commit.
   */
  projectClaim(messages) {
    let totalChars = 0;
    for (const turn of messages) {
      for (const msg of turn) {
        totalChars += measureContentChars(msg.content);
      }
    }
    const estimatedTokens = BigInt(Math.max(1, Math.ceil(totalChars / CHARS_PER_TOKEN_HEURISTIC)));
    const cap = this.opts.defaultBudgetMicrosCap;
    const override = this.opts.estimateOverrideAtomic;
    const amountMicros = override !== void 0 && /^[0-9]+$/.test(override) ? BigInt(override) : cap !== void 0 && cap > 0n ? cap : estimatedTokens * DEFAULT_MICROS_PER_TOKEN;
    const unit = this.opts.unitId ? { ...DEFAULT_UNIT, unitId: this.opts.unitId } : DEFAULT_UNIT;
    return {
      scopeId: this.opts.budgetId ?? this.effectiveTenantId,
      amountAtomic: amountMicros.toString(),
      unit,
      // HARDEN_D05_WI — thread caller-supplied windowInstanceId onto the
      // wire claim (substrate coerces omitted to "").
      ...this.opts.windowInstanceId ? { windowInstanceId: this.opts.windowInstanceId } : {}
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
  async handleChatModelStart(_llm, messages, runId, parentRunId, _extraParams, _tags, metadata, name) {
    const tenantId = this.effectiveTenantId;
    const idempotencyKey = deriveIdempotencyKey({
      tenantId,
      runId,
      ...parentRunId !== void 0 ? { parentRunId } : {}
    });
    const traceparent = readTraceparent(metadata);
    const projectedClaim = this.projectClaim(messages);
    const req = {
      trigger: "LLM_CALL_PRE",
      runId,
      stepId: STEP_ID_LLM_CALL,
      llmCallId: runId,
      decisionId: runId,
      route: name ?? DEFAULT_ROUTE,
      projectedClaims: [projectedClaim],
      idempotencyKey,
      ...traceparent !== void 0 ? { traceparent } : {},
      ...parentRunId !== void 0 ? { parentRunId } : {}
    };
    let outcome;
    try {
      outcome = await this.client.reserve(req);
    } catch (err) {
      if (err instanceof DecisionDenied) {
        throw err;
      }
      const reason = err instanceof Error ? err.message : String(err);
      console.warn(
        `[spendguard:langchain] reserve() failed for runId=${runId}; LLM call proceeds without budget gate (${reason})`
      );
      return;
    }
    this.inflight.set(runId, {
      decisionId: outcome.decisionId,
      reservationId: outcome.reservationIds[0] ?? "",
      unit: projectedClaim.unit,
      ...this.opts.pricing !== void 0 ? { pricing: this.opts.pricing } : {},
      estimatedAmountAtomic: projectedClaim.amountAtomic
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
  async handleLLMEnd(output, runId, _parentRunId, _tags) {
    const pending = this.inflight.get(runId);
    if (pending === void 0) {
      console.warn(
        `[spendguard:langchain] handleLLMEnd: no inflight entry for runId=${runId} (reserve may have failed or commit was already delivered)`
      );
      return;
    }
    this.inflight.delete(runId);
    const usage = extractTokenUsage(output);
    if (usage === void 0) {
      console.warn(
        `[spendguard:langchain] handleLLMEnd: no tokenUsage in LLMResult for runId=${runId}; committing with actual tokens = 0`
      );
    }
    const usageSum = (usage?.promptTokens ?? 0) + (usage?.completionTokens ?? 0);
    const estimatedAmountAtomic = usageSum > 0 ? String(usageSum) : pending.estimatedAmountAtomic;
    const req = {
      runId,
      stepId: STEP_ID_LLM_CALL,
      llmCallId: runId,
      decisionId: pending.decisionId,
      reservationId: pending.reservationId,
      estimatedAmountAtomic,
      // HARDEN_D05_WI — reuse the reserve-time unit so payload.unit_id matches
      // the reservation (ledger rejects mismatched commit units).
      unit: pending.unit ?? DEFAULT_UNIT,
      // HARDEN_D05_WI — repeat the reserve-time freeze tuple (ledger rejects
      // commits whose pricing tuple differs from the reservation's).
      pricing: pending.pricing ?? EMPTY_PRICING,
      providerEventId: "",
      outcome: "SUCCESS",
      outcomeKind: "SUCCESS",
      actualInputTokensWire: String(usage?.promptTokens ?? 0),
      actualOutputTokensWire: String(usage?.completionTokens ?? 0)
    };
    await this.safeCommit(req);
  }
  /**
   * SLICE 3 wires `handleLLMError` against `client.commitEstimated()` with
   * the PROVIDER_ERROR / FAILURE outcome shape. Mirrors `handleLLMEnd`'s
   * inflight-lookup discipline; the error's `.message` is threaded onto
   * `actualErrorMessage` so the substrate's outcome event carries the
   * provider's failure reason.
   */
  async handleLLMError(err, runId, _parentRunId, _tags) {
    const pending = this.inflight.get(runId);
    if (pending === void 0) {
      console.warn(
        `[spendguard:langchain] handleLLMError: no inflight entry for runId=${runId} (reserve may have failed or commit was already delivered)`
      );
      return;
    }
    this.inflight.delete(runId);
    const req = {
      runId,
      stepId: STEP_ID_LLM_CALL,
      llmCallId: runId,
      decisionId: pending.decisionId,
      reservationId: pending.reservationId,
      estimatedAmountAtomic: "0",
      // HARDEN_D05_WI — reserve-time unit + freeze tuple must match the
      // reservation even on the FAILURE commit path.
      unit: pending.unit ?? DEFAULT_UNIT,
      pricing: pending.pricing ?? EMPTY_PRICING,
      providerEventId: "",
      outcome: "PROVIDER_ERROR",
      outcomeKind: "FAILURE",
      actualErrorMessage: err.message
    };
    await this.safeCommit(req);
  }
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
  async safeCommit(req) {
    try {
      await this.client.commitEstimated(req);
    } catch (err) {
      const reason = err instanceof Error ? err.message : String(err);
      console.warn(
        `[spendguard:langchain] commitEstimated() failed for runId=${req.runId}; LLM result preserved (${reason})`
      );
    }
  }
};
function measureContentChars(content) {
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
function readTraceparent(metadata) {
  if (metadata === void 0) return void 0;
  const tp = metadata.traceparent;
  return typeof tp === "string" ? tp : void 0;
}
function extractTokenUsage(output) {
  const tokenUsage = output.llmOutput?.tokenUsage ?? output.llmOutput?.token_usage;
  if (tokenUsage !== void 0 && tokenUsage !== null && typeof tokenUsage === "object") {
    const bag = tokenUsage;
    const prompt = readTokenCount(bag, ["promptTokens", "prompt_tokens"]);
    const completion = readTokenCount(bag, ["completionTokens", "completion_tokens"]);
    if (prompt !== void 0 || completion !== void 0) {
      return {
        promptTokens: prompt ?? 0,
        completionTokens: completion ?? 0
      };
    }
  }
  for (const turn of output.generations ?? []) {
    for (const gen of turn ?? []) {
      const message = gen?.message;
      const usage = message?.usage_metadata;
      if (usage === void 0 || usage === null || typeof usage !== "object") {
        continue;
      }
      const bag = usage;
      const prompt = readTokenCount(bag, ["input_tokens", "promptTokens", "prompt_tokens"]);
      const completion = readTokenCount(bag, [
        "output_tokens",
        "completionTokens",
        "completion_tokens"
      ]);
      if (prompt !== void 0 || completion !== void 0) {
        return {
          promptTokens: prompt ?? 0,
          completionTokens: completion ?? 0
        };
      }
    }
  }
  return void 0;
}
function readTokenCount(bag, keys) {
  for (const key of keys) {
    const value = bag[key];
    if (typeof value === "number" && Number.isFinite(value)) {
      return value;
    }
  }
  return void 0;
}

// src/version.ts
var VERSION = "0.1.0-pre";

export { SpendGuardCallbackHandler, VERSION };
