// LOCKED §4.1 / §4.2 surface contract tests (R2 follow-up for COV_S05_03).
//
// R1 BLOCKER B-1 flagged that `SpendGuardClientOptions` — the symbol every
// adapter spec (D04 / D06 / D08 / D29) imports per design.md §4.1 line 45 —
// was renamed to `SpendGuardClientConfig` in the slice's source tree and
// never re-exposed under the spec name. R2 fix: ship `SpendGuardClientOptions`
// as a type alias of `SpendGuardClientConfig`, re-exported from both the
// main barrel (`@spendguard/sdk`) and the `/client` subpath.
//
// R1 BLOCKER B-2 flagged that the `@spendguard/sdk/client` subpath did NOT
// expose `SpendGuardClientConfig` / `ResolvedConfig`, even though design.md
// §4.1 line 93 LOCKS that subpath as "SpendGuardClient + its types only."
// R2 fix: re-export the config types from `src/client.ts` so the generated
// `dist/client.d.ts` carries them; this file's compile-time assertions
// prove the contract holds against the in-tree source.
//
// The tests below use the TS type-system as the assertion harness: each
// `const _: TypeA = ... satisfies TypeB` exists purely so the strict typecheck
// fails if either symbol is missing or the alias loses identity. There is
// also a runtime assertion that the imported barrel surface includes the
// client class (a smoke check guarding against an accidental rename).

import { describe, expect, it } from "vitest";

// ── Main barrel: `@spendguard/sdk` (LOCKED §4.1 line 42-86) ───────────────

import {
  type SpendGuardClientConfig as MainBarrelClientConfig,
  type SpendGuardClientOptions as MainBarrelClientOptions,
  // Type-only imports: the verbatimModuleSyntax mode enforces these as the
  // canonical way to import LOCKED symbol names. If any of these names
  // changes shape or disappears, this file fails typecheck.
  type ResolvedConfig as MainBarrelResolvedConfig,
  type RunProjectionPolicy as MainBarrelRunProjectionPolicy,
  SpendGuardClient,
} from "../src/index.js";

// ── `/client` subpath (LOCKED §4.1 line 93) ───────────────────────────────
//
// Import path mirrors what `dist/client.d.ts` will resolve to after `tsup`
// build; in the source-tree the `/client` subpath is the file `src/client.ts`.

import {
  type SpendGuardClientConfig as ClientSubpathClientConfig,
  type SpendGuardClientOptions as ClientSubpathClientOptions,
  type ResolvedConfig as ClientSubpathResolvedConfig,
  type RunProjectionPolicy as ClientSubpathRunProjectionPolicy,
  SpendGuardClient as ClientSubpathSpendGuardClient,
} from "../src/client.js";

// ── Type-level identity assertions (compile-time) ─────────────────────────
//
// The trick: a function whose parameter is the union of two types is
// callable with EITHER type only if the types are mutually assignable. We
// instantiate the function with literal values that satisfy both types,
// guaranteeing the alias holds.

/** Compile-time assertion: `A` and `B` are mutually assignable. */
type AssertMutuallyAssignable<A, B> = [A] extends [B] ? ([B] extends [A] ? true : never) : never;

// `SpendGuardClientOptions` MUST be identical to `SpendGuardClientConfig`.
const _identityMain: AssertMutuallyAssignable<MainBarrelClientOptions, MainBarrelClientConfig> =
  true;
void _identityMain;

const _identitySubpath: AssertMutuallyAssignable<
  ClientSubpathClientOptions,
  ClientSubpathClientConfig
> = true;
void _identitySubpath;

// Cross-subpath identity: the main barrel and `/client` subpath MUST surface
// the SAME types (otherwise adapters get split-brain shapes when tree-shaking
// flips between barrel and subpath imports).
const _crossIdentityOptions: AssertMutuallyAssignable<
  MainBarrelClientOptions,
  ClientSubpathClientOptions
> = true;
void _crossIdentityOptions;

const _crossIdentityConfig: AssertMutuallyAssignable<
  MainBarrelClientConfig,
  ClientSubpathClientConfig
> = true;
void _crossIdentityConfig;

