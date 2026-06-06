# COV_S05_08 — D05 TS SDK substrate: OTel + retry + idempotency cache

> **Deliverable**: D05 TS SDK substrate
> **Slice**: 8 of 10 (M)
> **Spec set**: [`docs/specs/coverage/D05_ts_sdk_substrate/`](../specs/coverage/D05_ts_sdk_substrate/)

## Scope

Three small but high-value cross-cutting modules per design §6.4 + §6.5:
1. **OTel hook** — optional `@opentelemetry/api` Tracer field on SpendGuardClientConfig. When provided, the client wraps every RPC in `spendguard.<rpc>` span with attributes (per design §6.4). When NOT provided, zero OTel dep cost.
2. **Retry helper** — bounded retry for sidecar-side `UNAVAILABLE` / `DEADLINE_EXCEEDED` / `CANCELLED` cluster, mirroring Python's `_classify_rpc_error`. Max 2 attempts. Idempotency-key REQUIRED (otherwise no-op + pointed SidecarUnavailable error per design §6.5).
3. **Idempotency cache** — in-process LRU keyed by `idempotencyKey`. Returns the cached DecisionOutcome when the same idempotency key is reserved twice in a row (deterministic adapter behavior under retries).

Concretely:
- `sdk/typescript/src/otel.ts` — NEW:
  - Wrap-RPC-in-span helper. Reads `cfg.otelTracer?` (optional peer dep).
  - `peerDependenciesMeta.optional: true` on `@opentelemetry/api` — adapters NOT enabling OTel never pay the dep cost.
- `sdk/typescript/src/retry.ts` — NEW:
  - `_classify_rpc_error(error)` — returns `"transient"` for UNAVAILABLE/DEADLINE_EXCEEDED/CANCELLED; `"permanent"` otherwise (mirror Python)
  - `runWithRetry(rpcFn, opts)` — invokes rpcFn; on transient + idempotency-key-present, retries once (max 2 attempts total)
  - Without idempotency key: no retry, throw pointed `SidecarUnavailable(cause)` so adapters can route
- `sdk/typescript/src/cache.ts` — NEW:
  - `InMemoryIdempotencyCache` LRU with cap (default 1000 entries)
  - `get(key) → DecisionOutcome | undefined`
  - `set(key, outcome, ttl)` with 5-min TTL default
  - Disabled-mode → no-op cache
- `sdk/typescript/src/client.ts` — modify:
  - SpendGuardClientConfig adds `otelTracer?: import("@opentelemetry/api").Tracer` (deferred import per design §6.4)
  - SpendGuardClientConfig adds `idempotencyCache?: IdempotencyCache` (interface)
  - All 5 wired RPCs (handshake/reserve/commitEstimated/release/queryBudget) wrap in OTel span if tracer present
  - reserve() wraps in retry helper; if idempotency cache present + cached → return cached outcome
- `sdk/typescript/src/index.ts` — barrel re-export: `runWithRetry`, `InMemoryIdempotencyCache`, types
- `sdk/typescript/package.json` — peerDependenciesMeta optional for `@opentelemetry/api`; `./retry` + `./cache` subpaths
- ≥18 new tests:
  - otel.test.ts (6+): span name + attributes; tracer NOT present → no spans; tracer error doesn't break RPC
  - retry.test.ts (6+): transient classification; permanent NOT retried; idempotency key required; max 2 attempts; SidecarUnavailable thrown when no key
  - cache.test.ts (6+): cache hit; cache miss; LRU eviction; TTL expiry; disabled-mode no-op

## Files touched

| File | Why |
|------|-----|
| `sdk/typescript/src/otel.ts` | NEW — span wrapper |
| `sdk/typescript/src/retry.ts` | NEW — _classify_rpc_error + runWithRetry |
| `sdk/typescript/src/cache.ts` | NEW — InMemoryIdempotencyCache LRU |
| `sdk/typescript/src/client.ts` | OTel + retry + cache integration |
| `sdk/typescript/src/index.ts` | Barrel re-exports |
| `sdk/typescript/package.json` | peerDependenciesMeta + subpaths |
| `sdk/typescript/tsup.config.ts` | otel/retry/cache entries |
| `sdk/typescript/tests/otel.test.ts` | NEW |
| `sdk/typescript/tests/retry.test.ts` | NEW |
| `sdk/typescript/tests/cache.test.ts` | NEW |
| `sdk/typescript/tests/locked-surface.test.ts` | Surface assertions |

## Test/verification plan

1. `pnpm run typecheck` clean
2. `pnpm run test` — 263 + ~18 = ~281 passing
3. `pnpm run lint` clean
4. `pnpm run build` clean
5. Bundle: dist/index.js minified ≤ 120 KB
6. OTel dep stays peer-optional (verify `@opentelemetry/api` NOT in regular deps)

## Anti-scope

- No release dance / NPM publish — SLICE 10
- No proto bump for LLM_CALL_OUTCOME — cross-component slice
- No new RPC bodies — all wired in SLICE 4-5
- No identity-propagation RunContext (deferred per SLICE 7 R2 amendment)

## Backlinks

- Spec set: [`design.md`](../specs/coverage/D05_ts_sdk_substrate/design.md) §6.4 (OTel hook semantics), §6.5 (retry helper), §8 slice 8 row
- SLICE 7: [`COV_S05_07_d05_run_plan.md`](COV_S05_07_d05_run_plan.md)
