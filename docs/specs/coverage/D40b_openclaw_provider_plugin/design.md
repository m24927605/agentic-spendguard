# D40b - OpenClaw provider plugin adapter

**Status:** Spec - LOCKED 2026-06-12.
**Parent strategy:** [`framework-coverage-addendum-2026-06-10.md`](../../../strategy/framework-coverage-addendum-2026-06-10.md) §2.
**Pattern:** Pattern 1 - in-process provider wrapper, with D40a as the durable fallback.
**Owner sub-agent:** Backend Architect.
**Prerequisite:** D40a should ship first. D40b may start after D40a spec lock, but its demo cannot be the only OpenClaw coverage claim.

> **LOCKED design.md trumps slice docs.** OpenClaw plugin APIs are churn-prone; any implementation-time correction must be recorded as a dated append-only amendment in this file.

## 1. Problem

D40a covers OpenClaw traffic that can be routed through a custom OpenAI-compatible base URL. That is durable, but it gives SpendGuard only the gateway-level view. D40b targets OpenClaw's in-process provider plugin or capability-registration surface so SpendGuard can bind to OpenClaw session/run context, provider metadata, and local-first operator configuration before dispatch.

The risk is explicit: OpenClaw uses rolling versioning, not semver, and hook-style plugins may be legacy relative to capability registration. D40b therefore pins one OpenClaw version and ships a small adapter with strict tests and a documented compatibility envelope.

## 2. Goals

1. New package or plugin tree under `integrations/openclaw-provider-plugin/`.
2. Register a SpendGuard provider that wraps an existing OpenClaw provider.
3. Reserve before provider dispatch; throw/return OpenClaw-native denial on DENY or sidecar outage.
4. Commit on success, provider error, client timeout, and run abort.
5. Thread `unitId`, `windowInstanceId`, and pricing tuple from day 1.
6. Demo mode `openclaw_provider_plugin` proves ALLOW, DENY-before-provider, STREAM, and provider error settlement.

## 3. Non-goals

- No attempt to stabilize all OpenClaw historical plugin APIs.
- No sandboxing of arbitrary OpenClaw plugins; D40b documents the in-process trust boundary.
- No fork or patch of OpenClaw core.
- No multi-provider feature matrix on day 1. OpenAI-compatible chat path is first.
- No changes to frozen cross-language fixtures.

## 4. Public surface - LOCKED

The adapter exposes a factory with explicit SpendGuard configuration:

```ts
export interface OpenClawSpendGuardOptions {
  client: SpendGuardClient;
  tenantId: string;
  budgetId: string;
  windowInstanceId: string;
  unitId: string;
  pricing: PricingFreeze;
  route?: string;
  claimEstimator?: OpenClawClaimEstimator;
  runIdProvider?: (ctx: OpenClawProviderContext) => string;
}

export function createSpendGuardOpenClawProvider(
  upstream: OpenClawProvider,
  options: OpenClawSpendGuardOptions,
): OpenClawProvider;
```

Exact `OpenClawProvider` and `OpenClawProviderContext` imported types are pinned by `OB-V1`. If OpenClaw's API cannot support a wrapper with this shape, a dated amendment must replace the surface before implementation continues.

## 5. Lifecycle - LOCKED

```text
OpenClaw model call
  -> SpendGuard provider wrapper
     -> flatten prompt/messages
     -> derive runId + llmCallId + idempotencyKey using @spendguard/sdk
     -> client.reserve(trigger="LLM_CALL_PRE")
       -> DENY/outage: abort provider call, no upstream invocation
       -> ALLOW: call upstream provider
     -> success: client.commitEstimated(outcome="SUCCESS")
     -> provider error: client.commitEstimated(outcome="PROVIDER_ERROR"), rethrow
     -> abort/timeout: client.commitEstimated(outcome="RUN_ABORTED" or "CLIENT_TIMEOUT")
```

Reserve-path errors are fail-closed. No fail-open option or env var is allowed in the adapter.

## 6. Identity and claims - LOCKED

All ID derivation uses `@spendguard/sdk`. The adapter contains no local hash dependency.

```text
signature = "v1|openclaw|" + tenantId + "|" + runId + "|" + flattenedPrompt
llmCallId = deriveUuidFromSignature(signature, { scope: "openclaw_llm_call_id" })
idempotencyKey = deriveIdempotencyKey({
  tenantId,
  sessionId: runId,
  runId,
  stepId: "llm_call",
  llmCallId,
  trigger: "LLM_CALL_PRE"
})
```

