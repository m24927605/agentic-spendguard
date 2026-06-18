# COV_S05_02 — D05 TS SDK substrate: proto codegen

> **Deliverable**: D05 TS SDK substrate
> **Slice**: 2 of 10 (M)
> **Spec set**: [`docs/specs/coverage/D05_ts_sdk_substrate/`](../../specs/coverage/D05_ts_sdk_substrate/)

## Scope

Stand up the proto codegen pipeline so subsequent slices (S05_03+) can `import` typed gRPC stubs without running codegen by hand.

Concretely:
- `sdk/typescript/scripts/proto.ts` — TS Node script that:
  - Reads from `proto/spendguard/` (canonical proto source at repo root)
  - Runs `protobuf-ts` plugin (locked decision §9.6) to emit ESM-ready TS
  - Writes into `sdk/typescript/src/_proto/` (the locked output tree per design §4.1)
  - Is idempotent: re-running produces byte-identical output
  - Honours `--check` flag for CI determinism gate (fails non-zero if rerunning would change anything)
- `sdk/typescript/package.json` — add `proto:gen` and `proto:check` scripts; add `@protobuf-ts/plugin` + `@protobuf-ts/runtime` + `@protobuf-ts/runtime-rpc` + `@protobuf-ts/grpc-transport` to devDependencies and runtime dependencies as appropriate per spec §6.2.
- `sdk/typescript/src/_proto/` — generated tree (committed; codegen output is treated as source of record so consumers don't need a protoc toolchain).
- `Makefile` parity target: a new `make sdk-ts-proto` target that delegates to `pnpm --filter @spendguard/sdk run proto:gen`. Mirrors the existing Python codegen target pattern.
- CI determinism gate: a new GH workflow step (or existing CI extension) that runs `pnpm --filter @spendguard/sdk run proto:check` and fails the build if there's drift.

## Files touched

| File | Why |
|------|-----|
| `sdk/typescript/scripts/proto.ts` | Codegen script |
| `sdk/typescript/package.json` | Add proto scripts + deps |
| `sdk/typescript/src/_proto/**/*.ts` | Generated tree (committed) |
| `Makefile` | parity target |
| `.github/workflows/*.yml` (existing CI extension OR new sdk-ts-ci.yml) | Determinism gate |

## Test/verification plan

1. `cd sdk/typescript && pnpm install` succeeds (new deps install).
2. `pnpm run proto:gen` produces the `src/_proto/` tree.
3. `pnpm run proto:gen && pnpm run proto:gen` produces byte-identical output (idempotency).
4. `pnpm run proto:check` exits 0 against a freshly-generated tree.
5. `pnpm run typecheck` passes against generated tree (no `any`, no unresolved imports).
6. `pnpm run build` exits 0 (build skeleton from SLICE 1 still runs).
7. Generated tree size ≤ 250 KB unminified per spec §10 budget — verified by `du -sb sdk/typescript/src/_proto/`.

## Anti-scope

- No `SpendGuardClient` class — SLICE 03.
- No public surface exports beyond `_proto` subpath — that arrives with the client.
- No browser-targeting generation — Node only, ESM only.

## Backlinks

- Spec set: [`design.md`](../../specs/coverage/D05_ts_sdk_substrate/design.md) §4.1 subpath table, §6.2 codegen, §9.6 locked decision (protobuf-ts over ts-proto), §10 bundle budget
- Build plan: [`framework-coverage-build-plan-2026-06.md`](../../strategy/framework-coverage-build-plan-2026-06.md) §1.5
