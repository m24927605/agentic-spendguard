// src/usage.ts — usage extraction from commit-hook args (implementation.md
// §3.5; COV_D38_03).
//
// ── [VERIFY-AT-IMPL: V4] PINNED (COV_D38_03, @mastra/core 1.41.0) ─────────
// Flat normalized usage fields + where each commit hook exposes them:
//
//   - Field names: `inputTokens` / `outputTokens` (camelCase) — Mastra's
//     `LanguageModelUsage` is `LanguageModelV2Usage & {...}`
//     (dist/stream/types.d.ts), and the loop's `normalizeUsage()` flattens
//     AI SDK v6 / V3-model nested usage ({ inputTokens: { total, ... } })
//     onto the same flat fields, exactly as design §6.6 predicted. Both
//     fields are OPTIONAL (`number | undefined`) on the installed type.
//   - `processOutputStep` exposes usage DIRECTLY: args.usage:
//     LanguageModelUsage (fed from outputStream._getImmediateUsage()).
//   - `processLLMResponse` exposes NO flat usage field on its args; usage
//     rides inside args.chunks — the stripped `{ type: "finish", payload }`
//     model chunk carries `payload.output.usage` (normalizeUsage output).
//     A `response-metadata` chunk, when present, carries the provider
//     response id at `payload.id` (→ providerEventId).
//   - Hook ordering on streamed steps: `processLLMResponse` (input-processor
//     runner — installed .d.ts: "called after the LLM step completes (or a
//     cached response is replayed)"; `fromCache: boolean` flags replays)
//     runs BEFORE `processOutputStep`
//     (output-processor runner) — `processOutputStep` is the LAST commit
//     hook on a streamed step, so it is the §6.1 backstop. NOTE:
//     `processOutputStep` only fires for processors mounted via the Agent's
//     `outputProcessors` list (the V5-pinned `inputProcessors` mount drives
//     `processInputStep`/`processLLMRequest`/`processLLMResponse` only).
//
// Discipline (D04/D06 `extractTokenUsage` parity): accepts camelCase AND
// snake_case shapes, tolerates non-object bags, and returns `undefined`
// (NOT zeros) when usage is absent so the caller selects the §6.6 LOCKED
// estimated-amount fallback.

export interface ExtractedUsage {
  inputTokens: number;
  outputTokens: number;
  providerEventId?: string;
}

/** Read one token field accepting camelCase + snake_case spellings. */
function readTokenField(bag: Record<string, unknown>, camel: string, snake: string): unknown {
  return bag[camel] !== undefined ? bag[camel] : bag[snake];
}

/** Coerce a candidate token count: finite non-negative number, else undefined. */
function asTokenCount(value: unknown): number | undefined {
  return typeof value === "number" && Number.isFinite(value) && value >= 0 ? value : undefined;
}

/** Parse a flat usage bag → ExtractedUsage tokens, or undefined when absent. */
function parseUsageBag(bag: unknown): { inputTokens: number; outputTokens: number } | undefined {
  if (bag === null || typeof bag !== "object") {
    return undefined;
  }
  const record = bag as Record<string, unknown>;
  const inputTokens = asTokenCount(readTokenField(record, "inputTokens", "input_tokens"));
  const outputTokens = asTokenCount(readTokenField(record, "outputTokens", "output_tokens"));
  // BOTH fields must be present numbers — partial usage is treated as absent
  // (the V4-pinned LanguageModelUsage allows `undefined` per field; the
  // §6.6 fallback is safer than fabricating a zero for the missing side).
  if (inputTokens === undefined || outputTokens === undefined) {
    return undefined;
  }
  return { inputTokens, outputTokens };
}

/** Stripped chunk shape carried on ProcessLLMResponseArgs.chunks (V4 pin). */
interface StrippedChunk {
  type?: unknown;
  payload?: unknown;
}

/** Read `payload.output.usage` (model finish chunk) else `payload.usage`. */
function usageFromChunkPayload(payload: unknown): unknown {
  if (payload === null || typeof payload !== "object") {
    return undefined;
  }
  const record = payload as Record<string, unknown>;
  const output = record.output;
  if (output !== null && typeof output === "object") {
    const outputUsage = (output as Record<string, unknown>).usage;
    if (outputUsage !== undefined) {
      return outputUsage;
    }
  }
  return record.usage;
}

/**
 * Extract `(inputTokens, outputTokens[, providerEventId])` from commit-hook
 * args. Handles BOTH installed arg shapes (V4 pin above):
 *
 *   - `ProcessOutputStepArgs`-shaped bags: flat `args.usage`.
 *   - `ProcessLLMResponseArgs`-shaped bags: the last `finish` chunk's
 *     `payload.output.usage` in `args.chunks`; `providerEventId` from a
 *     `response-metadata` chunk's `payload.id` when present.
 *
 * Returns `undefined` (NOT zeros) when no usable usage is exposed.
 */
export function extractUsage(args: unknown): ExtractedUsage | undefined {
  if (args === null || typeof args !== "object") {
    return undefined;
  }
  const record = args as Record<string, unknown>;

  // Shape 1 — processOutputStep: flat usage on the args bag.
  const direct = parseUsageBag(record.usage);
  if (direct !== undefined) {
    return direct;
  }

  // Shape 2 — processLLMResponse: usage inside the stripped finish chunk.
  const chunks = record.chunks;
  if (!Array.isArray(chunks)) {
    return undefined;
  }
  let tokens: { inputTokens: number; outputTokens: number } | undefined;
  let providerEventId: string | undefined;
  for (const chunk of chunks as StrippedChunk[]) {
    if (chunk === null || typeof chunk !== "object") {
      continue;
    }
    if (chunk.type === "finish" || chunk.type === "step-finish") {
      // Last finish wins (multiple finish chunks should not happen; D04
      // stream discipline).
      tokens = parseUsageBag(usageFromChunkPayload(chunk.payload)) ?? tokens;
    } else if (chunk.type === "response-metadata") {
      const payload = chunk.payload;
      if (payload !== null && typeof payload === "object") {
        const id = (payload as Record<string, unknown>).id;
        if (typeof id === "string" && id.length > 0) {
          providerEventId = id;
        }
      }
    }
  }
  if (tokens === undefined) {
    return undefined;
  }
  return providerEventId !== undefined ? { ...tokens, providerEventId } : tokens;
}
