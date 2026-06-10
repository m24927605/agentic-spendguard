import { deriveUuidFromSignature, deriveIdempotencyKey, ApprovalRequired } from '@spendguard/sdk';
export { ApprovalRequired, DecisionDenied, DecisionSkipped, DecisionStopped, SidecarUnavailable, SpendGuardError } from '@spendguard/sdk';

// src/wrapWithSpendGuard.ts

// src/extract.ts
function extractTotalTokens(result) {
  if (!isObject(result)) return 0;
  const usage = result.usage;
  if (isObject(usage)) {
    const t = usage.total_tokens;
    if (typeof t === "number" && Number.isFinite(t)) return t;
    const tCamel = usage.totalTokens;
    if (typeof tCamel === "number" && Number.isFinite(tCamel)) return tCamel;
  }
  const usageMeta = result.usage_metadata ?? result.usageMetadata;
  if (isObject(usageMeta)) {
    const t = usageMeta.total_tokens;
    if (typeof t === "number" && Number.isFinite(t)) return t;
    const tCamel = usageMeta.totalTokens;
    if (typeof tCamel === "number" && Number.isFinite(tCamel)) return tCamel;
  }
  const rmeta = result.response_metadata ?? result.responseMetadata;
  if (isObject(rmeta)) {
    const tokenUsage = rmeta.token_usage ?? rmeta.tokenUsage;
    if (isObject(tokenUsage)) {
      const t = tokenUsage.total_tokens;
      if (typeof t === "number" && Number.isFinite(t)) return t;
      const tCamel = tokenUsage.totalTokens;
      if (typeof tCamel === "number" && Number.isFinite(tCamel)) return tCamel;
    }
  }
  return 0;
}
function extractProviderEventId(result) {
  if (!isObject(result)) return "";
  const topId = result.id;
  if (typeof topId === "string" && topId.length > 0) return topId;
  const rmeta = result.response_metadata ?? result.responseMetadata;
  if (isObject(rmeta)) {
    const id = rmeta.id;
    if (typeof id === "string" && id.length > 0) return id;
  }
  return "";
}
function isObject(value) {
  return value !== null && typeof value === "object";
}
function deriveIdentity(args) {
  const seed = args.input.inngestIdempotencyKey ?? args.input.stepId;
  const decisionId = deriveUuidFromSignature(seed, { scope: "decision_id" });
  const stepId = args.input.stepId;
  const llmCallId = args.input.stepId;
  const idempotencyKey = deriveIdempotencyKey({
    tenantId: args.tenantId,
    sessionId: args.input.runId,
    runId: args.input.runId,
    stepId,
    llmCallId,
    trigger: "LLM_CALL_PRE"
  });
  return { decisionId, idempotencyKey, llmCallId, stepId };
}
function deriveStepIdempotencyKey(args) {
  void (args.inngestIdempotencyKey ?? args.stepId);
  return deriveIdempotencyKey({
    tenantId: args.tenantId,
    sessionId: args.runId,
    runId: args.runId,
    stepId: args.stepId,
    llmCallId: args.stepId,
    trigger: "LLM_CALL_PRE"
  });
}

