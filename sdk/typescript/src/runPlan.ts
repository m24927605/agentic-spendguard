// SpendGuard SDK — `withRunPlan` + `currentRunPlan` (Signal 3 substrate).
//
// Power-user opt-in: a caller decorates an agent body with the expected
// number of LLM calls and tool calls. `SpendGuardClient.reserve()` reads the
// active plan via `currentRunPlan()` and folds the sum into the wire
// `DecisionRequest.plannedStepsHint` field. The sidecar forwards the hint to
// `run_cost_projector`, which uses Signal 3 (explicit) to override Signal 1
// (history-induced) — see `docs/specs/run-cost-projector-spec-v1alpha1.md` §5.
//
// Without `withRunPlan`, `currentRunPlan()` returns `null` and the SDK ships
// `plannedStepsHint = 0` (proto3 default) — the projector falls back to the
// history-induced estimate.
//
// ── Spec lineage (LOCKED) ──────────────────────────────────────────────────
//
//   - `docs/specs/coverage/D05_ts_sdk_substrate/design.md` §4.7 lines 290-303
//     (RunPlan + withRunPlan + currentRunPlan signatures).
//   - `docs/specs/coverage/D05_ts_sdk_substrate/implementation.md` §9
//     lines 1103-1138 (skeleton + outer-wins nesting + TypeError validation).
//   - `docs/specs/coverage/D05_ts_sdk_substrate/review-standards.md` §8
//     (run-plan correctness gates §8.1-§8.5).
//   - `sdk/python/src/spendguard/run_plan.py` (Python reference; identical
//     semantics modulo language idiom).
//
// ── R2 retirement note ─────────────────────────────────────────────────────
//
// SLICE 7 R1 shipped an IDENTITY-propagation RunPlan shape (runId, parentRunId,
// traceparent, tracestate, budgetGrantJti) because the slice doc described
// that shape. R1 review caught that the LOCKED design.md §4.7 + implementation.md
// §9 specify the BUDGET-HINT shape ({plannedCalls, plannedTools}) — and the
// LOCKED design wins per review-standards §1.2 (verbatim signature gate). R2
// retires the identity shape and ships the LOCKED budget-hint surface. The
// identity-propagation pattern is useful in its own right and should be
// re-proposed as a separate `RunContext` / `withRunContext` substrate in a
// future slice with its own spec amendment.

import { AsyncLocalStorage } from "node:async_hooks";

/**
 * Caller-declared plan for one logical agent run.
 *
 * - `plannedCalls`  — expected number of LLM calls in the run. Non-negative integer.
 * - `plannedTools` — expected number of tool calls in the run. Non-negative integer.
 *
 * The SDK ships `plannedStepsHint = plannedCalls + plannedTools` on every
 * `DecisionRequest` issued inside the active scope (per spec §5.1: steps are
 * the disjoint union of LLM + tool calls). The sidecar-side projector
 * enforces an upper bound `[0, MAX_PLANNED_STEPS]`
 * (`services/run_cost_projector/src/server.rs::MAX_PLANNED_STEPS`); we don't
 * repeat the bound here so a future Rust-side bump doesn't require an SDK
 * release.
 */
export interface RunPlan {
  plannedCalls: number;
  plannedTools: number;
}

// `AsyncLocalStorage` is the Node stdlib equivalent of Python's
// `contextvars.ContextVar`. Bun + Deno ship compatible implementations
// (design.md §4.7 line 292). Zero runtime dependency.
const storage = new AsyncLocalStorage<RunPlan>();

/**
 * Read the in-scope `RunPlan` if one exists, otherwise `null`.
 *
 * Returns `null` (not `undefined`) per LOCKED design.md §4.7 line 300 — the
 * shape is `RunPlan | null` for parity with the Python `current_run_plan()`
 * reference (which returns `None`) and review-standards §8.3.
 *
 * Returned object is the SAME reference `withRunPlan` stored (defensively
 * copied at scope entry); adapters MUST treat the returned value as
 * immutable.
 */
export function currentRunPlan(): RunPlan | null {
  return storage.getStore() ?? null;
}

/**
 * Higher-order function that installs `plan` in async-local scope around `fn`.
 *
 * CURRIED form per design.md §4.7 lines 295-298 — calling `withRunPlan(plan, fn)`
 * returns a NEW callable; the wrapped `fn` is only invoked when the returned
 * callable is called. Mirrors the `@with_run_plan(...)` decorator pattern in
 * `sdk/python/src/spendguard/run_plan.py`.
 *
 * ## Validation (LOCKED §8.4)
 *
 * `plannedCalls` and `plannedTools` are validated at HOF construction time
 * (NOT at wrapped-call time): if either is missing, non-integer, or negative,
 * a `TypeError` is thrown synchronously from `withRunPlan` — surfacing the
 * misuse at decorator application rather than first call. `plannedTools` is
 * optional on input and defaults to `0`.
 *
 * ## Nesting (LOCKED §8.2 — OUTER WINS)
 *
 * When the returned callable is invoked inside an existing `withRunPlan`
 * scope, the inner call is a NO-OP for plan storage — the outer plan stays
 * active and `currentRunPlan()` continues to return it. The inner `fn` is
 * still invoked with its arguments; only the storage swap is skipped. This
 * matches `run_plan.py` lines 183-191 (outer wins) and protects the budget
 * envelope from being silently rewritten by a sub-agent helper.
 *
 * @param plan The run plan to install. Validated immediately.
 * @param fn   The (sync or async) function to run inside the plan scope.
 * @returns A NEW async function that, when called, runs `fn(...args)` with
 *          the plan in scope and returns `Promise<TRet>`.
 *
 * @throws TypeError if `plannedCalls` or `plannedTools` is not a non-negative
 *         integer.
 */
export function withRunPlan<TArgs extends unknown[], TRet>(
  plan: { plannedCalls: number; plannedTools?: number },
  fn: (...args: TArgs) => TRet | Promise<TRet>,
): (...args: TArgs) => Promise<TRet> {
  if (!Number.isInteger(plan.plannedCalls) || plan.plannedCalls < 0) {
    throw new TypeError(
      `withRunPlan: plannedCalls must be a non-negative integer, got ${String(plan.plannedCalls)}`,
    );
  }
  const tools = plan.plannedTools ?? 0;
  if (!Number.isInteger(tools) || tools < 0) {
    throw new TypeError(
      `withRunPlan: plannedTools must be a non-negative integer, got ${String(plan.plannedTools)}`,
    );
  }
  const fullPlan: RunPlan = Object.freeze({
    plannedCalls: plan.plannedCalls,
    plannedTools: tools,
  });
  return async (...args: TArgs): Promise<TRet> => {
    // Nested: outer wins. If a plan is already active, the inner withRunPlan
    // is a pass-through — fn runs with the OUTER plan still visible to
    // currentRunPlan(). Matches run_plan.py lines 183-191.
    const existing = storage.getStore();
    if (existing !== undefined) {
      return await fn(...args);
    }
    return await storage.run(fullPlan, () => fn(...args));
  };
}
