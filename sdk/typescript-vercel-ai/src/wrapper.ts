// `wrapGenerate` / `wrapStream` placeholders.
//
// SLICE 2/3 ships only the factory skeleton + `transformParams` reserve
// wiring. The generate-commit (SLICE 4) and stream-commit (SLICE 5) paths
// throw a clear NotImplemented signal so a consumer who calls into a
// SLICE-2/3 build under `generateText(...)` / `streamText(...)` gets a
// pointed error instead of silent skip.
//
// The errors here are NOT re-exported to the public barrel — they're an
// internal "you composed a half-shipped slice" signal. Consumers will only
// ever see them by running a SLICE 2/3-only build against the real
// `wrapLanguageModel`; once SLICE 4 lands these functions are replaced
// with the real `commitOnSuccess` / `rollbackOnFailure` paths.
//
// Why throw, not return a synthetic result?
//   - `wrapGenerate` MUST return `Awaited<ReturnType<LanguageModelV1['doGenerate']>>`
//     — any synthetic fill-in would fabricate provider data that the
//     consumer downstream would treat as authoritative.
//   - A SLICE 2/3 build that ships to a demo / staging env should fail
//     loudly the moment the integration tries to actually run, not after
//     a downstream consumer trusts a stub completion text.

/**
 * Error thrown by SLICE 2/3 stubs for `wrapGenerate` / `wrapStream`.
 *
 * NOT a public-surface symbol; consumers should never catch this — they
 * should upgrade to the SLICE 4+ build of `@spendguard/vercel-ai` where
 * the real commit paths land.
 */
export class SpendGuardMiddlewareNotImplemented extends Error {
  constructor(hook: "wrapGenerate" | "wrapStream") {
    super(
      `@spendguard/vercel-ai: ${hook} is not implemented in this SLICE 2/3 build. Upgrade to the SLICE 4 build (wrapGenerate) or SLICE 5 build (wrapStream) of @spendguard/vercel-ai. See docs/specs/coverage/D06_vercel_ai_sdk/design.md §7.`,
    );
    this.name = "SpendGuardMiddlewareNotImplemented";
  }
}

/**
 * SLICE 2/3 stub for `wrapGenerate`. Real implementation lands in SLICE 4.
 *
 * @internal
 */
export async function wrapGenerateStub(): Promise<never> {
  throw new SpendGuardMiddlewareNotImplemented("wrapGenerate");
}

/**
 * SLICE 2/3 stub for `wrapStream`. Real implementation lands in SLICE 5.
 *
 * @internal
 */
export async function wrapStreamStub(): Promise<never> {
  throw new SpendGuardMiddlewareNotImplemented("wrapStream");
}
