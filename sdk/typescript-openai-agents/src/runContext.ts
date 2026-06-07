// `runContext` / `currentRunContext` — Node `AsyncLocalStorage`-backed
// per-invocation context for the OpenAI Agents TS adapter.
//
// design.md §6 / §7 (locked decision #4): the storage SHALL be keyed on
// `Symbol.for("@spendguard/run-context/v1")` so D04 / D06 / D08 / D29 all
// observe the SAME `AsyncLocalStorage` instance when a process imports more
// than one adapter — a Mastra + LangChain + Agents composite needs to dedupe
// to one storage so a single `runId` flows across frameworks. The
// `Symbol.for(...)` global-registry key works across module boundaries (a
// distinct npm install of the same package would still resolve to the same
// Symbol because the registry is per-process).
//
// D05 v0.2 will subsume this module into `@spendguard/sdk/run-context`; until
// then every adapter ships an identical ~12-line copy (design.md §7).
//
// NAMING NOTE: the type name `RunContext` collides with `@openai/agents`'s
// own exported `RunContext` (a different concept — that's the per-run state
// the OpenAI runner threads through tools). Consumers who import both should
// alias one of them:
//
//   import type { RunContext as SpendGuardRunContext } from "@spendguard/openai-agents";
//   import { RunContext } from "@openai/agents";
//
// The spec (design.md §4) locked the name `RunContext` for our adapter; we
// honour that here. The collision is documented in README.md / the JSDoc.

import { AsyncLocalStorage } from "node:async_hooks";

/**
 * Per-call context written by {@link runContext} and read by
 * {@link currentRunContext} via Node `AsyncLocalStorage`.
 *
 * The `runId` is the only required field today. Future fields (parentRunId,
 * traceparent, …) land in v0.2 when D05 v0.2 hoists this into the substrate.
 *
 * NOTE: not to be confused with `@openai/agents`'s own `RunContext` — that
 * is the per-run state the OpenAI runner threads through tools. This is the
 * SpendGuard-side per-call context the adapter reads to mint the
 * `(runId, stepId, llmCallId)` triple the substrate idempotency cache keys on.
 */
export interface RunContext {
  readonly runId: string;
}

// Shared module-singleton key. D04/D06/D08/D29 all import this same Symbol;
// pnpm dedupes the *file* — but even when it does NOT dedupe (npm-pack
// landing a duplicate copy in transitive node_modules), `Symbol.for(...)` is
// keyed on the global Symbol registry, which is per-process. Two separate
// copies of THIS file STILL resolve to the same Symbol, hence the same
// AsyncLocalStorage slot. Locked in design.md §7 decision #4.
const STORAGE_KEY = Symbol.for("@spendguard/run-context/v1");
type GlobalSlot = { [STORAGE_KEY]?: AsyncLocalStorage<RunContext> };

function storage(): AsyncLocalStorage<RunContext> {
  const slot = globalThis as GlobalSlot;
  if (!slot[STORAGE_KEY]) {
    slot[STORAGE_KEY] = new AsyncLocalStorage<RunContext>();
  }
  return slot[STORAGE_KEY];
}

/**
 * Run `fn` with the given `ctx` in scope. Inside `fn` (and any async work it
 * launches, including `await`s, `Promise.all`, `setImmediate`,
 * `process.nextTick`), {@link currentRunContext} returns this `ctx`.
 *
 * Nested calls: the inner `ctx` wins inside the inner `fn`; once the inner
 * `fn` resolves, the outer `ctx` is restored — review-standards.md §7.3.
 *
 * @example
 * ```ts
 * import { runContext } from "@spendguard/openai-agents";
 * import { Agent, Runner } from "@openai/agents";
 * import { newUuid7 } from "@spendguard/sdk";
 *
 * const runId = newUuid7();
 * const result = await runContext({ runId }, () =>
 *   Runner.run(agent, "Say hello in three words."),
 * );
 * ```
 */
export async function runContext<T>(ctx: RunContext, fn: () => Promise<T>): Promise<T> {
  return storage().run(ctx, fn);
}

/**
 * Read the active {@link RunContext}. Throws `Error` when called outside any
 * `runContext()` scope so the adapter's PRE hook fails loud rather than
 * silently fabricating a run id.
 *
 * @throws Error when no active context is in scope.
 */
export function currentRunContext(): RunContext {
  const ctx = storage().getStore();
  if (!ctx) {
    throw new Error(
      "@spendguard/openai-agents called outside an active runContext().\n" +
        "Wrap your Runner.run call:\n\n" +
        "    await runContext({ runId }, () => Runner.run(agent, input))\n",
    );
  }
  return ctx;
}
