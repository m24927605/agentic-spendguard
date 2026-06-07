// Botpress `event.payload.usage` → SpendGuard real-usage adapter.
//
// review-standards.md §3.9 / A06 / A07 — Botpress's normalised usage shape
// `{ inputTokens, outputTokens }` covers OpenAI's `prompt_tokens` /
// `completion_tokens`, Anthropic's `input_tokens` / `output_tokens`, and
// Bedrock's per-vendor shape (the @botpress/sdk 0.7 client normalises
// before emitting the afterAiGeneration hook).
//
// If Botpress's normalisation regresses or the upstream provider returns no
// usage at all, this module returns `undefined` so the caller hits the
// estimator-snapshot fallback in `SpendGuardReservation.commitSuccess`.

import type { ReservationHandle } from "../reservation.js";

/** Loose-shape Botpress afterAiGeneration `data` we extract usage from. */
export interface BotpressAfterHookData {
  /** Botpress 0.7 normalised shape. */
  readonly payload?: {
    readonly usage?: {
      readonly inputTokens?: number;
      readonly outputTokens?: number;
    };
  };
  /** Some 0.7.x patch versions emit usage at the top of `data`. */
  readonly usage?: {
    readonly inputTokens?: number;
    readonly outputTokens?: number;
  };
  /** Upstream-event-style payload occasionally surfaces the provider's
   *  raw response under `data.response.usage`. We sniff this last so the
   *  per-provider fallback path stays additive. */
  readonly response?: {
    readonly usage?: {
      readonly input_tokens?: number;
      readonly output_tokens?: number;
      readonly prompt_tokens?: number;
      readonly completion_tokens?: number;
    };
  };
  /** Provider event id for dedup at canonical ingest. */
  readonly providerEventId?: string;
}

/**
 * Extract `{ inputTokens, outputTokens }` from Botpress's afterAiGeneration
 * data. Returns `undefined` when none of the recognised shapes is present.
 */
export function extractUsageFromBotpressEvent(
  data: BotpressAfterHookData | undefined,
): { inputTokens: number; outputTokens: number } | undefined {
  if (data === undefined) return undefined;
  const primary = data.payload?.usage ?? data.usage;
  if (primary !== undefined) {
    if (typeof primary.inputTokens === "number" || typeof primary.outputTokens === "number") {
      return {
        inputTokens: primary.inputTokens ?? 0,
        outputTokens: primary.outputTokens ?? 0,
      };
    }
  }
  const raw = data.response?.usage;
  if (raw !== undefined) {
    const inputTokens = raw.input_tokens ?? raw.prompt_tokens;
    const outputTokens = raw.output_tokens ?? raw.completion_tokens;
    if (typeof inputTokens === "number" || typeof outputTokens === "number") {
      return {
        inputTokens: inputTokens ?? 0,
        outputTokens: outputTokens ?? 0,
      };
    }
  }
  return undefined;
}

/** Convenience: snapshot → usage shape for the estimator fallback path
 *  (review-standards.md §3.10 / INV-5 secondary). */
export function snapshotToUsage(snapshot: ReservationHandle["estimatorSnapshot"]): {
  inputTokens: number;
  outputTokens: number;
} {
  return {
    inputTokens: snapshot.inputTokens,
    outputTokens: snapshot.outputTokens,
  };
}

/** Convenience: pull `providerEventId` from the loose afterAiGeneration
 *  data. Empty string is the wire-stable "missing" sentinel that lines
 *  up with the Kong-shaped trace payload's `provider_event_id` field. */
export function pickProviderEventId(data: BotpressAfterHookData | undefined): string {
  return data?.providerEventId ?? "";
}
