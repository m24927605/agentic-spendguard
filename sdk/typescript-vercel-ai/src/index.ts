// @spendguard/vercel-ai — Vercel AI SDK middleware for SpendGuard budget
// guardrails. Transitively covers Mastra Agents via `@spendguard/vercel-ai/mastra`
// (a function-reference alias added in SLICE 7).
//
// SLICE 1 shipped the package skeleton. SLICE 2 added the
// `createSpendGuardMiddleware` factory shape, the LOCKED options surface,
// and the WeakMap stash plumbing. SLICE 3 wires `transformParams` against
// the substrate's `reserve()` RPC. SLICE 4/5 will wire `wrapGenerate`
// (commit/rollback) and `wrapStream` (TransformStream-based
// commit-after-finish). SLICE 6 lands the `@ai-sdk/openai` + `@ai-sdk/anthropic`
// provider matrix. SLICE 7 adds the Mastra subpath alias + Mastra `Agent`
// integration tests. SLICE 8 ships the publish pipeline.
//
// Public surface (LOCKED per design.md §4 / review-standards.md §1):
//   - `createSpendGuardMiddleware(opts): LanguageModelV1Middleware`     — SLICE 2/3
//   - `SpendGuardMiddlewareOptions`                                     — SLICE 2/3
//   - `DecisionDenied` / `SidecarUnavailable` / `SpendGuardError`       — SLICE 2/3
//   - `VERSION`                                                         — SLICE 1
//
// No `default` export — review-standards.md §1.5.

export { createSpendGuardMiddleware } from "./middleware.js";
export type { SpendGuardMiddlewareOptions } from "./options.js";
export { DecisionDenied, SidecarUnavailable, SpendGuardError } from "./errors.js";
export { VERSION } from "./version.js";
