import { DecisionDenied, deriveUuidFromSignature, deriveIdempotencyKey as deriveIdempotencyKey$1 } from '@spendguard/sdk';
export { DecisionDenied, SidecarUnavailable, SpendGuardError } from '@spendguard/sdk';

// src/middleware.ts
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

// src/wrapper.ts
var STEP_ID_LLM_CALL = "llm_call";
var DEFAULT_UNIT = { unit: "USD_MICROS", denomination: 1 };
var EMPTY_PRICING = {
  pricingVersion: "",
  pricingHash: new Uint8Array(0)
};
function makeWrapGenerate(client, lookupStash) {
  return async ({
    doGenerate,
    params
  }) => {
    const entry = lookupStash(params);
    if (entry === void 0) {
      return doGenerate();
    }
    try {
      const result = await doGenerate();
      const usage = extractUsageFromGenerate(result);
      await safeCommit(client, entry, {
        outcomeKind: "SUCCESS",
        outcome: "SUCCESS",
        actualInputTokensWire: String(usage.promptTokens),
        actualOutputTokensWire: String(usage.completionTokens)
      });
      return result;
    } catch (err) {
      const message = err instanceof Error ? err.message : String(err);
      await safeCommit(client, entry, {
        outcomeKind: "FAILURE",
        outcome: "PROVIDER_ERROR",
        actualErrorMessage: message
      });
      throw err;
    }
  };
}
function makeWrapStream(client, lookupStash) {
  return async ({
    doStream,
    params
  }) => {
    const entry = lookupStash(params);
    const inner = await doStream();
    if (entry === void 0) {
      return inner;
    }
    const instrumented = instrumentStream(inner.stream, async (kind, ctx) => {
      if (kind === "finish") {
        await safeCommit(client, entry, {
          outcomeKind: "SUCCESS",
          outcome: "SUCCESS",
          actualInputTokensWire: String(ctx.promptTokens),
          actualOutputTokensWire: String(ctx.completionTokens)
        });
      } else {
        await safeCommit(client, entry, {
          outcomeKind: "FAILURE",
          outcome: "PROVIDER_ERROR",
          actualErrorMessage: ctx.errorMessage
        });
      }
    });
    return { ...inner, stream: instrumented };
  };
}
function instrumentStream(inner, onTerminal) {
  let terminal = false;
  let lastPromptTokens = 0;
  let lastCompletionTokens = 0;
  const transform = new TransformStream({
    transform(part, controller) {
      controller.enqueue(part);
      if (part.type === "finish") {
        const usage = extractUsageFromStreamPart(part);
        if (usage !== void 0) {
          lastPromptTokens = usage.promptTokens;
          lastCompletionTokens = usage.completionTokens;
        }
      } else if (part.type === "error") {
        if (!terminal) {
          terminal = true;
          const message = part.error instanceof Error ? part.error.message : String(part.error);
          void onTerminal("error", {
            promptTokens: 0,
            completionTokens: 0,
            errorMessage: message
          }).catch((commitErr) => {
            console.warn(
              `[spendguard:vercel-ai] stream FAILURE commit threw: ${commitErr instanceof Error ? commitErr.message : String(commitErr)}`
            );
          });
        }
      }
    },
    async flush() {
      if (terminal) return;
      terminal = true;
      try {
        await onTerminal("finish", {
          promptTokens: lastPromptTokens,
          completionTokens: lastCompletionTokens,
          errorMessage: ""
        });
      } catch (commitErr) {
        console.warn(
          `[spendguard:vercel-ai] stream SUCCESS commit threw: ${commitErr instanceof Error ? commitErr.message : String(commitErr)}`
        );
      }
    }
  });
  const piped = inner.pipeThrough(transform);
  return new ReadableStream({
    async start(controller) {
      const reader = piped.getReader();
      try {
        for (; ; ) {
          const { value, done } = await reader.read();
          if (done) break;
          controller.enqueue(value);
        }
        controller.close();
      } catch (err) {
        if (!terminal) {
          terminal = true;
          const message = err instanceof Error ? err.message : String(err);
          try {
            await onTerminal("error", {
              promptTokens: 0,
              completionTokens: 0,
              errorMessage: message
            });
          } catch (commitErr) {
            console.warn(
              `[spendguard:vercel-ai] stream FAILURE commit threw: ${commitErr instanceof Error ? commitErr.message : String(commitErr)}`
            );
          }
        }
        controller.error(err);
      } finally {
        reader.releaseLock();
      }
    },
    async cancel(reason) {
      if (!terminal) {
        terminal = true;
        const message = reason instanceof Error ? reason.message : reason !== void 0 ? String(reason) : "stream cancelled";
        try {
          await onTerminal("error", {
            promptTokens: 0,
            completionTokens: 0,
            errorMessage: message
          });
        } catch (commitErr) {
          console.warn(
            `[spendguard:vercel-ai] stream cancel FAILURE commit threw: ${commitErr instanceof Error ? commitErr.message : String(commitErr)}`
          );
        }
      }
    }
  });
}
async function safeCommit(client, entry, outcome) {
  const req = {
    runId: entry.runId,
    stepId: STEP_ID_LLM_CALL,
    llmCallId: entry.runId,
    decisionId: entry.decisionId,
    reservationId: entry.reservationId,
    estimatedAmountAtomic: "0",
    unit: DEFAULT_UNIT,
    pricing: EMPTY_PRICING,
    providerEventId: "",
    outcome: outcome.outcome,
    outcomeKind: outcome.outcomeKind,
    ...outcome.outcomeKind === "SUCCESS" ? {
      actualInputTokensWire: outcome.actualInputTokensWire,
      actualOutputTokensWire: outcome.actualOutputTokensWire
    } : { actualErrorMessage: outcome.actualErrorMessage }
  };
  try {
    await client.commitEstimated(req);
  } catch (commitErr) {
    console.warn(
      `[spendguard:vercel-ai] commitEstimated(${outcome.outcomeKind}) threw for runId=${entry.runId}: ${commitErr instanceof Error ? commitErr.message : String(commitErr)}`
    );
  }
}
function extractUsageFromGenerate(result) {
  if (result === null || typeof result !== "object") {
    return { promptTokens: 0, completionTokens: 0 };
  }
  const bag = result;
  return extractUsageFromBag(bag.usage);
}
function extractUsageFromStreamPart(part) {
  const usage = part.usage;
  if (usage === void 0 || usage === null) return void 0;
  return extractUsageFromBag(usage);
}
function extractUsageFromBag(bag) {
  if (bag === null || typeof bag !== "object") {
    return { promptTokens: 0, completionTokens: 0 };
  }
  const obj = bag;
  const prompt = readNumeric(obj, ["promptTokens", "prompt_tokens"]);
  const completion = readNumeric(obj, ["completionTokens", "completion_tokens"]);
  return {
    promptTokens: prompt ?? 0,
    completionTokens: completion ?? 0
  };
}
function readNumeric(bag, keys) {
  for (const key of keys) {
    const value = bag[key];
    if (typeof value === "number" && Number.isFinite(value)) {
      return value;
    }
  }
  return void 0;
}

