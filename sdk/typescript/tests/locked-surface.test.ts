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
