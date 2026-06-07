# `@spendguard/sdk` — Third-Party License Notices

This package (`@spendguard/sdk`) is itself licensed under Apache License 2.0
— see the repository root `LICENSE` file
(<https://github.com/m24927605/agentic-spendguard/blob/main/LICENSE>) for the
full Apache-2.0 text.

This document satisfies review-standards §11.6 by listing every direct
runtime dependency and its license. Transitive deps that flow in via these
direct deps follow their own license; this notice is required only for the
direct edges below.

---

## Runtime dependencies (shipped to consumers)

These are the packages declared under `"dependencies"` in `package.json`
that ship inside the consumer's `node_modules/@spendguard/sdk` tree and
that the published `dist/*.js` imports at runtime.

### `@grpc/grpc-js`

- License: **Apache License 2.0**
- Project: <https://github.com/grpc/grpc-node/tree/master/packages/grpc-js>
- Use: Node-native gRPC client transport for the UDS sidecar connection
  (`SpendGuardClient.handshake() / reserve() / commitEstimated() / release()`).
- Full Apache-2.0 text:
  <https://github.com/grpc/grpc-node/blob/master/LICENSE>

### `@noble/hashes`

- License: **MIT**
- Project: <https://github.com/paulmillr/noble-hashes>
- Use: BLAKE2b primitive backing `deriveUuidFromSignature()` (P0
  byte-equivalent with the Python SDK and Rust sidecar). The package is also
  used via Node's `node:crypto` HMAC-SHA256 for `computePromptHash()` — the
  `@noble/hashes` dependency is BLAKE2b-only.
- Full MIT text:
  <https://github.com/paulmillr/noble-hashes/blob/main/LICENSE>

  > Copyright (c) 2022 Paul Miller (https://paulmillr.com)
  >
  > Permission is hereby granted, free of charge, to any person obtaining
  > a copy of this software and associated documentation files (the
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

### `@protobuf-ts/runtime`

- License: **Apache License 2.0**
- Project: <https://github.com/timostamm/protobuf-ts>
- Use: Generated `src/_proto/**/*.ts` modules import this runtime for
  message encode/decode.

### `@protobuf-ts/runtime-rpc`

- License: **Apache License 2.0**
- Project: <https://github.com/timostamm/protobuf-ts>
- Use: gRPC service-client surface that `_proto/spendguard/sidecar_adapter/v1/adapter.client.ts`
  builds against.

### `@protobuf-ts/grpc-transport`

- License: **Apache License 2.0**
- Project: <https://github.com/timostamm/protobuf-ts>
- Use: Adapter from the generated `runtime-rpc` clients to `@grpc/grpc-js`
  channels (the UDS transport).

---

## Optional peer dependencies

### `@opentelemetry/api`

- License: **Apache License 2.0**
- Project: <https://github.com/open-telemetry/opentelemetry-js-api>
- Use: Optional. Consumers that supply an `otelTracer` config field get
  per-RPC spans (design.md §6.4). Not shipped inside `@spendguard/sdk`'s
  tree because it is declared under `"peerDependencies"`; consumers install
  the version they want.

---

## Build/Dev-only dependencies

Dev-only dependencies (`tsup`, `vitest`, `typescript`, `@biomejs/biome`,
`@protobuf-ts/plugin`, `tsx`, `@types/node`) are NOT shipped to consumers
— they live under `"devDependencies"` and are stripped from the published
tarball. They do not require notice attribution because they do not appear
in the consumer's runtime tree.

---

## Generated proto sources

`src/_proto/spendguard/**` is generated from `proto/spendguard/**` at
build time via the `@protobuf-ts/plugin` codegen. The `.proto` source files
in this repository are Apache-2.0 licensed under the repo root `LICENSE`.

The generated TypeScript files inherit the original proto's license
(Apache-2.0); they are not third-party code and do not need separate notice
beyond the repo root `LICENSE`.

---

## Apache-2.0 compliance summary

Per Apache-2.0 §4(c), this `LICENSE_NOTICES.md` file is the NOTICE document
for redistribution. Consumers that re-distribute `@spendguard/sdk` in
binary form (e.g. as part of a larger Node application's deploy artifact)
should preserve this file. Consumers that depend on `@spendguard/sdk` via
npm and ship it as-installed are already compliant — npm's install tree
preserves this file under `node_modules/@spendguard/sdk/LICENSE_NOTICES.md`.
