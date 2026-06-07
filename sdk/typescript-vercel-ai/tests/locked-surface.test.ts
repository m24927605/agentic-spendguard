import { describe, expect, it } from "vitest";

describe("@spendguard/vercel-ai locked surface", () => {
  it("exports VERSION", async () => {
    const mod = await import("../src/index.js");
    expect(typeof mod.VERSION).toBe("string");
    expect(mod.VERSION).toMatch(/^\d+\.\d+\.\d+/);
  });

  // SLICE 7 — review-standards §1.4 + §1.6 LOCK: the `/mastra` subpath
  // alias MUST be a function-reference re-export (strict `===` identity)
  // of `createSpendGuardMiddleware` from the root barrel — NOT a wrapper
  // and NOT a copy. Drift here would silently double-instantiate the
  // factory at the import-site for Mastra consumers, breaking the
  // `LanguageModelV1Middleware`-shape contract guarantees that the rest
  // of the test suite asserts against the root factory.
  it("Mastra subpath alias === root factory (function-reference equality)", async () => {
    const root = await import("../src/index.js");
    const mastra = await import("../src/mastra.js");
    expect(mastra.createSpendGuardLanguageMiddleware).toBe(root.createSpendGuardMiddleware);
    expect(typeof mastra.createSpendGuardLanguageMiddleware).toBe("function");
  });

  it("Mastra subpath re-exports the same VERSION + error classes", async () => {
    const root = await import("../src/index.js");
    const mastra = await import("../src/mastra.js");
    expect(mastra.VERSION).toBe(root.VERSION);
    expect(mastra.DecisionDenied).toBe(root.DecisionDenied);
    expect(mastra.SidecarUnavailable).toBe(root.SidecarUnavailable);
    expect(mastra.SpendGuardError).toBe(root.SpendGuardError);
  });
});