// src/wrapWithSpendGuard.ts
var DEFAULT_ROUTE = "llm.call.inngest";
var DEFAULT_UNIT = { unit: "USD_MICROS", denomination: 1 };
var EMPTY_PRICING = {
  pricingVersion: "",
  pricingHash: new Uint8Array(0)
};
var TRIGGER_PRE = "LLM_CALL_PRE";
function wrapWithSpendGuard(stepAi, client, options) {
  validateOptions(options);
  const route = options.route ?? DEFAULT_ROUTE;
  const unit = options.unit ?? DEFAULT_UNIT;
  const pricing = options.pricing ?? EMPTY_PRICING;
  const tenantId = options.tenantId;
  async function runReserveAndCommit(body, inputBuilder) {
    const input = inputBuilder();
    const id = deriveIdentity({ tenantId, input });
    const claims = projectClaims(input);
    const commitUnit = claims[0]?.unit ?? unit;
    let outcome;
    if (options.idempotencyCache !== void 0) {
      const cached = options.idempotencyCache.get(id.idempotencyKey);
      if (cached !== void 0) {
        outcome = cached;
      }
    }
    if (outcome === void 0) {
      try {
        outcome = await client.reserve(buildReserveRequest(input, id, claims));
      } catch (err) {
        if (err instanceof ApprovalRequired && options.onApprovalRequired !== void 0) {
          const resumed = await options.onApprovalRequired(err, input);
          if (resumed === null || resumed === void 0) {
            throw err;
          }
          outcome = resumed;
        } else {
          throw err;
        }
      }
      if (options.idempotencyCache !== void 0 && outcome !== void 0) {
        options.idempotencyCache.set(id.idempotencyKey, outcome);
      }
    }
    try {
      const result = await body();
      const totalTokens = extractTotalTokens(result);
      const providerEventId = extractProviderEventId(result);
      try {
        await client.commitEstimated(
          buildCommitRequest(input, id, outcome, {
            outcomeStatus: "SUCCESS",
            estimatedAmountAtomic: String(totalTokens),
            providerEventId,
            // HARDEN_D05_WI — reuse the reserve-time unit so payload.unit_id
            // matches the reservation.
            unit: commitUnit,
            pricing
          })
        );
      } catch (commitErr) {
        const reason = commitErr instanceof Error ? commitErr.message : String(commitErr);
        console.warn(
          `[spendguard:inngest-agent-kit] SUCCESS commit failed for stepId=${id.stepId}; provider result preserved (${reason})`
        );
      }
      return result;
    } catch (providerErr) {
      try {
        await client.commitEstimated(
          buildCommitRequest(input, id, outcome, {
            outcomeStatus: "PROVIDER_ERROR",
            estimatedAmountAtomic: "0",
            providerEventId: "",
            // HARDEN_D05_WI — reserve-time unit + freeze tuple must match
            // the reservation even on the PROVIDER_ERROR commit path.
            unit: commitUnit,
            pricing,
            errorMessage: providerErr instanceof Error ? providerErr.message : String(providerErr)
          })
        );
      } catch (commitErr) {
        const reason = commitErr instanceof Error ? commitErr.message : String(commitErr);
        console.warn(
          `[spendguard:inngest-agent-kit] PROVIDER_ERROR commit failed for stepId=${id.stepId}; original provider error preserved (${reason})`
        );
      }
      throw providerErr;
    }
  }
  function buildReserveRequest(input, id, claims) {
    const req = {
      trigger: TRIGGER_PRE,
      runId: input.runId,
      stepId: id.stepId,
      llmCallId: id.llmCallId,
      decisionId: id.decisionId,
      route,
      projectedClaims: claims,
      idempotencyKey: id.idempotencyKey
    };
    if (options.claimEstimate !== void 0) {
      req.claimEstimate = options.claimEstimate;
    }
    if (options.windowInstanceId !== void 0) {
      req.windowInstanceId = options.windowInstanceId;
    }
    return req;
  }
  function buildCommitRequest(input, id, outcome, extras) {
    const req = {
      runId: input.runId,
      stepId: id.stepId,
      llmCallId: id.llmCallId,
      decisionId: outcome.decisionId,
      reservationId: outcome.reservationIds[0] ?? "",
      estimatedAmountAtomic: extras.estimatedAmountAtomic,
      unit: extras.unit,
      pricing: extras.pricing,
      providerEventId: extras.providerEventId,
      outcome: extras.outcomeStatus
    };
    if (extras.errorMessage !== void 0) {
      req.actualErrorMessage = extras.errorMessage;
    }
    return req;
  }
  function projectClaims(input) {
    if (options.claimEstimator !== void 0) {
      return applyEstimateOverride([...options.claimEstimator(input)]);
    }
    const claimUnit = options.unitId ? { ...unit, unitId: options.unitId } : unit;
    return applyEstimateOverride([
      {
        scopeId: options.budgetId ?? tenantId,
        amountAtomic: "0",
        unit: claimUnit,
        // HARDEN_D05_WI — thread caller-supplied windowInstanceId onto the
        // wire claim (substrate coerces omitted to "").
        ...options.windowInstanceId ? { windowInstanceId: options.windowInstanceId } : {}
      }
    ]);
  }
  function applyEstimateOverride(claims) {
    const override = options.estimateOverrideAtomic;
    if (override !== void 0 && /^[0-9]+$/.test(override)) {
      return claims.map((claim) => ({ ...claim, amountAtomic: override }));
    }
    return claims;
  }
  function inputFromCtx(ctx, name, model, body) {
    const stepId = ctx?.step.id ?? name;
    const input = {
      stepId,
      attempt: ctx?.step.attempt ?? 0,
      runId: ctx?.runId ?? "",
      model,
      body
    };
    if (ctx?.step.idempotencyKey !== void 0) {
      input.inngestIdempotencyKey = ctx.step.idempotencyKey;
    }
    if (ctx?.eventId !== void 0) {
      input.eventId = ctx.eventId;
    }
    return input;
  }
  return {
    async infer(name, opts, runtimeCtx) {
      const ctx = runtimeCtx;
      return runReserveAndCommit(
        () => stepAi.infer(name, opts, runtimeCtx),
        () => inputFromCtx(ctx, name, opts.model, opts.body)
      );
    },
    wrap(name, fn, ...args) {
      const maybeCtx = args[args.length - 1] ?? void 0;
      const ctx = maybeCtx !== void 0 && typeof maybeCtx === "object" && "step" in maybeCtx ? maybeCtx : void 0;
      return runReserveAndCommit(
        () => stepAi.wrap(name, fn, ...args),
        () => inputFromCtx(ctx, name, void 0, args)
      );
    }
  };
}
function validateOptions(opts) {
  if (opts === null || typeof opts !== "object") {
    throw new TypeError("wrapWithSpendGuard: opts must be an object");
  }
  if (typeof opts.tenantId !== "string" || opts.tenantId.length === 0) {
    throw new TypeError("wrapWithSpendGuard: opts.tenantId is required (non-empty string)");
  }
}

// src/version.ts
var VERSION = "0.1.0";

export { VERSION, deriveIdentity, deriveStepIdempotencyKey, extractProviderEventId, extractTotalTokens, wrapWithSpendGuard };
