# `@spendguard/inngest-agent-kit` — Third-Party License Notices

This package (`@spendguard/inngest-agent-kit`) is itself licensed under
Apache License 2.0 — see the repository root `LICENSE` file
(<https://github.com/m24927605/agentic-spendguard/blob/main/LICENSE>) for the
full Apache-2.0 text.

This document satisfies D29 review-standards §13.5 by listing every
direct runtime + peer + dev dependency and its license. Transitive deps
that flow in via these direct edges follow their own license; this notice
is required only for the direct edges below.

---

## Runtime dependencies

The published tarball contains only the adapter's own compiled output
(`dist/index.js` + `dist/index.d.ts`). The adapter takes NO direct
runtime dependencies — every external symbol the adapter references at
runtime is resolved through the consumer's `node_modules` via peer
dependencies (next section).

---

## Peer dependencies (resolved in the consumer's tree)

These packages are declared under `"peerDependencies"` in `package.json` —
the consumer installs them. The adapter imports their public surfaces at
build time and re-exports a handful (`SpendGuardError`, `DecisionDenied`,
`DecisionStopped`, `DecisionSkipped`, `ApprovalRequired`,
`SidecarUnavailable`) for ergonomic pattern-matching.

### `@spendguard/sdk`

- License: **Apache License 2.0**
- Project: <https://github.com/m24927605/agentic-spendguard>
- Use: Substrate gRPC client (`SpendGuardClient.reserve` /
  `commitEstimated`), typed errors (`DecisionDenied`, `DecisionStopped`,
  `DecisionSkipped`, `ApprovalRequired`, `SidecarUnavailable`,
  `SpendGuardError`), UUID + idempotency-key derivation
  (`deriveUuidFromSignature`, `deriveIdempotencyKey`),
  `InMemoryIdempotencyCache`. The adapter does NOT pin a version — the
  consumer's lockfile wins; the floor is `^0.1.0`.
- Full Apache-2.0 text:
  <https://github.com/m24927605/agentic-spendguard/blob/main/LICENSE>

### `@inngest/agent-kit`

- License: **Apache License 2.0**
- Project: <https://github.com/inngest/agent-kit>
- Use: The adapter targets the public `step.ai` namespace shape
  (`infer(name, opts, ctx)` + `wrap(name, fn, ...args)`). The adapter
  never instantiates `@inngest/agent-kit` symbols itself — consumers
  build their Inngest function with `({ step })` destructured and pass
  `step.ai` to `wrapWithSpendGuard(step.ai, client, opts)`.
- Full Apache-2.0 text:
  <https://github.com/inngest/agent-kit/blob/main/LICENSE.md>

  > Copyright Inngest, Inc. and contributors. Licensed under the Apache
  > License, Version 2.0. See `LICENSE.md` in the upstream repository for
  > the full text.

### `inngest`

- License: **Apache License 2.0**
- Project: <https://github.com/inngest/inngest-js>
- Use: Provides the `Inngest` client + `createFunction` factory the
  consumer uses to register the agent function. The adapter does not
  import `inngest` at runtime — the consumer brings their own.
- Full Apache-2.0 text:
  <https://github.com/inngest/inngest-js/blob/main/LICENSE.md>

---

## Build/Dev-only dependencies

Dev-only dependencies (`tsup`, `vitest`, `typescript`, `@biomejs/biome`,
`@types/node`, `zod`) are NOT shipped to consumers — they live under
`"devDependencies"` and are stripped from the published tarball. They do
not require notice attribution because they do not appear in the
consumer's runtime tree.

---

## Apache-2.0 compliance summary

Per Apache-2.0 §4(c), this `LICENSE_NOTICES.md` file is the NOTICE
document for redistribution. Consumers that re-distribute
`@spendguard/inngest-agent-kit` in binary form (e.g. as part of a larger
Node application's deploy artifact) should preserve this file. Consumers
that depend on `@spendguard/inngest-agent-kit` via npm and ship it
as-installed are already compliant — npm's install tree preserves this
file under
`node_modules/@spendguard/inngest-agent-kit/LICENSE_NOTICES.md`.
