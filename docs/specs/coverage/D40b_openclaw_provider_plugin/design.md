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
