// `beforeAiGeneration` hook — reserves SpendGuard budget BEFORE Botpress
// dispatches the upstream model HTTP.
//
// review-standards.md §3.7 / §3.8 / B01 / B02 / B03 / B07:
//   - ALLOW returns `{ data }` and stashes `data._spendguardHandle` for
//     `afterAiGeneration` to read.
//   - DENY throws Botpress `RuntimeError` (`code="BUDGET_DENIED"`) — no
//     upstream HTTP. INV-1.
//   - DEGRADE throws Botpress `RuntimeError` (`code="BUDGET_DEGRADED"`)
//     unless `SPENDGUARD_BOTPRESS_FAIL_OPEN=1` is set.
//   - Strict ordering: the sidecar `/v1/decision` POST completes before
//     this hook returns; INV-2 is enforced by the Botpress hook contract
//     (the hook is `await`-ed by the runtime before dispatching upstream).
//
// review-standards.md §3 cross-cutting on re-entrancy (INV-10): a fresh
// `SpendGuardReservation` is instantiated per hook call. The reservation
// has no module-level mutable state, so two concurrent calls for the same
// conversation produce distinct handles (B05).

import { type BotpressHookInput, toBindingFromHookInput } from "../adapter/binding.js";
import { toRuntimeError } from "../adapter/errors.js";
import type { Configuration } from "../config.js";
import { type ReservationHandle, SpendGuardReservation } from "../reservation.js";

/** Mutable handle stash for the afterAiGeneration cross-hook handoff
 *  (review-standards.md §3.11 / §3.12). `data._spendguardHandle` is the
 *  only field we add to the Botpress hook payload object. */
export interface SpendGuardHandleStash {
  _spendguardHandle?: ReservationHandle;
}

export interface BeforeAiHookArgs {
  readonly input: BotpressHookInput;
  readonly configuration: Configuration;
  /** Optional reservation override — used by the unit tests to inject
   *  a `SpendGuardReservation` configured with a mock sidecar fetch. */
  readonly reservationOverride?: SpendGuardReservation;
}

/**
 * Execute the beforeAiGeneration logic with a typed signature. The
 * top-level Botpress hook wires this into the `new Integration({...hooks})`
 * registration in `src/index.ts`.
 *
 * @throws RuntimeError on DENY / DEGRADE / config error.
 */
export async function runBeforeAiGeneration(
  args: BeforeAiHookArgs,
): Promise<{ data: BotpressHookInput["data"] & SpendGuardHandleStash }> {
  // Construct lazily so a SpendGuardConfigError raised by the Zod-style
  // runtime check inside SpendGuardReservation's constructor also flows
  // through the toRuntimeError translator (B06).
  let reservation: SpendGuardReservation;
  try {
    reservation = args.reservationOverride ?? new SpendGuardReservation(args.configuration);
  } catch (err) {
    throw toRuntimeError(err);
  }
  const ctx = toBindingFromHookInput({
    input: args.input,
    configuration: args.configuration,
  });
  let handle: ReservationHandle;
  try {
    handle = await reservation.reserve(ctx);
  } catch (err) {
    throw toRuntimeError(err);
  }
  // Stash on the Botpress hook payload object so afterAiGeneration can
  // locate the handle. The Botpress runtime threads `data` through to the
  // matching afterAiGeneration hook verbatim (per @botpress/sdk@0.7 hook
  // contract).
  const stash = args.input.data as BotpressHookInput["data"] & SpendGuardHandleStash;
  // Use `Object.defineProperty` so the field is enumerable: false — it
  // shows up in tests via direct property access but does NOT serialise
  // into the conversation payload Botpress logs.
  Object.defineProperty(stash, "_spendguardHandle", {
    value: handle,
    writable: true,
    enumerable: false,
    configurable: true,
  });
  return { data: stash };
}