Default claim projection mirrors D38/D04 discipline: at least one token, `ceil(chars/4)`, USD micros default, and explicit `unitId` + `windowInstanceId` on every claim. Custom `claimEstimator` may replace the claims but must return claims with non-empty unit/window fields.

## 7. Trust boundary - LOCKED

OpenClaw plugins run in-process. D40b does not make untrusted plugins safe. The README and docs page must state:

> The OpenClaw provider plugin runs in the OpenClaw process. It is an enforcement hook, not a sandbox boundary. Operators should install it only in trusted OpenClaw deployments. Use D40a base-URL routing when the plugin API changes or when plugin installation is not acceptable.

## 8. VERIFY-AT-IMPL marker register

| Marker | Question to pin during implementation | Owning slice |
|---|---|---|
| `OB-V1` | Exact provider plugin/capability-registration API and imported type names for the pinned OpenClaw version. | `COV_D40B_01_plugin_package_init` |
| `OB-V2` | Whether `before_model_resolve` is still supported or legacy-only, and whether capability registration is required. | `COV_D40B_01_plugin_package_init` |
| `OB-V3` | Exact wrapper point that guarantees execution before upstream provider dispatch. | `COV_D40B_02_provider_wrapper_reserve` |
| `OB-V4` | Streaming lifecycle hook that indicates final usage/termination. | `COV_D40B_03_commit_failure_streaming` |
| `OB-V5` | Provider error and client abort signal shape. | `COV_D40B_03_commit_failure_streaming` |
| `OB-V6` | Plugin install/config path for local-first demo. | `COV_D40B_05_openclaw_plugin_demo` |

## 9. Slice plan

| Slice | Title |
|---|---|
| `COV_D40B_01_plugin_package_init` | Package/plugin skeleton and OpenClaw API pins. |
| `COV_D40B_02_provider_wrapper_reserve` | Pre-dispatch reserve and fail-closed denial. |
| `COV_D40B_03_commit_failure_streaming` | Success, failure, abort, and stream settlement. |
| `COV_D40B_04_failclosed_tests` | Hash-reuse, surface, fail-closed, and regression tests. |
| `COV_D40B_05_openclaw_plugin_demo` | Real demo mode and SQL gate. |
| `COV_D40B_06_docs_publish` | README, docs page, CHANGELOG, publish/closeout. |

## 10. Definition of done

D40b is shipped when all six slices land on main, the live `openclaw_provider_plugin` demo passes, every `OB-V*` marker is pinned, and docs clearly distinguish D40a base-URL fallback from D40b plugin enforcement.

## 11. Append-only implementation amendments

### 2026-06-12 - `COV_D40B_01_plugin_package_init` OpenClaw API pins

Pinned OpenClaw package: `openclaw@2026.6.2`, commit
`d4819948f37d45fe8f1428401316eaae456cdf16`. This matches D40a's pinned
source. The Node runtime floor is `>=22.19.0`, per OpenClaw's pinned
`package.json` `engines.node`.

Primary-source evidence:

- OpenClaw pinned `package.json`: <https://github.com/openclaw/openclaw/blob/d4819948f37d45fe8f1428401316eaae456cdf16/package.json>
- Provider plugin guide: <https://github.com/openclaw/openclaw/blob/d4819948f37d45fe8f1428401316eaae456cdf16/docs/plugins/sdk-provider-plugins.md>
- Public provider entry facade: <https://github.com/openclaw/openclaw/blob/d4819948f37d45fe8f1428401316eaae456cdf16/packages/plugin-sdk/src/provider-entry.ts>
- Provider type exports and provider registration API: <https://github.com/openclaw/openclaw/blob/d4819948f37d45fe8f1428401316eaae456cdf16/src/plugins/types.ts>
- Provider stream hook exports: <https://github.com/openclaw/openclaw/blob/d4819948f37d45fe8f1428401316eaae456cdf16/src/plugin-sdk/provider-stream.ts>

`OB-V1` is pinned to the capability-registration/provider plugin surface, not the draft `generate(request, context)` / `stream(request, context)` object shape. The exact imported OpenClaw type names for this slice are:

