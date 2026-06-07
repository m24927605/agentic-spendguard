// `extractUsage` — pull token usage from an OpenAI Agents `ModelResponse`.
//
// The Agents SDK's `ModelResponse.usage` is a `Usage` class instance with
// `inputTokens` / `outputTokens` / `totalTokens` / `requests` number fields
// (see `@openai/agents-core/dist/usage.d.ts`). For interop with custom
// providers / mocked responses the extractor also accepts:
//   - `usage.totalTokens` as a `string` (some providers serialize numbers as
//     strings to dodge JS BigInt rounding for large counts);
//   - snake_case shapes (`prompt_tokens` / `completion_tokens` /
//     `total_tokens`) on the off-chance a custom `Model` impl passes a raw
//     provider response through verbatim.
//
// `totalTokens` is the primary commit field — the substrate's
// `CommitEstimatedRequest.estimatedAmountAtomic` consumes it as `string`
// (the wire is int64-as-string). When usage is missing or unparseable,
// `extractUsage()` returns the safe zero shape so the commit still fires
// — review-standards.md §10.2 / §10.5 ("no swallowing, no inventing").

import type { ModelResponse } from "@openai/agents";

/**
 * Extracted token totals from a `ModelResponse`. All fields are numbers in
 * canonical token units. Missing / unparseable usage degrades to `0`.
 */
export interface ExtractedUsage {
  inputTokens: number;
  outputTokens: number;
  totalTokens: number;
}

interface RawUsageBag {
  inputTokens?: number | string;
  outputTokens?: number | string;
  totalTokens?: number | string;
  // Snake_case mirror — kept for providers that pass raw OpenAI HTTP shape
  // through without re-shaping into Usage.
  prompt_tokens?: number | string;
  completion_tokens?: number | string;
  total_tokens?: number | string;
}

/**
 * Pull canonical token counts from an OpenAI Agents `ModelResponse`.
 *
 * @param response - The response returned by `inner.getResponse(...)`. Only
 *   `.usage` is read; the rest of the response passes through verbatim.
 * @returns `{ inputTokens, outputTokens, totalTokens }` — each safe-zero
 *   on missing or malformed data.
 */
export function extractUsage(response: ModelResponse | undefined | null): ExtractedUsage {
  if (!response) {
    return zeroUsage();
  }
  const raw = (response as { usage?: RawUsageBag }).usage;
  if (!raw || typeof raw !== "object") {
    return zeroUsage();
  }

  const inputTokens = toFiniteNumber(raw.inputTokens) ?? toFiniteNumber(raw.prompt_tokens) ?? 0;
  const outputTokens =
    toFiniteNumber(raw.outputTokens) ?? toFiniteNumber(raw.completion_tokens) ?? 0;
  // Prefer the explicit total. Fall back to (input + output) when missing —
  // some providers ship per-side counts only.
  const totalCandidate = toFiniteNumber(raw.totalTokens) ?? toFiniteNumber(raw.total_tokens);
  const totalTokens = totalCandidate ?? inputTokens + outputTokens;

  return { inputTokens, outputTokens, totalTokens };
}

function zeroUsage(): ExtractedUsage {
  return { inputTokens: 0, outputTokens: 0, totalTokens: 0 };
}

function toFiniteNumber(value: number | string | undefined | null): number | undefined {
  if (value == null) {
    return undefined;
  }
  if (typeof value === "number") {
    return Number.isFinite(value) ? value : undefined;
  }
  if (typeof value === "string") {
    const trimmed = value.trim();
    if (trimmed === "") {
      return undefined;
    }
    const parsed = Number(trimmed);
    return Number.isFinite(parsed) ? parsed : undefined;
  }
  return undefined;
}
