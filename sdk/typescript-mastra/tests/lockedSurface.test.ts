// COV_D38_02 — LOCKED §5 surface tests (tests.md TP-01..TP-06, gate A3.2).
//
// The §5 surface is a verbatim contract: exact export set, no default
// export, constructor validation, no fail-open option key, error-class
// reference identity, stable processor name.

import type { Processor } from "@mastra/core/processors";
import {
  DecisionDenied as SdkDecisionDenied,
  SidecarUnavailable as SdkSidecarUnavailable,
  SpendGuardError as SdkSpendGuardError,
} from "@spendguard/sdk";
import { describe, expect, it } from "vitest";
import {
  DecisionDenied,
  SidecarUnavailable,
  SpendGuardError,
  SpendGuardProcessor,
  VERSION,
} from "../src/index.js";
import type { SpendGuardProcessorOptions } from "../src/index.js";
// COV_D38_04: runtime-import the type-only options module so the v8
// coverage report scores it (it compiles to an empty module; without this
// import it shows as 0 % and noisily drags the package floor).
import "../src/options.js";
import { MockSpendGuardClient } from "./_support/mockSidecar.js";

// ── Type-level assertions (compile-time gates; vitest just hosts them) ────

type Expect<T extends true> = T;
type NotAKey<K extends string, T> = K extends keyof T ? false : true;

// TP-04 — options type has NO fail-open key (design §5 surface rule, P0).
type _tp04a = Expect<NotAKey<"failOpen", SpendGuardProcessorOptions>>;
type _tp04b = Expect<NotAKey<"degradeOnUnavailable", SpendGuardProcessorOptions>>;
type _tp04c = Expect<NotAKey<"enforcementMode", SpendGuardProcessorOptions>>;

describe("COV_D38_02 locked surface (TP-01..TP-06)", () => {
  it("TP-01: barrel exports exactly the §5 value symbols, no default export", async () => {
    const barrel = await import("../src/index.js");
    expect(Object.keys(barrel).sort()).toEqual([
      "DecisionDenied",
      "SidecarUnavailable",
      "SpendGuardError",
      "SpendGuardProcessor",
      "VERSION",
    ]);
    expect("default" in barrel).toBe(false);
    // Type-only exports (SpendGuardProcessorOptions / ClaimEstimator /
    // ClaimEstimatorInput) are proven by tests/_support/sampleConsumer.ts
    // importing them under `pnpm run typecheck` (A4.3).
    expect(VERSION).toBe("0.1.1");
  });

  it("TP-02: SpendGuardProcessor satisfies the installed Processor type (V1 gate)", () => {
    const mock = new MockSpendGuardClient();
    const guard = new SpendGuardProcessor({ client: mock.client, tenantId: "tenant-tp02" });
    // Compile-time gate: assignment fails to typecheck if the class drifts
    // from the installed @mastra/core Processor interface.
    const asProcessor: Processor = guard;
    expect(asProcessor).toBe(guard);
    expect(typeof guard.processInputStep).toBe("function");
    expect(typeof guard.processLLMRequest).toBe("function");
    // Installed Processor REQUIRES `id` (V1 pin — see src/processor.ts).
    expect(guard.id).toBe("spendguard-processor");
  });

  it("TP-03: missing client / empty tenantId → TypeError at construction", () => {
    const mock = new MockSpendGuardClient();
    expect(() => new SpendGuardProcessor(undefined as never)).toThrow(TypeError);
    expect(() => new SpendGuardProcessor({} as never)).toThrow(TypeError);
    expect(() => new SpendGuardProcessor({ tenantId: "tenant-x" } as never)).toThrow(TypeError);
    expect(() => new SpendGuardProcessor({ client: mock.client, tenantId: "" })).toThrow(TypeError);
    expect(() => new SpendGuardProcessor({ client: mock.client, tenantId: 42 as never })).toThrow(
      TypeError,
    );
    // Valid construction does NOT throw.
    expect(
      () => new SpendGuardProcessor({ client: mock.client, tenantId: "tenant-ok" }),
    ).not.toThrow();
  });

  it("TP-04: no fail-open option key (runtime companion to the type gate)", () => {
    // The compile-time Expect<> aliases above are the real gate. This runtime
    // companion is backed by the type, not a hardcoded list: the object below
    // is `Required<SpendGuardProcessorOptions>`, so if a fail-open key were
    // ever ADDED to the options type, typecheck would force it to appear here
    // and the runtime assertion would fail.
    const mock = new MockSpendGuardClient();
    const fullOptions: Required<SpendGuardProcessorOptions> = {
      client: mock.client,
      tenantId: "tenant-tp04",
      budgetId: "budget-tp04",
      unitId: "unit-tp04",
      route: "mastra-llm",
      defaultBudgetMicrosCap: 1_000_000n,
      claimEstimator: () => [],
      runIdProvider: () => "run-tp04",
      // §6.7 amendment #3 (2026-06-11): `pricing` is part of the §5 surface.
      pricing: { pricingVersion: "v-tp04", pricingHash: new Uint8Array([1]) },
    };
    for (const forbidden of ["failOpen", "degradeOnUnavailable", "enforcementMode"]) {
      expect(Object.keys(fullOptions)).not.toContain(forbidden);
    }
  });

  it("TP-05: re-exported error classes are reference-identical to @spendguard/sdk's", () => {
    expect(DecisionDenied).toBe(SdkDecisionDenied);
    expect(SidecarUnavailable).toBe(SdkSidecarUnavailable);
    expect(SpendGuardError).toBe(SdkSpendGuardError);
  });

  it('TP-06: readonly name === "spendguard-processor"', () => {
    const mock = new MockSpendGuardClient();
    const guard = new SpendGuardProcessor({ client: mock.client, tenantId: "tenant-tp06" });
    expect(guard.name).toBe("spendguard-processor");
  });
});
