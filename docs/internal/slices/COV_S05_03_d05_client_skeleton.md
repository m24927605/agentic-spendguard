# COV_S05_03 — D05 TS SDK substrate: SpendGuardClient skeleton

> **Deliverable**: D05 TS SDK substrate
> **Slice**: 3 of 10 (M)
> **Spec set**: [`docs/specs/coverage/D05_ts_sdk_substrate/`](../../specs/coverage/D05_ts_sdk_substrate/)

## Scope

Stand up the `SpendGuardClient` class shell that subsequent SLICE 4-8 will fill in. This slice ships the lifecycle surface (`connect` / `close` / `asyncDispose`), the env-var resolution + config validation, the UDS connection via `@grpc/grpc-js`, and the typed-config builder. No `reserve()` / `commitEstimated()` / `release()` / `queryBudget()` business logic — those land in SLICE 4 + 5.

Concretely:
- `sdk/typescript/src/client.ts`:
  - `SpendGuardClient` class with public constructor accepting `SpendGuardClientConfig`
  - `connect(): Promise<void>` — opens UDS via `@grpc/grpc-js` ChannelCredentials.createInsecure + grpc-transport
  - `close(): Promise<void>` — gracefully closes channel
  - `[Symbol.asyncDispose](): Promise<void>` — ESM 2024 disposable
  - `reserve(...)`, `commitEstimated(...)`, `release(...)`, `queryBudget(...)` — all defined but throw `SpendGuardError("not yet wired — SLICE 4-5")`
  - `SpendGuardClient.fromEnv(): SpendGuardClient` — convenience factory reading `SPENDGUARD_SOCKET_PATH` / `SPENDGUARD_TENANT_ID` / `SPENDGUARD_RUN_PROJECTION_DEFAULT` env vars
- `sdk/typescript/src/config.ts`:
  - `SpendGuardClientConfig` interface (locked surface per design §4.2)
  - `validateConfig(cfg)` runtime validator (tenant_id is UUID, socket path exists, etc.)
- `sdk/typescript/src/errors.ts`:
  - `SpendGuardError` base class
  - `SpendGuardConfigError`, `SpendGuardConnectionError`, `SpendGuardDecisionError` discriminated subtypes
- `sdk/typescript/src/index.ts` — public barrel exporting `SpendGuardClient`, `SpendGuardClientConfig`, `SpendGuardError*`, `_proto` types
- `sdk/typescript/package.json` — populate the `exports` map: `./index`, `./proto` per design §4.1 locked subpaths

## Files touched

| File | Why |
|------|-----|
| `sdk/typescript/src/client.ts` | SpendGuardClient shell + lifecycle |
| `sdk/typescript/src/config.ts` | Config interface + runtime validator |
| `sdk/typescript/src/errors.ts` | Typed error hierarchy |
| `sdk/typescript/src/index.ts` | Public barrel |
| `sdk/typescript/package.json` | Populate `exports` map |

## Test/verification plan

1. `pnpm run typecheck` clean.
2. `pnpm run build` produces `dist/index.js` and `dist/index.d.ts`; both ESM.
3. `pnpm run lint` clean.
4. `pnpm pack --dry-run` shows public exports match design §4.1 locked subpaths.
5. Unit test: `new SpendGuardClient({...})` constructs, `connect()` opens a mock UDS server and resolves, `close()` drains and resolves, no socket leak after `[Symbol.asyncDispose]`.
6. Unit test: `SpendGuardClient.fromEnv()` reads env vars correctly, throws SpendGuardConfigError on missing required vars.
7. Unit test: `reserve` / `commitEstimated` / `release` / `queryBudget` all throw `SpendGuardError` with "SLICE 4-5" message.

## Anti-scope

- No `reserve()` / `commitEstimated()` / `release()` business logic — SLICE 04.
- No `queryBudget()` real path — SLICE 05.
- No `ids.ts` / `promptHash.ts` / `pricing.ts` — SLICE 06.
- No `withRunPlan` — SLICE 07.
- No OTel / retry / idempotency cache — SLICE 08.
- No cross-language fixture tests — SLICE 09.
- No npm publish — SLICE 10.

## Backlinks

- Spec set: [`design.md`](../../specs/coverage/D05_ts_sdk_substrate/design.md) §4.1 (subpath), §4.2 (config), §8 slice plan
- Build plan: [`framework-coverage-build-plan-2026-06.md`](../../strategy/framework-coverage-build-plan-2026-06.md) §1.5
