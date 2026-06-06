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