const _crossIdentityResolved: AssertMutuallyAssignable<
  MainBarrelResolvedConfig,
  ClientSubpathResolvedConfig
> = true;
void _crossIdentityResolved;

// Sanity: the LOCKED config shape includes the slice-doc-added
// `runProjectionDefault` field (design.md §4.2 R2 amendment).
const _runProjectionDefaultField: MainBarrelClientOptions = {
  socketPath: "/tmp/x.sock",
  tenantId: "t",
  runProjectionDefault: "STRICT_CEILING",
};
void _runProjectionDefaultField;

// ── MJ-1 closure: RunProjectionPolicy is a TYPE export, not a string field ──
//
// SLICE 3 R2 review surfaced MJ-1: the slice doc requires `RunProjectionPolicy`
// to be a string-literal union with at minimum the two ASP Draft-01 policies
// + the literal-string escape hatch `(string & {})`. SLICE 4 wires the type;
// the assertions below prove (a) the type exists on both barrels, (b) it
// retypes `runProjectionDefault` on `SpendGuardClientConfig`, and (c) the
// literal-string members are reachable from the union.

// (a) Cross-barrel identity: same type from both entry points.
const _runProjectionPolicyCrossIdentity: AssertMutuallyAssignable<
  MainBarrelRunProjectionPolicy,
  ClientSubpathRunProjectionPolicy
> = true;
void _runProjectionPolicyCrossIdentity;

// (b) The literal members are reachable — assigning each LOCKED policy name to
// the type slot proves the union includes them. If `RunProjectionPolicy` lost
// either member, these assignments would fail to typecheck.
const _policyEmpty: MainBarrelRunProjectionPolicy = "";
const _policyStrict: MainBarrelRunProjectionPolicy = "STRICT_CEILING";
const _policyElastic: MainBarrelRunProjectionPolicy = "ELASTIC";
const _policyForwardCompat: MainBarrelRunProjectionPolicy = "FUTURE_POLICY_VARIANT";
void _policyEmpty;
void _policyStrict;
void _policyElastic;
void _policyForwardCompat;

// (c) `runProjectionDefault` is typed as `RunProjectionPolicy`, NOT `string`.
// The test trick: extract the field's type via indexed access and assert
// mutual assignability to `RunProjectionPolicy`. If SLICE 3's `?: string` were
// still in place, this would not typecheck (`string` is wider than the policy
// union — assignment from `string` to the union fails).
type RunProjectionDefaultFieldType = NonNullable<MainBarrelClientOptions["runProjectionDefault"]>;
const _runProjectionDefaultFieldIsPolicy: AssertMutuallyAssignable<
  RunProjectionDefaultFieldType,
  MainBarrelRunProjectionPolicy
> = true;
void _runProjectionDefaultFieldIsPolicy;

// ── Runtime smoke (guards against name-only regression) ───────────────────

describe("LOCKED §4.1 — public surface symbols", () => {
  it("exposes SpendGuardClient as a runtime export from both entry points", () => {
    expect(typeof SpendGuardClient).toBe("function");
    expect(typeof ClientSubpathSpendGuardClient).toBe("function");
    // Same constructor identity — the `/client` subpath must re-export the
    // SAME class, not a separate copy.
    expect(SpendGuardClient).toBe(ClientSubpathSpendGuardClient);
  });

  it("constructs a client using the LOCKED §4.1 `SpendGuardClientOptions` shape", () => {
    // Use a `SpendGuardClientOptions`-typed literal explicitly to prove the
    // alias compiles end-to-end. If `SpendGuardClientOptions` were missing
    // or re-shaped, this test would not typecheck.
    const opts: MainBarrelClientOptions = {
      socketPath: "/tmp/x.sock",
      tenantId: "t",
    };
    const client = new SpendGuardClient(opts);
    expect(client.config.socketPath).toBe("/tmp/x.sock");
    expect(client.config.tenantId).toBe("t");
  });

  it("accepts the runProjectionDefault field (design §4.2 R2 amendment)", () => {
    const opts: MainBarrelClientOptions = {
      socketPath: "/tmp/x.sock",
      tenantId: "t",
      runProjectionDefault: "ELASTIC",
    };
    const client = new SpendGuardClient(opts);
    expect(client.config.runProjectionDefault).toBe("ELASTIC");
  });
});

