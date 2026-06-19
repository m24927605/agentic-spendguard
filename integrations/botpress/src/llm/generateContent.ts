// `generateContent` action implementation — the SpendGuard gate point.
//
// Flow (fail-closed):
//   1. RESERVE  — SpendGuardReservation.reserve() against the sidecar. DENY ->
//      RuntimeError(BUDGET_DENIED); DEGRADE / transport -> RuntimeError(
//      BUDGET_DEGRADED); config error -> RuntimeError(BUDGET_CONFIG). On any
//      of these the upstream provider is NEVER called (INV-1).
//   2. FORWARD  — call the configured upstream provider. On a provider error
//      we RELEASE the reservation (so the TTL sweeper does not double-charge)
//      and surface a RuntimeError.
//   3. COMMIT   — SpendGuardReservation.commitSuccess() with the provider's
//      real token usage. A commit failure releases the reservation and throws.
//
// This module is wired into `new bp.Integration({ actions: { generateContent }})`
// in src/index.ts. It is kept Botpress-runtime-agnostic (takes the parsed
// configuration + a minimal logger + an injectable forward + an optional
// reservation override) so the unit tier can exercise the full ordering
// without the Botpress server or a live provider socket.

import { toBindingFromActionInput } from "../adapter/binding.js";
import type { BotpressActionCtx } from "../adapter/binding.js";
import { resolveMaxTokens, resolveModel } from "../adapter/binding.js";
import { toRuntimeError } from "../adapter/errors.js";
import type { Configuration } from "../config.js";
import {
  type ForwardFn,
  ProviderForwardError,
  defaultForward,
  toForwardRequest,
  toGenerateContentOutput,
} from "../provider/forward.js";
import { type ReservationHandle, SpendGuardReservation } from "../reservation.js";
import type { GenerateContentInput, GenerateContentOutput } from "./schemas.js";

/** Minimal logger surface — satisfied by the Botpress `IntegrationLogger`
 *  (`forBot().info/warn/error`) and by `console` in tests. */
export interface MinimalLogger {
  warn(message: string): void;
}

export interface GenerateContentArgs {
  readonly input: GenerateContentInput;
  readonly configuration: Configuration;
  readonly ctx: BotpressActionCtx;
  readonly logger?: MinimalLogger;
  /** Injected upstream forward; defaults to `defaultForward`. */
  readonly forward?: ForwardFn;
  /** Injected reservation (test seam); defaults to a fresh instance built from
   *  `configuration`. */
  readonly reservationOverride?: SpendGuardReservation;
  /** Resolved USD cost reported to Botpress billing. Defaults to 0 when no
   *  pricing freeze is wired (the SpendGuard sidecar is the source of truth
   *  for the ledgered cost; Botpress billing is advisory). */
  readonly costResolver?: (usage: { inputTokens: number; outputTokens: number }) => number;
}

/**
 * Execute the SpendGuard-gated `generateContent` action.
 *
 * @throws RuntimeError on DENY / DEGRADE / config error / provider error /
 *         commit failure — always after releasing any held reservation.
 */
export async function runGenerateContent(
  args: GenerateContentArgs,
): Promise<GenerateContentOutput> {
  const { input, configuration, ctx } = args;
  const forward = args.forward ?? defaultForward;
  const cost = args.costResolver ?? (() => 0);

  // Build the reservation lazily so a SpendGuardConfigError raised by the
  // constructor's runtime validation also flows through toRuntimeError.
  let reservation: SpendGuardReservation;
  try {
    reservation = args.reservationOverride ?? new SpendGuardReservation(configuration);
  } catch (err) {
    throw toRuntimeError(err);
  }

  const callCtx = toBindingFromActionInput({ input, configuration, ctx });

  // 1. RESERVE — fail-closed. No upstream call if this throws.
  let handle: ReservationHandle;
  try {
    handle = await reservation.reserve(callCtx);
  } catch (err) {
    throw toRuntimeError(err);
  }

  // 2. FORWARD — release the reservation on a provider error.
  const resolvedModel = resolveModel(input, configuration);
  const resolvedMaxTokens = resolveMaxTokens(input);
  const forwardReq = toForwardRequest(input, configuration, resolvedModel, resolvedMaxTokens);
  let result: Awaited<ReturnType<ForwardFn>>;
  try {
    result = await forward(forwardReq);
  } catch (err) {
    await reservation.releaseFailure(handle, err);
    const providerErr =
      err instanceof ProviderForwardError
        ? err
        : new ProviderForwardError(
            `spendguard:botpress: upstream forward failed: ${
              err instanceof Error ? err.message : String(err)
            }`,
          );
    throw toRuntimeError(providerErr);
  }

  // 3. COMMIT — real usage. On commit failure, release + throw.
  const realUsage = { inputTokens: result.inputTokens, outputTokens: result.outputTokens };
  try {
    await reservation.commitSuccess(handle, realUsage, result.id);
  } catch (commitErr) {
    await reservation.releaseFailure(handle, commitErr);
    throw toRuntimeError(commitErr);
  }

  return toGenerateContentOutput(result, configuration.upstreamProvider, cost(realUsage));
}
