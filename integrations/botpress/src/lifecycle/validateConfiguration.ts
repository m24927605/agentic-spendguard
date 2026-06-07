// `validateConfiguration` — Botpress register lifecycle hook.
//
// review-standards.md §2.7 + INV-4: issues a 1-token reserve + release
// roundtrip via the same `SpendGuardReservation.reserve` /
// `releaseFailure` codepath. This proves the sidecar wiring at integration
// install time, catching mis-configured URLs / mTLS material / budget IDs
// before the bot ever serves a conversation.
//
// The roundtrip mirrors plugins/dify/spendguard/provider/spendguard.py
// (validate_provider_credentials).

import { toRuntimeError } from "../adapter/errors.js";
import type { Configuration } from "../config.js";
import { type BotpressCallContext, SpendGuardReservation } from "../reservation.js";

export interface ValidateArgs {
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
export async function validateConfiguration(args: ValidateArgs): Promise<void> {
  const reservation = args.reservationOverride ?? new SpendGuardReservation(args.configuration);
  const ctx: BotpressCallContext = {
    botId: "validateConfiguration",
    conversationId: "validateConfiguration-probe",
    userId: "validateConfiguration-probe",
    model: args.configuration.upstreamProvider,
    messages: [{ role: "user", content: "probe" }],
    maxTokens: 1,
    runId: "validateConfiguration",
  };
  try {
    const handle = await reservation.reserve(ctx);
    // Release immediately — INV-4 says the probe MUST exercise the full
    // reserve + release roundtrip, not just the reserve.
    await reservation.releaseFailure(handle, new Error("validateConfiguration probe complete"));
  } catch (err) {
    throw toRuntimeError(err);
  }
}
