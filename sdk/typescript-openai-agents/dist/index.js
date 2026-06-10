import { currentRunContext } from './chunk-5A3754OQ.js';
export { currentRunContext, runContext } from './chunk-5A3754OQ.js';
import { deriveUuidFromSignature, deriveIdempotencyKey } from '@spendguard/sdk';
export { ApprovalRequired, DecisionDenied, DecisionStopped, SidecarUnavailable, SpendGuardError } from '@spendguard/sdk';
import { blake2b } from '@noble/hashes/blake2b';
import { bytesToHex } from '@noble/hashes/utils';

// src/defaultEstimator.ts
var MODEL_BASELINE_TOKENS = Object.freeze({
  "gpt-4o-mini": 500,
  "gpt-4o": 1500,
  "gpt-4.1-mini": 500,
  "gpt-4.1": 1500,
  o1: 3e3,
  "o3-mini": 1500,
  o3: 3e3
});
var DEFAULT_BASELINE_TOKENS = 800;
function resolveBaselineTokens(modelName) {
  return MODEL_BASELINE_TOKENS[modelName] ?? DEFAULT_BASELINE_TOKENS;
}
function defaultClaimEstimator(opts) {
  const baseline = resolveBaselineTokens(opts.modelName);
  const amountAtomic = String(baseline);
  return (_input) => [
    {
      scopeId: opts.scopeId,
      amountAtomic,
      unit: opts.unit,
      // HARDEN_D05_WI — thread caller-supplied windowInstanceId onto the
      // wire claim (substrate coerces omitted to "").
      ...opts.windowInstanceId ? { windowInstanceId: opts.windowInstanceId } : {}
    }
  ];
}
function deriveAgentSignature(input, systemInstructions) {
  const repr = renderInputCanonical(input);
  const sysSegment = systemInstructions == null ? "" : systemInstructions;
  const text = `${repr}|${sysSegment}`;
  return bytesToHex(blake2b(text, { dkLen: 16 }));
}
function renderInputCanonical(input) {
  if (typeof input === "string") {
    const escaped = input.replace(/\\/g, "\\\\").replace(/'/g, "\\'");
    return `'${escaped}'`;
  }
  const json = JSON.stringify(input);
  return json ?? "null";
}

// src/usage.ts
function extractUsage(response) {
  if (!response) {
    return zeroUsage();
  }
  const raw = response.usage;
  if (!raw || typeof raw !== "object") {
    return zeroUsage();
  }
  const inputTokens = toFiniteNumber(raw.inputTokens) ?? toFiniteNumber(raw.prompt_tokens) ?? 0;
  const outputTokens = toFiniteNumber(raw.outputTokens) ?? toFiniteNumber(raw.completion_tokens) ?? 0;
  const totalCandidate = toFiniteNumber(raw.totalTokens) ?? toFiniteNumber(raw.total_tokens);
  const totalTokens = totalCandidate ?? inputTokens + outputTokens;
  return { inputTokens, outputTokens, totalTokens };
}
function zeroUsage() {
  return { inputTokens: 0, outputTokens: 0, totalTokens: 0 };
}
function toFiniteNumber(value) {
  if (value == null) {
    return void 0;
  }
  if (typeof value === "number") {
    return Number.isFinite(value) ? value : void 0;
  }
  if (typeof value === "string") {
    const trimmed = value.trim();
    if (trimmed === "") {
      return void 0;
    }
    const parsed = Number(trimmed);
    return Number.isFinite(parsed) ? parsed : void 0;
  }
  return void 0;
}

// src/core.ts
var DEFAULT_ROUTE = "llm.call";
var DEFAULT_UNIT = { unit: "USD_MICROS", denomination: 1 };
var EMPTY_PRICING = {
  pricingVersion: "",
  pricingHash: new Uint8Array(0)
};
var TRIGGER_PRE = "LLM_CALL_PRE";
var SCOPE_DECISION_ID = "decision_id";
var SCOPE_LLM_CALL_ID = "llm_call_id";
async function bracketedGetResponse(inner, request, opts, innerModelName) {
  const ctx = currentRunContext();
  const sig = deriveAgentSignature(request.input, request.systemInstructions);
  const llmCallId = deriveUuidFromSignature(sig, { scope: SCOPE_LLM_CALL_ID });
  const decisionId = deriveUuidFromSignature(sig, { scope: SCOPE_DECISION_ID });
  const stepId = `${ctx.runId}:oai-call:${sig.slice(0, 16)}`;
  const idempotencyKey = deriveIdempotencyKey({
    tenantId: opts.tenantId,
    sessionId: ctx.runId,
    runId: ctx.runId,
    stepId,
    llmCallId,
    trigger: TRIGGER_PRE
  });
  const claims = projectClaimsSlice3(request, opts, innerModelName);
  const req = {
    trigger: TRIGGER_PRE,
    runId: ctx.runId,
    stepId,
    llmCallId,
    decisionId,
    route: DEFAULT_ROUTE,
    projectedClaims: claims,
    idempotencyKey
  };
  const outcome = await opts.client.reserve(req);
  let response;
  let providerError;
  try {
    response = await inner.getResponse(request);
  } catch (err) {
    providerError = err;
    response = void 0;
  }
  if (outcome.reservationIds.length > 0) {
    const reservationId = outcome.reservationIds[0] ?? "";
    if (providerError) {
      await safeCommit(opts, {
        runId: ctx.runId,
        stepId,
        llmCallId,
        decisionId: outcome.decisionId,
        reservationId,
        estimatedAmountAtomic: "0",
        // HARDEN_D05_WI — reserve-time unit + freeze tuple must match the
        // reservation even on the PROVIDER_ERROR commit path.
        unit: claims[0]?.unit ?? DEFAULT_UNIT,
        pricing: opts.pricing ?? EMPTY_PRICING,
        providerEventId: "",
        outcome: "PROVIDER_ERROR"
      });
      throw providerError;
    }
    const usage = extractUsage(response);
    const providerEventId = response?.responseId ?? response?.requestId ?? "";
    await safeCommit(opts, {
      runId: ctx.runId,
      stepId,
      llmCallId,
      decisionId: outcome.decisionId,
      reservationId,
      estimatedAmountAtomic: String(usage.totalTokens),
      // HARDEN_D05_WI — reuse the reserve-time unit so payload.unit_id matches
      // the reservation (ledger rejects mismatched commit units).
      unit: claims[0]?.unit ?? DEFAULT_UNIT,
      // HARDEN_D05_WI — repeat the reserve-time freeze tuple (ledger rejects
      // commits whose pricing tuple differs from the reservation's).
      pricing: opts.pricing ?? EMPTY_PRICING,
      providerEventId,
      outcome: "SUCCESS"
    });
  } else if (providerError) {
    throw providerError;
  }
  return response;
}
function projectClaimsSlice3(request, opts, innerModelName) {
  const scopeId = opts.budgetId ?? opts.tenantId;
  const unit = opts.unitId ? { ...DEFAULT_UNIT, unitId: opts.unitId } : DEFAULT_UNIT;
  const claims = defaultClaimEstimator({
    scopeId,
    unit,
    modelName: innerModelName,
    // HARDEN_D05_WI — thread caller-supplied windowInstanceId.
    ...opts.windowInstanceId ? { windowInstanceId: opts.windowInstanceId } : {}
  })(request.input);
  const override = opts.estimateOverrideAtomic;
  if (override !== void 0 && /^[0-9]+$/.test(override)) {
    return claims.map((claim) => ({ ...claim, amountAtomic: override }));
  }
  return claims;
}
async function safeCommit(opts, req) {
  try {
    await opts.client.commitEstimated(req);
  } catch (err) {
    const reason = err instanceof Error ? err.message : String(err);
    console.warn(
      `[spendguard:openai-agents] commitEstimated() failed for runId=${req.runId}; inner response preserved (${reason})`
    );
  }
}

// src/withSpendGuard.ts
function withSpendGuard(inner, opts) {
  validateOpts(opts);
  const innerModelName = inner.model ?? "";
  const wrapped = {
    async getResponse(request) {
      return bracketedGetResponse(inner, request, opts, innerModelName);
    },
    /**
     * Stream pass-through. v0.1.x scope: NO PRE/POST gating around the
     * stream. POST_D08 / v0.2 will add per-chunk gating when the substrate's
     * `LLM_STREAM_DELTA` trigger ships. Documented in reviewer gate 1.5.
     */
    getStreamedResponse(request) {
      return inner.getStreamedResponse(request);
    }
  };
  if (typeof inner.getRetryAdvice === "function") {
    wrapped.getRetryAdvice = inner.getRetryAdvice.bind(inner);
  }
  return wrapped;
}
function validateOpts(opts) {
  if (opts === null || typeof opts !== "object") {
    throw new TypeError("withSpendGuard: opts must be an object");
  }
  if (!opts.client) {
    throw new TypeError("withSpendGuard: opts.client is required");
  }
  if (typeof opts.tenantId !== "string" || opts.tenantId.length === 0) {
    throw new TypeError("withSpendGuard: opts.tenantId is required (non-empty string)");
  }
}

// src/model.ts
var SpendGuardAgentsModel = class {
  inner;
  opts;
  innerModelName;
  /**
   * Construct a `SpendGuardAgentsModel`. Throws `TypeError` synchronously
   * when `inner` / `opts.client` / `opts.tenantId` are missing — surfaces
   * misconfiguration at construction rather than on the first call.
   */
  constructor(opts) {
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
    const { inner: _strip, ...rest } = opts;
    this.opts = rest;
    this.innerModelName = opts.inner.model ?? "";
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
  async getResponse(request) {
    return bracketedGetResponse(this.inner, request, this.opts, this.innerModelName);
  }
  /**
   * Stream pass-through. v0.1.x scope: NO PRE/POST gating. POST_D08 /
   * v0.2 will land per-chunk gating once the substrate's
   * `LLM_STREAM_DELTA` trigger ships.
   */
  getStreamedResponse(request) {
    return this.inner.getStreamedResponse(request);
  }
  /**
   * Forward `getRetryAdvice` to the inner model. The optional retry-advice
   * hook is consulted by the Agents Runner when an LLM call fails; the
   * adapter has no opinion of its own on retry policy at v0.1.x.
   */
  getRetryAdvice(args) {
    if (typeof this.inner.getRetryAdvice === "function") {
      return this.inner.getRetryAdvice(args);
    }
    return void 0;
  }
};

// src/version.ts
var VERSION = "0.1.0";

export { SpendGuardAgentsModel, VERSION, deriveAgentSignature, extractUsage, withSpendGuard };
