// `afterAiGeneration` hook — commits real usage with the SpendGuard
// sidecar AFTER Botpress completes the upstream model call.
//
// review-standards.md §3.9 / §3.10 / §3.11 / §3.12 / A01-A08:
//   - Real usage path: `event.payload.usage = { inputTokens, outputTokens }`
//     → commit with `actual_amount_atomic = inputTokens + outputTokens`.
//     INV-5 primary.
//   - Missing usage path: estimator-snapshot fallback + WARN log. INV-5
//     secondary.
//   - Missing handle path: when `data._spendguardHandle` is undefined
//     (before-hook never ran for whatever reason), return without RPC —
//     no phantom commit. §3.11.
//   - Commit-failure path: release the reservation, then throw the
//     translated RuntimeError so Botpress records the failure.
//   - Cancellation path: `data._cancelled = true` flag → release with
//     CANCELLED classification (A05).
//   - Handle cleanup: successful commit removes `data._spendguardHandle`
//     so a stale handle does not leak across hooks. §3.12 / INV-10.

import type { BotpressHookInput } from "../adapter/binding.js";
import { toRuntimeError } from "../adapter/errors.js";
import { extractUsageFromBotpressEvent, pickProviderEventId } from "../adapter/usage.js";
import type { Configuration } from "../config.js";
import { type ReservationHandle, SpendGuardReservation } from "../reservation.js";
import type { SpendGuardHandleStash } from "./beforeAiGeneration.js";

export interface AfterAiHookArgs {
  readonly input: BotpressHookInput & {
    readonly data: BotpressHookInput["data"] &
      SpendGuardHandleStash & {
        readonly _cancelled?: boolean;
      };
  };
  readonly configuration: Configuration;
  /** Optional reservation override for unit tests. */
  readonly reservationOverride?: SpendGuardReservation;
}

export async function runAfterAiGeneration(
  args: AfterAiHookArgs,
): Promise<{ data: BotpressHookInput["data"] & SpendGuardHandleStash }> {
  const data = args.input.data;
  const handle = data._spendguardHandle;
  if (handle === undefined) {
    // No before-hook handle — nothing to commit. §3.11.
    return { data };
  }
  const reservation = args.reservationOverride ?? new SpendGuardReservation(args.configuration);

  const cancelled = data._cancelled === true;
  if (cancelled) {
    await reservation.releaseFailure(
      handle,
      Object.assign(new Error("Botpress conversation cancelled"), {
        name: "AbortError",
      }),
    );
    clearHandle(data, handle);
    return { data };
  }

  const realUsage = extractUsageFromBotpressEvent(
    data as Parameters<typeof extractUsageFromBotpressEvent>[0],
  );
  const providerEventId = pickProviderEventId(data as Parameters<typeof pickProviderEventId>[0]);
  try {
    await reservation.commitSuccess(handle, realUsage, providerEventId);
  } catch (commitErr) {
    // Commit failed — release the reservation so the TTL sweeper does
    // not double-charge, then re-throw a translated RuntimeError so
    // Botpress records the conversation as failed.
    try {
      await reservation.releaseFailure(handle, commitErr);
    } catch (releaseErr) {
      // Already swallowed inside releaseFailure, but defensive against
      // a future change.
      console.warn(
        `spendguard:botpress: release-after-commit-failure swallowed for handle=${handle.reservationId}: ${
          releaseErr instanceof Error ? releaseErr.message : String(releaseErr)
        }`,
      );
    }
    clearHandle(data, handle);
    throw toRuntimeError(commitErr);
  }
  clearHandle(data, handle);
  return { data };
}

function clearHandle(
  data: BotpressHookInput["data"] & SpendGuardHandleStash,
  handle: ReservationHandle,
): void {
  // Defensive cleanup — preserve the handle id only if the operator
  // explicitly opted out via env var. Production default is to scrub.
  if ((process.env.SPENDGUARD_BOTPRESS_KEEP_HANDLE ?? "").trim() === "1") {
    return;
  }
  void handle; // tag the parameter as used
  try {
    // Cast to a permissive shape so the assignment compiles under
    // `exactOptionalPropertyTypes: true`. Setting to undefined matches
    // the A08 assertion (`expect(...).toBeUndefined()`) while satisfying
    // biome's noDelete preference.
    (data as { _spendguardHandle?: ReservationHandle | undefined })._spendguardHandle = undefined;
  } catch {
    // Frozen object — ignore. The handle stays but is harmless past
    // the after-hook return point.
  }
}