// src/middleware.ts
var DEFAULT_ROUTE = "vercel-ai-llm";
var DEFAULT_UNIT2 = { unit: "USD_MICROS", denomination: 1 };
var STEP_ID_LLM_CALL2 = "llm_call";
var CHARS_PER_TOKEN_HEURISTIC = 4;
var DEFAULT_MICROS_PER_TOKEN = 1000n;
var RUN_ID_SCOPE = "vercel_ai_run_id";
var STASH = /* @__PURE__ */ new WeakMap();
function createSpendGuardMiddleware(opts) {
  validateOpts(opts);
  return {
    middlewareVersion: "v1",
    transformParams: async ({ params }) => {
      const runId = deriveRunId(params, opts.tenantId);
      const idempotencyKey = deriveIdempotencyKey({
        tenantId: opts.tenantId,
        runId
      });
      const req = {
        trigger: "LLM_CALL_PRE",
        runId,
        stepId: STEP_ID_LLM_CALL2,
        llmCallId: runId,
        decisionId: runId,
        route: DEFAULT_ROUTE,
        projectedClaims: [projectClaim(params, opts)],
        idempotencyKey
      };
      let outcome;
      try {
        outcome = await opts.client.reserve(req);
      } catch (err) {
        if (err instanceof DecisionDenied) {
          throw err;
        }
        const reason = err instanceof Error ? err.message : String(err);
        console.warn(
          `[spendguard:vercel-ai] reserve() failed for runId=${runId}; LLM call proceeds without budget gate (${reason})`
        );
        return params;
      }
      STASH.set(params, {
        decisionId: outcome.decisionId,
        reservationId: outcome.reservationIds[0] ?? "",
        runId,
        idempotencyKey
      });
      return params;
    },
    // SLICE 4 + SLICE 5 wire the real commit / release paths via the stash
    // lookup pointer (avoids an import cycle with `./wrapper.js`). The
    // factories build hook callbacks typed against AI SDK v4's
    // `LanguageModelV1Middleware` shape.
    wrapGenerate: makeWrapGenerate(
      opts.client,
      (params) => STASH.get(params)
    ),
    wrapStream: makeWrapStream(
      opts.client,
      (params) => STASH.get(params)
    )
  };
}
function validateOpts(opts) {
  if (opts === null || typeof opts !== "object") {
    throw new TypeError("createSpendGuardMiddleware: opts must be an object");
  }
  if (!opts.client) {
    throw new TypeError("createSpendGuardMiddleware: opts.client is required");
  }
  if (typeof opts.tenantId !== "string" || opts.tenantId.length === 0) {
    throw new TypeError("createSpendGuardMiddleware: opts.tenantId is required (non-empty string)");
  }
}
function deriveRunId(params, tenantId) {
  const promptText = flattenPromptText(params.prompt);
  const signature = `v1|${tenantId}|${promptText}`;
  return deriveUuidFromSignature(signature, { scope: RUN_ID_SCOPE });
}
function flattenPromptText(prompt) {
  const out = [];
  for (const msg of prompt) {
    if (msg.role === "system") {
      out.push(msg.content);
      continue;
    }
    if (msg.role === "tool") {
      continue;
    }
    for (const part of msg.content) {
      if (part.type === "text") {
        out.push(part.text);
      }
    }
  }
  return out.join("\n");
}
function projectClaim(params, opts) {
  const totalChars = flattenPromptText(params.prompt).length;
  const estimatedTokens = BigInt(Math.max(1, Math.ceil(totalChars / CHARS_PER_TOKEN_HEURISTIC)));
  const amountMicros = estimatedTokens * DEFAULT_MICROS_PER_TOKEN;
  return {
    scopeId: opts.budgetId ?? opts.tenantId,
    amountAtomic: amountMicros.toString(),
    unit: DEFAULT_UNIT2
  };
}

// src/version.ts
var VERSION = "0.1.0";

export { VERSION, createSpendGuardMiddleware };
