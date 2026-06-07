// @spendguard/vercel-ai — Vercel AI SDK middleware for SpendGuard budget
// guardrails. Transitively covers Mastra Agents via `@spendguard/vercel-ai/mastra`
// (a function-reference alias added in SLICE 7).
//
// SLICE 1 ships the package skeleton. SLICE 2 will add the
// `createSpendGuardMiddleware` factory (validation + WeakMap stash). SLICE 3-5
// will wire `transformParams` (reserve), `wrapGenerate` (commit/rollback), and
// `wrapStream` (TransformStream-based commit-after-finish). SLICE 6 lands the
// `@ai-sdk/openai` + `@ai-sdk/anthropic` provider matrix. SLICE 7 adds the
// Mastra subpath alias + Mastra `Agent` integration tests. SLICE 8 ships the
// publish pipeline.
//
// Public surface (LOCKED per design.md §4 / review-standards.md §1):
//   - `createSpendGuardMiddleware(opts): LanguageModelV2Middleware`  — SLICE 2
//   - `SpendGuardMiddlewareOptions`                                  — SLICE 2
//   - `wrapWithSpendGuard(model, opts)` shorthand                    — SLICE 2
//   - `DecisionDenied` / `SidecarUnavailable` / `SpendGuardError`
//                                                                    — SLICE 2
//   - `VERSION`                                                      — SLICE 1
//
// No `default` export — review-standards.md §1.5.

export { VERSION } from "./version.js";
