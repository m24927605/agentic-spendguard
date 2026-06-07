import { AsyncLocalStorage } from 'async_hooks';

// src/runContext.ts
var STORAGE_KEY = /* @__PURE__ */ Symbol.for("@spendguard/run-context/v1");
function storage() {
  const slot = globalThis;
  if (!slot[STORAGE_KEY]) {
    slot[STORAGE_KEY] = new AsyncLocalStorage();
  }
  return slot[STORAGE_KEY];
}
async function runContext(ctx, fn) {
  return storage().run(ctx, fn);
}
function currentRunContext() {
  const ctx = storage().getStore();
  if (!ctx) {
    throw new Error(
      "@spendguard/openai-agents called outside an active runContext().\nWrap your Runner.run call:\n\n    await runContext({ runId }, () => Runner.run(agent, input))\n"
    );
  }
  return ctx;
}

export { currentRunContext, runContext };
