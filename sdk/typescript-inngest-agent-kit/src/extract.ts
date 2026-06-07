// Token-usage + provider-event-id extraction from `step.ai` results.
//
// Inngest AgentKit's `step.ai.infer()` returns the raw provider payload
// directly; the adapter probes it for the canonical `usage.total_tokens`
// fields the major providers expose. Mirrors D04 `extract.ts`.
//
// LOCKED probe order (review-standards §7.1-7.5):
//   1. `result.usage.total_tokens`                          — OpenAI shape
//   2. `result.usage_metadata.total_tokens`                 — Anthropic / Gemini
//   3. `result.response_metadata.token_usage.total_tokens`  — legacy
//   → fall through to 0 (never throws — §7.4)
//
// `providerEventId` order:
//   1. `result.id`
//   2. `result.response_metadata.id`
//   → fall through to `""`

/**
 * Pull a canonical `total_tokens` count out of an opaque provider result.
 *
 * Returns 0 when no recognisable usage payload is present (review-standards
 * §7.4). NEVER throws. Tolerates non-object `usage` fields (review-standards
 * §7.6 — drift tolerance).
 */
export function extractTotalTokens(result: unknown): number {
  if (!isObject(result)) return 0;

  // Probe 1: OpenAI shape — `result.usage.total_tokens`
  const usage = result.usage;
  if (isObject(usage)) {
    const t = usage.total_tokens;
    if (typeof t === "number" && Number.isFinite(t)) return t;
    const tCamel = usage.totalTokens;
    if (typeof tCamel === "number" && Number.isFinite(tCamel)) return tCamel;
  }

  // Probe 2: Anthropic / Gemini shape — `result.usage_metadata.total_tokens`
  const usageMeta = result.usage_metadata ?? result.usageMetadata;
  if (isObject(usageMeta)) {
    const t = usageMeta.total_tokens;
    if (typeof t === "number" && Number.isFinite(t)) return t;
    const tCamel = usageMeta.totalTokens;
    if (typeof tCamel === "number" && Number.isFinite(tCamel)) return tCamel;
  }

  // Probe 3: legacy `result.response_metadata.token_usage.total_tokens`
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

/**
 * Pull a provider event id (commonly the chat-completion id) out of a
 * `step.ai` result.
 *
 * Probe order:
 *   1. `result.id`
 *   2. `result.response_metadata.id` / `result.responseMetadata.id`
 *   → `""`
 *
 * NEVER throws. Returns `""` for any unrecognised shape so the commit path
 * stays wire-safe.
 */
export function extractProviderEventId(result: unknown): string {
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

// ── Internal helpers ──────────────────────────────────────────────────────

function isObject(value: unknown): value is Record<string, unknown> {
  return value !== null && typeof value === "object";
}