- `ProviderPlugin` from `openclaw/plugin-sdk/provider-model-shared`, used as `OpenClawProvider`.
- `OpenClawPluginApi` from `openclaw/plugin-sdk/plugin-entry`, whose provider methods include `registerProvider(provider: ProviderPlugin)` and `registerModelCatalogProvider(provider: UnifiedModelCatalogProviderPlugin)`.
- `ProviderWrapStreamFnContext` from `openclaw/plugin-sdk/plugin-entry`, reserved for later `OB-V3`/`OB-V4` wrapper-point pinning.

The public SpendGuard factory name and options surface remain locked, but the concrete OpenClaw wrapper target is `ProviderPlugin` catalog/registration behavior. OpenClaw at the pinned commit does not expose a single provider-call context type equivalent to the draft `OpenClawProviderContext`; the slice 1 placeholder keeps that type structural until `OB-V3` pins the exact pre-dispatch context. The placeholder package must not claim `generate` or `stream` interception until `OB-V3`/`OB-V4` pin the exact pre-dispatch and streaming terminal hooks.

`OB-V2` is pinned as follows: `before_model_resolve` is not the provider-plugin registration surface at the pinned commit. OpenClaw provider plugins register through `definePluginEntry(...).register(api)` with `api.registerProvider(...)`; the provider guide describes `registerModelCatalogProvider` as the newer control-plane catalog surface. The provider hook list uses `catalog`, `resolveDynamicModel`, `prepareDynamicModel`, `createStreamFn`, and `wrapStreamFn` for provider/runtime behavior. Capability registration is therefore required for D40b.

### 2026-06-12 - `COV_D40B_02_provider_wrapper_reserve` pre-dispatch wrapper point

`OB-V3` is pinned to OpenClaw's provider runtime hook `wrapStreamFn`.
The pinned provider guide states that OpenClaw calls provider hooks in order
and lists `createStreamFn` immediately before `wrapStreamFn`; the guide's
runtime-hook example returns a wrapper that receives `ctx.streamFn`, mutates
request headers, and then calls the inner stream function. The pinned type
surface has `wrapStreamFn?: (...) => StreamFn | null | undefined` and
`ProviderWrapStreamFnContext` extends the provider prepare-extra-params context
with `model?` and `streamFn?`; D40b must not claim session/run/turn fields on
that context until a later OpenClaw source pin proves them. D40b therefore uses
`wrapStreamFn(ctx)` as the pre-dispatch enforcement point: the SpendGuard
wrapper builds `async (params) => { await client.reserve(...); return
inner(params); }`, where `inner` is the upstream provider's `wrapStreamFn(ctx)`
result or `ctx.streamFn`. A reserve denial or sidecar outage propagates before
`inner(params)` is invoked, satisfying design §5 without relying on legacy
`before_model_resolve` hooks.

Primary-source evidence is the same pinned OpenClaw source set recorded in the
2026-06-12 `COV_D40B_01_plugin_package_init` amendment, especially the provider
plugin guide hook-order table and `src/plugin-sdk/provider-stream.ts`.

### 2026-06-12 - `COV_D40B_03_commit_failure_streaming` settlement pins

`OB-V4` is pinned to the return value of the `StreamFn` produced by
`wrapStreamFn`. The pinned OpenClaw provider API does not expose a separate
SpendGuard-usable terminal callback for provider streams; the wrapper therefore
owns settlement at the JavaScript boundary it returns. Non-streaming values
settle SUCCESS after `inner(params)` resolves. Async iterable values settle
exactly once when the iterator completes; iterator throws settle failure and
rethrow; early iterator return settles RUN_ABORTED.

`OB-V5` is pinned to thrown values observed at that same wrapper boundary.
OpenClaw does not expose a dedicated provider-error discriminated union in the
pinned provider hook. D40b classifies `AbortError`, `ABORT_ERR`, or an already
aborted request `signal` as RUN_ABORTED; `TimeoutError`, `ETIMEDOUT`, or a
timeout/timed-out message as CLIENT_TIMEOUT; every other thrown value is
PROVIDER_ERROR. Provider errors are rethrown after settlement so OpenClaw's
native caller surface is preserved.

D40b uses the TS SDK's single-event `commitEstimated` path: set `outcome` and
the single-event usage/metadata fields only. It must not set `outcomeKind` for
this adapter, because `outcomeKind` asks the SDK to emit a companion terminal
event and violates this slice's exactly-one-terminal-settlement requirement.
