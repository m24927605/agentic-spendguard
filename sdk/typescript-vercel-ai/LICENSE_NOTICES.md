# `@spendguard/vercel-ai` — Third-Party License Notices

This package (`@spendguard/vercel-ai`) is itself licensed under Apache
License 2.0 — see the repository root `LICENSE` file
(<https://github.com/m24927605/agentic-spendguard/blob/main/LICENSE>) for
the full Apache-2.0 text.

This document satisfies D06 review-standards §12.4 by listing every
direct runtime + peer + dev dependency and its license. Transitive deps
that flow in via these direct edges follow their own license; this
notice is required only for the direct edges below.

---

## Runtime dependencies

The published tarball contains only the middleware's own compiled output
(`dist/index.js` + `dist/index.d.ts` + the `dist/mastra.js` Mastra
subpath alias + `dist/mastra.d.ts`) — **no third-party runtime
dependencies are bundled**. Every external symbol the middleware
references at runtime is resolved through the consumer's `node_modules`
via peer dependencies (next section). This keeps the published package
small and lets the consumer's lockfile pin the exact versions of
`@spendguard/sdk`, `ai`, and `zod` their application uses.

---

## Peer dependencies (resolved in the consumer's tree)

These packages are declared under `"peerDependencies"` in `package.json`
— the consumer installs them. The middleware imports their public
surfaces at build time and re-exports a handful (`SpendGuardError`,
`DecisionDenied`, `SidecarUnavailable`) for ergonomic pattern-matching.

### `@spendguard/sdk`

- License: **Apache License 2.0**
- Project: <https://github.com/m24927605/agentic-spendguard>
- Use: Substrate gRPC client (`SpendGuardClient.reserve` /
  `commitEstimated`), typed errors (`DecisionDenied`,
  `SidecarUnavailable`, `SpendGuardError`), idempotency-key derivation
  (`deriveIdempotencyKey`), and UUID-from-signature helper
  (`deriveUuidFromSignature`). The middleware does NOT pin a version —
  the consumer's lockfile wins; the floor is `^0.5.0`.
- Full Apache-2.0 text:
  <https://github.com/m24927605/agentic-spendguard/blob/main/LICENSE>

### `ai` (Vercel AI SDK)

- License: **Apache License 2.0**
- Project: <https://github.com/vercel/ai>
- npm: <https://www.npmjs.com/package/ai>
- Use: `LanguageModelV1Middleware` type contract that the middleware
  factory implements; the consumer-side `wrapLanguageModel({ model,
  middleware })` glue + `generateText` / `streamText` entry points
  exercise the middleware's hooks.
- The Vercel AI SDK is published under Apache-2.0 by Vercel, Inc. and
  contributors. The middleware integrates against the public
  `LanguageModelV1Middleware` type surface; it does not vendor any
  upstream source.
- Full Apache-2.0 text: <https://www.apache.org/licenses/LICENSE-2.0>
- Verified via `ai@^4.0.0` package.json on npm (Apache-2.0).

### `zod`

- License: **MIT**
- Project: <https://github.com/colinhacks/zod>
- npm: <https://www.npmjs.com/package/zod>
- Use: Declared as an optional peer dep because the Vercel AI SDK uses
  `zod` for its structured-output validators; the middleware itself does
  NOT import `zod` directly — it is listed as an optional peer so
  consumers who do not use structured outputs are not forced to install
  it.
- Full MIT text:
  <https://github.com/colinhacks/zod/blob/main/LICENSE>

  > MIT License
  >
  > Copyright (c) 2020 Colin McDonnell
  >
  > Permission is hereby granted, free of charge, to any person obtaining a copy
  > of this software and associated documentation files (the "Software"), to deal
  > in the Software without restriction, including without limitation the rights
  > to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
  > copies of the Software, and to permit persons to whom the Software is
  > furnished to do so, subject to the following conditions:
  >
  > The above copyright notice and this permission notice shall be included in all
  > copies or substantial portions of the Software.
  >
  > THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
  > IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
  > FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
  > AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
  > LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
  > OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
  > SOFTWARE.

---

## Build/Dev-only dependencies

Dev-only dependencies (`tsup`, `vitest`, `typescript`, `@biomejs/biome`,
`@types/node`) are NOT shipped to consumers — they live under
`"devDependencies"` and are stripped from the published tarball. They do
not require notice attribution because they do not appear in the
consumer's runtime tree.

For completeness, the dev-only build tools are licensed as follows
(verified via their npm `package.json#license` fields):

- `tsup` — MIT (<https://github.com/egoist/tsup>)
- `vitest` — MIT (<https://github.com/vitest-dev/vitest>)
- `typescript` — Apache-2.0 (<https://github.com/microsoft/TypeScript>)
- `@biomejs/biome` — MIT + Apache-2.0 dual-licensed (<https://github.com/biomejs/biome>)
- `@types/node` — MIT (DefinitelyTyped)

The demo runner at
[`examples/vercel-ai-mastra/`](https://github.com/m24927605/agentic-spendguard/tree/main/examples/vercel-ai-mastra)
depends on `ai` + `zod` at runtime; the example is shipped under the
repo root Apache-2.0 license.

---

## Mastra ecosystem coverage

The package exposes a `@spendguard/vercel-ai/mastra` subpath alias that
re-exports the same factory under the Mastra-idiomatic name
`createSpendGuardLanguageMiddleware`. **No `@mastra/core` import** is
made from the package — the alias is a pure function-reference re-export
of the root barrel's `createSpendGuardMiddleware`. Mastra consumers
install `@mastra/core` themselves; the middleware integrates against
Mastra's underlying `ai`-package `generateText` / `streamText` calls
without taking a Mastra dependency.

- `@mastra/core` is published under Apache-2.0 by Mastra Inc.
  (<https://github.com/mastra-ai/mastra>). The middleware does not
  bundle, vendor, or re-export any `@mastra/core` symbol — the Mastra
  alias is a NAME (`createSpendGuardLanguageMiddleware`), not a
  type-or-implementation dependency.

---

## Apache-2.0 compliance summary

Per Apache-2.0 §4(c), this `LICENSE_NOTICES.md` file is the NOTICE
document for redistribution. Consumers that re-distribute
`@spendguard/vercel-ai` in binary form (e.g. as part of a larger Node
application's deploy artifact) should preserve this file. Consumers that
depend on `@spendguard/vercel-ai` via npm and ship it as-installed are
already compliant — npm's install tree preserves this file under
`node_modules/@spendguard/vercel-ai/LICENSE_NOTICES.md`.
