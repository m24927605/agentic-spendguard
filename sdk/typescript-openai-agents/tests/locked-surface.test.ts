// SLICE 1 + SLICE 2 — locked public-surface smoke test.
//
// review-standards.md §3 (Public-surface lock — P0) requires every symbol
// listed in design.md §4 to be exported from `src/index.ts` and every
// signature to match the spec verbatim. This file is the single source of
// truth for that lock — adding a public symbol means adding an assertion
// here.
//
// review-standards.md §3.5: no `default export`. Verified by importing the
// barrel as a namespace and asserting absence of `.default`.

import { describe, expect, it } from "vitest";

describe("@spendguard/openai-agents locked surface (SLICE 1 + 2)", () => {
  it("exports VERSION as a non-empty semver string", async () => {
    const mod = await import("../src/index.js");
    expect(typeof mod.VERSION).toBe("string");
    expect(mod.VERSION).toMatch(/^\d+\.\d+\.\d+/);
  });

  it("exports the SLICE 2 factory + class surface", async () => {
    const mod = await import("../src/index.js");
    expect(typeof mod.withSpendGuard).toBe("function");
    expect(typeof mod.SpendGuardAgentsModel).toBe("function");
    expect(typeof mod.runContext).toBe("function");
    expect(typeof mod.currentRunContext).toBe("function");
    expect(typeof mod.deriveAgentSignature).toBe("function");
    expect(typeof mod.extractUsage).toBe("function");
  });

  it("re-exports the substrate error classes", async () => {
    const mod = await import("../src/index.js");
    const sdk = await import("@spendguard/sdk");
    // Class identity preserved — review-standards §10.1 / §10.5 ("no
    // wrapping, no inventing").
    expect(mod.DecisionDenied).toBe(sdk.DecisionDenied);
    expect(mod.DecisionStopped).toBe(sdk.DecisionStopped);
    expect(mod.ApprovalRequired).toBe(sdk.ApprovalRequired);
    expect(mod.SidecarUnavailable).toBe(sdk.SidecarUnavailable);
    expect(mod.SpendGuardError).toBe(sdk.SpendGuardError);
  });

  it("does NOT export a default", async () => {
    const mod = (await import("../src/index.js")) as Record<string, unknown>;
    expect(mod.default).toBeUndefined();
  });

  it("`./run-context` subpath shares the same runContext / currentRunContext", async () => {
    const root = await import("../src/index.js");
    const subpath = await import("../src/runContext.js");
    // Function-reference equality — the subpath re-exports the SAME
    // function objects so AsyncLocalStorage threads through one and only
    // one slot at runtime. Design §6 / §7 decision #4.
    expect(subpath.runContext).toBe(root.runContext);
    expect(subpath.currentRunContext).toBe(root.currentRunContext);
  });
});