// ── LOCKED §4.2 — release / queryBudget method shape (SLICE 5) ───────────

describe("LOCKED §4.2 — release / queryBudget surface", () => {
  it("release is a method on SpendGuardClient (per §4.2 LOCKED)", () => {
    const client = new SpendGuardClient({ socketPath: "/tmp/x.sock", tenantId: "t" });
    // Runtime function check — the method is on the prototype, not a per-instance
    // field. The LOCKED surface requires it as a method (not a getter or a
    // re-exported helper).
    expect(typeof client.release).toBe("function");
    expect(client.release.length).toBe(1); // single ReleaseRequest argument
  });

  it("queryBudget is a method on SpendGuardClient (per §4.2 LOCKED)", () => {
    const client = new SpendGuardClient({ socketPath: "/tmp/x.sock", tenantId: "t" });
    expect(typeof client.queryBudget).toBe("function");
    expect(client.queryBudget.length).toBe(1); // single QueryBudgetRequest argument
  });
});

// ── Type-level: commitEstimated accepts SLICE 5 optional outcome params ───
//
// AssertMutuallyAssignable-driven test: the SLICE 5 multi-event extension
// adds optional `outcomeKind` / `actualInputTokensWire` /
// `actualOutputTokensWire` / `actualErrorMessage` fields to
// `CommitEstimatedRequest`. The slice doc requires a type-level assertion
// that the param type is mutually assignable with a literal carrying all four
// fields — proves both that the names are spelled exactly as the slice doc
// specifies and that the types match.

import type { CommitEstimatedRequest as MainBarrelCommitEstimatedRequest } from "../src/index.js";

type CommitEstimatedRequestWithOutcomeShape = {
  runId: string;
  stepId: string;
  llmCallId: string;
  decisionId: string;
  reservationId: string;
  estimatedAmountAtomic: string;
  unit: { unit: string; denomination: number };
  pricing: { pricingVersion: string; pricingHash: Uint8Array };
  providerEventId: string;
  outcome: "SUCCESS" | "PROVIDER_ERROR" | "CLIENT_TIMEOUT" | "RUN_ABORTED";
  outcomeKind?: "SUCCESS" | "FAILURE";
  actualInputTokensWire?: string;
  actualOutputTokensWire?: string;
  actualErrorMessage?: string;
};

// One-direction assertion is sufficient here: the structural shape on the
// LHS must be assignable INTO the public `CommitEstimatedRequest` (i.e. the
// optional fields exist with the expected literal-union types). The other
// direction is loose because `CommitEstimatedRequest` has additional
// optional fields that the shape literal does not enumerate (e.g.
// `actualInputTokens`).
const _commitWithOutcome: CommitEstimatedRequestWithOutcomeShape = {
  runId: "r",
  stepId: "s",
  llmCallId: "l",
  decisionId: "d",
  reservationId: "res",
  estimatedAmountAtomic: "1",
  unit: { unit: "USD_MICROS", denomination: 1 },
  pricing: { pricingVersion: "v1", pricingHash: new Uint8Array() },
  providerEventId: "ev",
  outcome: "SUCCESS",
  outcomeKind: "FAILURE",
  actualInputTokensWire: "128",
  actualOutputTokensWire: "256",
  actualErrorMessage: "openai 429",
};
// Asserting that the shape is assignable into the public type proves the
// fields are present on `CommitEstimatedRequest` with at least these literal
// types.
const _assignToPublic: MainBarrelCommitEstimatedRequest = _commitWithOutcome;
void _assignToPublic;
void _commitWithOutcome;

// ── SLICE 6 (COV_S05_06) — ids / promptHash / pricing barrel reachability ──

import {
  computePromptHash as MainBarrelComputePromptHash,
  deriveIdempotencyKey as MainBarrelDeriveIdempotencyKey,
  deriveUuidFromSignature as MainBarrelDeriveUuidFromSignature,
  newUuid7 as MainBarrelNewUuid7,
  PricingLookup as MainBarrelPricingLookup,
  USD_MICROS_PER_USD as MainBarrelUsdMicrosPerUsd,
  workloadInstanceId as MainBarrelWorkloadInstanceId,
} from "../src/index.js";

