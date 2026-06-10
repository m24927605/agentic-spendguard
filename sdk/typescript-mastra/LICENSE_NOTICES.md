# `@spendguard/mastra` — Third-Party License Notices

This package (`@spendguard/mastra`) is itself licensed under Apache License
2.0 — see the repository root `LICENSE` file
(<https://github.com/m24927605/agentic-spendguard/blob/main/LICENSE>) for the
full Apache-2.0 text.

This document satisfies D38 review-standards §12.4 by listing every direct
runtime + peer dependency and its license. Transitive deps that flow in via
these direct edges follow their own license; this notice is required only
for the direct edges below.

---

## Runtime dependencies

The published tarball contains only the adapter's own compiled output
(`dist/index.js` + `dist/index.d.ts`) — **no third-party runtime
dependencies are bundled**. Every external symbol the adapter references at
runtime is resolved through the consumer's `node_modules` via peer
dependencies (next section). This keeps the published package small and
lets the consumer's lockfile pin the exact versions of `@spendguard/sdk`
and `@mastra/core` their application uses.

---

## Peer dependencies (resolved in the consumer's tree)

These packages are declared under `"peerDependencies"` in `package.json` —
the consumer installs them.

### `@spendguard/sdk`

- License: **Apache License 2.0**
- Project: <https://github.com/m24927605/agentic-spendguard>
- Use: Substrate gRPC client (`SpendGuardClient.reserve` /
  `commitEstimated`), typed errors (`DecisionDenied`, `SidecarUnavailable`,
  `SpendGuardError` — re-exported by this adapter for ergonomic
  pattern-matching), idempotency-key / UUID derivation
  (`deriveIdempotencyKey`, `deriveUuidFromSignature`), and the
  `BudgetClaim` / `PricingFreeze` wire types.
- Full Apache-2.0 text:
  <https://github.com/m24927605/agentic-spendguard/blob/main/LICENSE>

### `@mastra/core`

- License: **Apache License 2.0**
- Project: <https://github.com/mastra-ai/mastra>
- Use: the `Processor` interface from `@mastra/core/processors` that
  `SpendGuardProcessor` implements, plus the `Agent` / hook-argument types
  consumed at build time. Peer range `>=1.0.0 <2`.
- **`ee/` exclusion note**: the `@mastra/core` package exposes
  enterprise-edition subpaths (`dist/auth/ee/`, `dist/agent-builder/ee/`)
  that may carry additional licensing terms from upstream. This adapter
  imports **none** of them — only the Apache-2.0 core surfaces
  (`@mastra/core/processors`, `@mastra/core/agent`) are referenced, and
  none of `@mastra/core` is bundled into the published artifact.
- Full Apache-2.0 text:
  <https://github.com/mastra-ai/mastra/blob/main/LICENSE>

---

## Build/Dev-only dependencies

Dev-only dependencies (`tsup`, `vitest`, `@vitest/coverage-v8`,
`typescript`, `@biomejs/biome`, `@types/node`, `zod`) are NOT shipped to
consumers — they live under `"devDependencies"` and are stripped from the
published tarball. They do not require notice attribution because they do
not appear in the consumer's runtime tree.

---

## Apache-2.0 compliance summary

Per Apache-2.0 §4(c), this `LICENSE_NOTICES.md` file is the NOTICE document
for redistribution. Consumers that re-distribute `@spendguard/mastra` in
binary form (e.g. as part of a larger Node application's deploy artifact)
should preserve this file. Consumers that depend on `@spendguard/mastra`
via npm and ship it as-installed are already compliant — npm's install tree
preserves this file under
`node_modules/@spendguard/mastra/LICENSE_NOTICES.md`.
