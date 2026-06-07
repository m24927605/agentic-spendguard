// `@spendguard/vercel-ai/mastra` — function-reference subpath alias for
// Mastra Agent consumers.
//
// Per D06 design.md §4 + review-standards.md §1.4: Mastra users replace
// their import with this subpath; the body re-exports the SAME
// `createSpendGuardMiddleware` function under the Mastra-convention name
// `createSpendGuardLanguageMiddleware`. Function-reference identity is
// preserved (NOT a wrapper / NOT a copy) — review-standards §1.6 enforces
// `createSpendGuardMiddleware === createSpendGuardLanguageMiddleware` via
// strict equality assertion in `tests/locked-surface.test.ts`.
//
// Why a subpath alias instead of a separate package?
//   - Mastra Agents call `generateText` / `streamText` from `ai` underneath
//     (design.md §2). The same `LanguageModelV1Middleware` covers both
//     ecosystems byte-for-byte; shipping a separate `@spendguard/mastra`
//     package would force two installs + two version-bump cadences for one
//     wire shape.
//   - The subpath alias keeps the public surface explicit at the
//     import site: a Mastra consumer sees `createSpendGuardLanguageMiddleware`
//     (Mastra-idiomatic), a Vercel AI SDK consumer sees
//     `createSpendGuardMiddleware` (AI SDK-idiomatic). Same factory,
//     different name at the surface.
//   - Function-reference equality is preserved so callers doing
//     `import { createSpendGuardMiddleware } from "@spendguard/vercel-ai"`
//     AND
//     `import { createSpendGuardLanguageMiddleware } from "@spendguard/vercel-ai/mastra"`
//     get the SAME function — no double-instantiation, no behavioural drift.
//
// What changes between the two subpath surfaces?
//   - NOTHING beyond the import path + the alias name. The options shape,
//     return type, and runtime behaviour are identical.
//
// See `docs/site-v2/src/content/docs/docs/integrations/vercel-ai.mdx` for
// the Mastra-side usage walkthrough.

export {
  createSpendGuardMiddleware as createSpendGuardLanguageMiddleware,
} from "./middleware.js";

// Re-export the supporting types verbatim so a Mastra-only import is
// self-sufficient — no second import from the root `@spendguard/vercel-ai`
// required.
export type { SpendGuardMiddlewareOptions } from "./options.js";
export { DecisionDenied, SidecarUnavailable, SpendGuardError } from "./errors.js";
export { VERSION } from "./version.js";
