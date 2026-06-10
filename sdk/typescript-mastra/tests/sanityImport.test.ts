// COV_D38_01 — sanity import test (pre-TP-01 smoke, tests.md §4 slice row).
//
// Asserts the placeholder barrel imports cleanly under vitest, `VERSION` is
// exported and matches package.json, and the three error re-exports are
// reference-identical to the `@spendguard/sdk` substrate classes (direct
// re-export — `instanceof` holds across the boundary).
import { describe, expect, it } from "vitest";

import {
  DecisionDenied as SdkDecisionDenied,
  SidecarUnavailable as SdkSidecarUnavailable,
  SpendGuardError as SdkSpendGuardError,
} from "@spendguard/sdk";
import pkg from "../package.json" with { type: "json" };
import { DecisionDenied, SidecarUnavailable, SpendGuardError, VERSION } from "../src/index.js";

describe("COV_D38_01 sanity import", () => {
  it("exports VERSION matching package.json#version", () => {
    expect(VERSION).toBe("0.1.0");
    expect(VERSION).toBe(pkg.version);
  });

  it("re-exports the three substrate error classes reference-identically", () => {
    expect(DecisionDenied).toBe(SdkDecisionDenied);
    expect(SidecarUnavailable).toBe(SdkSidecarUnavailable);
    expect(SpendGuardError).toBe(SdkSpendGuardError);
  });

  it("barrel is the §5 shape — no default export, no extra symbols", async () => {
    // COV_D38_02 completed the COV_D38_01 placeholder subset to the full
    // design §5 verbatim barrel (adds `SpendGuardProcessor`; the option /
    // estimator exports are type-only). tests/lockedSurface.test.ts TP-01
    // is the authoritative §5 gate; this sanity check stays in sync.
    const barrel = await import("../src/index.js");
    expect(Object.keys(barrel).sort()).toEqual([
      "DecisionDenied",
      "SidecarUnavailable",
      "SpendGuardError",
      "SpendGuardProcessor",
      "VERSION",
    ]);
    expect("default" in barrel).toBe(false);
  });
});
