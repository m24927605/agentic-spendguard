# `@spendguard/langchain` — Third-Party License Notices

This package (`@spendguard/langchain`) is itself licensed under Apache License
2.0 — see the repository root `LICENSE` file
(<https://github.com/m24927605/agentic-spendguard/blob/main/LICENSE>) for the
full Apache-2.0 text.

This document satisfies D04 review-standards §12.4 by listing every direct
runtime + peer + dev dependency and its license. Transitive deps that flow
in via these direct edges follow their own license; this notice is required
only for the direct edges below.

---

## Runtime dependencies

The published tarball contains only the adapter's own compiled output
(`dist/index.js` + `dist/index.d.ts`) — **no third-party runtime
dependencies are bundled**. Every external symbol the adapter references at
runtime is resolved through the consumer's `node_modules` via peer
dependencies (next section). This keeps the published package small and lets
the consumer's lockfile pin the exact versions of `@spendguard/sdk` and
`@langchain/core` their application uses.

---

## Peer dependencies (resolved in the consumer's tree)

These packages are declared under `"peerDependencies"` in `package.json` —
the consumer installs them. The adapter imports their public surfaces at
build time and re-exports a handful (`SpendGuardError`, `DecisionDenied`,
`SidecarUnavailable`) for ergonomic pattern-matching.

### `@spendguard/sdk`

- License: **Apache License 2.0**
- Project: <https://github.com/m24927605/agentic-spendguard>
- Use: Substrate gRPC client (`SpendGuardClient.reserve` /
  `commitEstimated`), typed errors (`DecisionDenied`, `SidecarUnavailable`,
  `SpendGuardError`), and idempotency-key derivation
  (`deriveIdempotencyKey`). The adapter does NOT pin a version — the
  consumer's lockfile wins; the floor is `^0.1.0`.
- Full Apache-2.0 text:
  <https://github.com/m24927605/agentic-spendguard/blob/main/LICENSE>

### `@langchain/core`

- License: **MIT**
- Project: <https://github.com/langchain-ai/langchainjs>
- Use: `BaseCallbackHandler` superclass plus the
  `Serialized` / `BaseMessage` / `LLMResult` / `MessageContent` types that
  the adapter implements / accepts.
- Full MIT text:
  <https://github.com/langchain-ai/langchainjs/blob/main/LICENSE>

  > The MIT License
  >
  > Copyright (c) Harrison Chase
  >
  > Permission is hereby granted, free of charge, to any person obtaining a
  > copy of this software and associated documentation files (the
  > "Software"), to deal in the Software without restriction, including
  > without limitation the rights to use, copy, modify, merge, publish,
  > distribute, sublicense, and/or sell copies of the Software, and to
  > permit persons to whom the Software is furnished to do so, subject to
  > the following conditions:
  >
  > The above copyright notice and this permission notice shall be included
  > in all copies or substantial portions of the Software.
  >
  > THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS
  > OR IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF
  > MERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT.
  > IN NO EVENT SHALL THE AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY
  > CLAIM, DAMAGES OR OTHER LIABILITY, WHETHER IN AN ACTION OF CONTRACT,
  > TORT OR OTHERWISE, ARISING FROM, OUT OF OR IN CONNECTION WITH THE
  > SOFTWARE OR THE USE OR OTHER DEALINGS IN THE SOFTWARE.

---

## Build/Dev-only dependencies

Dev-only dependencies (`tsup`, `vitest`, `typescript`, `@biomejs/biome`,
`@types/node`, `@langchain/openai`) are NOT shipped to consumers — they
live under `"devDependencies"` and are stripped from the published tarball.
They do not require notice attribution because they do not appear in the
consumer's runtime tree.

For completeness, the dev-only `@langchain/openai` package (used by the
adapter's vitest E2E suite that exercises a stubbed `ChatOpenAI` fetch) is
**MIT** under the same `langchain-ai/langchainjs` umbrella license as
`@langchain/core`. The demo runner at
[`examples/langchain-ts/`](https://github.com/m24927605/agentic-spendguard/tree/main/examples/langchain-ts)
also depends on `@langchain/openai`; the example is shipped under the repo
root Apache-2.0 license.

---

## Apache-2.0 compliance summary

Per Apache-2.0 §4(c), this `LICENSE_NOTICES.md` file is the NOTICE document
for redistribution. Consumers that re-distribute `@spendguard/langchain` in
binary form (e.g. as part of a larger Node application's deploy artifact)
should preserve this file. Consumers that depend on `@spendguard/langchain`
via npm and ship it as-installed are already compliant — npm's install tree
preserves this file under
`node_modules/@spendguard/langchain/LICENSE_NOTICES.md`.
