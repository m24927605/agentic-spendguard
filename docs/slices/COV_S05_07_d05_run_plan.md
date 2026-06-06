# COV_S05_07 — D05 TS SDK substrate: withRunPlan + currentRunPlan

> **Deliverable**: D05 TS SDK substrate
> **Slice**: 7 of 10 (S)
> **Spec set**: [`docs/specs/coverage/D05_ts_sdk_substrate/`](../specs/coverage/D05_ts_sdk_substrate/)

## Scope

Land the run-plan context propagation surface that adapter code uses to thread the run identity through nested async boundaries (LangChain TS callbacks, Vercel AI SDK tool calls, OpenAI Agents TS handoffs):
1. `withRunPlan(plan, fn)` — higher-order function setting AsyncLocalStorage scope for `fn`'s execution
2. `currentRunPlan()` — reader returning the in-scope RunPlan or undefined
3. Sync + async parity — both forms return the same RunPlan; the AsyncLocalStorage propagation works across `await` boundaries (per design §4.7)

Concretely:
- `sdk/typescript/src/runPlan.ts` — NEW:
  ```ts
  import { AsyncLocalStorage } from "node:async_hooks";

  export interface RunPlan {
    runId: string;
    parentRunId?: string;
    traceparent?: string;
    tracestate?: string;
    budgetGrantJti?: string;
  }

  const storage = new AsyncLocalStorage<RunPlan>();

  export function withRunPlan<TArgs extends unknown[], TRet>(
    plan: RunPlan,
    fn: (...args: TArgs) => TRet,
    ...args: TArgs
  ): TRet {
    return storage.run(plan, () => fn(...args));
  }

  export function currentRunPlan(): RunPlan | undefined {
    return storage.getStore();
  }
  ```
- `sdk/typescript/src/index.ts` — barrel re-export `withRunPlan`, `currentRunPlan`, `RunPlan` type
- `sdk/typescript/package.json` — add `./runPlan` subpath export (follows the SLICE 6 4-subpath pattern)
- `sdk/typescript/tsup.config.ts` — add runPlan entry
- `sdk/typescript/src/client.ts` — modify `buildDecisionRequest` (or equivalent helper): when `currentRunPlan()` returns a plan, fold its `runId`/`parentRunId`/`traceparent`/`tracestate`/`budgetGrantJti` into the wire request IF the caller didn't supply them explicitly (caller-wins precedence)
- `sdk/typescript/tests/runPlan.test.ts` — NEW (≥12 tests):
  - withRunPlan + currentRunPlan basic round-trip
  - currentRunPlan returns undefined outside withRunPlan scope
  - Nested withRunPlan: inner scope sees inner plan; on inner return, outer scope sees outer plan
  - Async parity: `await` inside the fn preserves the run plan
  - Sync + async parity: both styles see the same plan
  - withRunPlan does NOT mutate the passed-in plan
  - Concurrent withRunPlan calls don't bleed plans across promise chains
  - SpendGuardClient.reserve() auto-folds currentRunPlan() into the wire request when present
  - Caller-supplied runId on reserve() takes precedence over currentRunPlan()
- `sdk/typescript/tests/locked-surface.test.ts` — barrel reachability of withRunPlan / currentRunPlan / RunPlan

## Files touched

| File | Why |
|------|-----|
| `sdk/typescript/src/runPlan.ts` | NEW — AsyncLocalStorage scope |
| `sdk/typescript/src/index.ts` | Barrel re-export |
| `sdk/typescript/src/client.ts` | Auto-fold currentRunPlan into wire request |
| `sdk/typescript/package.json` | ./runPlan subpath export |
| `sdk/typescript/tsup.config.ts` | runPlan entry |
| `sdk/typescript/tests/runPlan.test.ts` | NEW — context propagation tests |
| `sdk/typescript/tests/locked-surface.test.ts` | Surface assertion |

## Test/verification plan

1. `pnpm run typecheck` clean
2. `pnpm run test` — 233 + ~14 new = ~247 passing
3. `pnpm run lint` clean
4. `pnpm run build` clean
5. Bundle: dist/index.js minified ≤ 120 KB; dist/runPlan.js ≤ 5 KB

## Anti-scope

- No OTel / retry / idempotency cache — SLICE 8
- No release dance / NPM publish — SLICE 10
- No `@withRunPlan(...)` decorator syntax — v0.2 minor when TS 5 decorators stabilize (per design §4.7)
- No new RPC bodies — handshake/reserve/release/queryBudget/commitEstimated already wired

## Backlinks

- Spec set: [`design.md`](../specs/coverage/D05_ts_sdk_substrate/design.md) §4.7 withRunPlan + currentRunPlan, §3 module layout, §8 slice 7 row
- SLICE 6: [`COV_S05_06_d05_ids_prompt_hash_pricing.md`](COV_S05_06_d05_ids_prompt_hash_pricing.md)

## R2 amendment (2026-06-07)

The original SLICE 7 R1 implementation faithfully followed this slice doc's
identity-propagation `RunPlan` shape (`runId` / `parentRunId` / `traceparent`
/ `tracestate` / `budgetGrantJti`). However, R1 reviewer caught that the
LOCKED [`design.md`](../specs/coverage/D05_ts_sdk_substrate/design.md) §4.7
+ [`implementation.md`](../specs/coverage/D05_ts_sdk_substrate/implementation.md) §9
+ [`review-standards.md`](../specs/coverage/D05_ts_sdk_substrate/review-standards.md) §8
specify a DIFFERENT shape: `{ plannedCalls, plannedTools }` — a budget-hint
surface (Signal 3) informing run-projection of total planned work.

The slice doc was authored in error. The R2 fix retires the identity
propagation and ships the LOCKED budget-hint design. The identity-propagation
pattern remains useful and should be re-proposed in a future slice as a
separate `RunContext` / `withRunContext()` substrate with its own spec
amendment — the two shapes can coexist later but cannot share the `RunPlan`
symbol.

R2 deliverable:

- `RunPlan` interface: `{ plannedCalls: number; plannedTools: number }`
- `withRunPlan` — CURRIED form `(plan, fn) => (...args) => Promise<TRet>`
- `currentRunPlan` — returns `RunPlan | null` (not `undefined`)
- Nesting: OUTER plan wins (inner is a no-op for storage)
- Validation: `TypeError` synchronously at HOF construction time on
  non-integer or negative `plannedCalls` / `plannedTools`
- `client.ts` auto-fold: `plannedStepsHint = plan.plannedCalls + plan.plannedTools`
  when an active plan is in scope; `0` otherwise
- NO identity-field auto-fold — callers thread `runId` / `parentRunId` /
  `traceparent` / `tracestate` / `budgetGrantJti` explicitly per the existing
  SLICE 4-5 `ReserveRequest` wire path.
