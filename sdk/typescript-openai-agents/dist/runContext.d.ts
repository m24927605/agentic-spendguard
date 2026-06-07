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
interface RunContext {
    readonly runId: string;
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
declare function runContext<T>(ctx: RunContext, fn: () => Promise<T>): Promise<T>;
/**
 * Read the active {@link RunContext}. Throws `Error` when called outside any
 * `runContext()` scope so the adapter's PRE hook fails loud rather than
 * silently fabricating a run id.
 *
 * @throws Error when no active context is in scope.
 */
declare function currentRunContext(): RunContext;

export { type RunContext, currentRunContext, runContext };