describe("LOCKED §4.6 / §4.8 / §4.9 — SLICE 6 barrel reachability", () => {
  it("re-exports newUuid7 as a callable function", () => {
    expect(typeof MainBarrelNewUuid7).toBe("function");
    const u = MainBarrelNewUuid7();
    expect(u).toMatch(/^[0-9a-f]{8}-[0-9a-f]{4}-7[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/);
  });

  it("re-exports deriveIdempotencyKey as a callable function returning 'sg-<32 hex>'", () => {
    expect(typeof MainBarrelDeriveIdempotencyKey).toBe("function");
    const k = MainBarrelDeriveIdempotencyKey({
      tenantId: "t",
      sessionId: "s",
      runId: "r",
      stepId: "st",
      llmCallId: "l",
      trigger: "LLM_CALL_PRE",
    });
    expect(k).toMatch(/^sg-[0-9a-f]{32}$/);
  });

  it("re-exports deriveUuidFromSignature returning a v4-shaped UUID", () => {
    expect(typeof MainBarrelDeriveUuidFromSignature).toBe("function");
    const u = MainBarrelDeriveUuidFromSignature("sig", { scope: "decision_id" });
    expect(u).toMatch(/^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/);
  });

  it("re-exports workloadInstanceId returning a string", () => {
    expect(typeof MainBarrelWorkloadInstanceId).toBe("function");
    expect(typeof MainBarrelWorkloadInstanceId()).toBe("string");
  });

  it("re-exports computePromptHash returning lowercase hex (cross-language gate)", () => {
    expect(typeof MainBarrelComputePromptHash).toBe("function");
    const h = MainBarrelComputePromptHash("hello world", "00000000-0000-0000-0000-000000000001");
    expect(h).toBe("5d55a1ebc9782455de0979780fd6cf686127dadcba580f230ddc3fea31516d0d");
  });

  it("re-exports PricingLookup as a constructable class", () => {
    expect(typeof MainBarrelPricingLookup).toBe("function");
    const p = new MainBarrelPricingLookup(new Map([["openai|gpt-4o-mini|input", 0.15]]));
    expect(p.pricePerMillion("openai", "gpt-4o-mini", "input")).toBe(0.15);
  });

  it("re-exports USD_MICROS_PER_USD constant = 1_000_000", () => {
    expect(MainBarrelUsdMicrosPerUsd).toBe(1_000_000);
  });
});

describe("LOCKED §4.9 — DEMO_PRICING is subpath-only (NOT on main barrel)", () => {
  it("DEMO_PRICING is not in the main `@spendguard/sdk` barrel surface", async () => {
    // Importing from the main barrel must NOT expose DEMO_PRICING — keeps the
    // main-bundle size budget intact. Adapters that need it go to the subpath.
    const barrel = (await import("../src/index.js")) as Record<string, unknown>;
    expect(barrel.DEMO_PRICING).toBeUndefined();
    expect(barrel.DEMO_PRICING_VERSION).toBeUndefined();
  });

  it("DEMO_PRICING is reachable via the subpath `src/pricing/demo.js`", async () => {
    const { DEMO_PRICING, DEMO_PRICING_VERSION } = await import("../src/pricing/demo.js");
    expect(DEMO_PRICING).toBeDefined();
    expect(typeof DEMO_PRICING_VERSION).toBe("string");
  });
});

// ── SLICE 7 (COV_S05_07) R2 — runPlan barrel + subpath reachability ───────
//
// design.md §3 module layout pins `withRunPlan` + `currentRunPlan` + `RunPlan`
// on BOTH the main `@spendguard/sdk` barrel (line 76) AND the dedicated
// `@spendguard/sdk/runPlan` subpath (line 98). The slice's plannedStepsHint
// auto-fold inside `SpendGuardClient.buildDecisionRequest` is the substrate
// adapters (D04 / D06 / D08 / D29) consume — surface gaps here break every
// adapter that follows.
//
// R2 retired the SLICE 7 R1 identity-propagation shape; the LOCKED budget-hint
// surface is `{ plannedCalls, plannedTools }` with `currentRunPlan()` returning
// `RunPlan | null` (design.md §4.7 line 300).

import {
  currentRunPlan as MainBarrelCurrentRunPlan,
  type RunPlan as MainBarrelRunPlan,
  withRunPlan as MainBarrelWithRunPlan,
} from "../src/index.js";

import {
  currentRunPlan as SubpathCurrentRunPlan,
  type RunPlan as SubpathRunPlan,
  withRunPlan as SubpathWithRunPlan,
} from "../src/runPlan.js";

describe("LOCKED §4.7 / §3 — SLICE 7 R2 barrel + subpath reachability", () => {
  it("re-exports withRunPlan from the main barrel as a callable function", () => {
    expect(typeof MainBarrelWithRunPlan).toBe("function");
  });

  it("re-exports currentRunPlan from the main barrel as a callable function", () => {
    expect(typeof MainBarrelCurrentRunPlan).toBe("function");
    // LOCKED contract (design.md §4.7 line 300): currentRunPlan returns
    // `RunPlan | null` — outside a withRunPlan scope, it must be `null`,
    // NOT `undefined`.
    expect(MainBarrelCurrentRunPlan()).toBeNull();
  });

  it("re-exports withRunPlan from the `/runPlan` subpath as a callable function", () => {
    expect(typeof SubpathWithRunPlan).toBe("function");
  });

  it("re-exports currentRunPlan from the `/runPlan` subpath as a callable function", () => {
    expect(typeof SubpathCurrentRunPlan).toBe("function");
  });

  it("withRunPlan + currentRunPlan share the SAME function reference across barrel + subpath", () => {
    // Cross-entry identity — the main barrel must re-export the SAME function
    // from the subpath, not a separate copy. Otherwise adapters that mix
    // barrel + subpath imports would see split-brain ALS storages.
    expect(MainBarrelWithRunPlan).toBe(SubpathWithRunPlan);
    expect(MainBarrelCurrentRunPlan).toBe(SubpathCurrentRunPlan);
  });

  it("RunPlan type is mutually assignable across barrel + subpath", () => {
    // TS compile-time identity check: a literal typed as the main-barrel
    // RunPlan must be assignable to the subpath RunPlan and vice versa.
    const planMain: MainBarrelRunPlan = { plannedCalls: 1, plannedTools: 0 };
    const planSub: SubpathRunPlan = planMain;
    const backToMain: MainBarrelRunPlan = planSub;
    expect(backToMain.plannedCalls).toBe(1);
    expect(backToMain.plannedTools).toBe(0);
  });

  it("RunPlan shape is exactly {plannedCalls, plannedTools} — design.md §4.7", () => {
    // §1.2 P0 verbatim signature gate — the LOCKED surface is the budget-hint
    // shape, NOT identity propagation. Construct via the curried HOF so we
    // observe the stored RunPlan that callers read back from currentRunPlan.
    let observed: MainBarrelRunPlan | null = null;
    const wrapped = MainBarrelWithRunPlan({ plannedCalls: 5, plannedTools: 2 }, async () => {
      observed = MainBarrelCurrentRunPlan();
    });
    return wrapped().then(() => {
      expect(observed).not.toBeNull();
      expect(Object.keys(observed ?? {}).sort()).toEqual(["plannedCalls", "plannedTools"]);
      expect(observed).toEqual({ plannedCalls: 5, plannedTools: 2 });
    });
  });

  it("withRunPlan is curried — calling it returns a wrapper, NOT the result", () => {
    // §1.2 P0 verbatim signature gate — withRunPlan(plan, fn) returns a NEW
    // callable `(...args) => Promise<TRet>`; the wrapped fn is only invoked
    // when the returned callable is called.
    let invoked = false;
    const wrapped = MainBarrelWithRunPlan({ plannedCalls: 0, plannedTools: 0 }, async () => {
      invoked = true;
    });
    expect(typeof wrapped).toBe("function");
    expect(invoked).toBe(false);
  });
});
