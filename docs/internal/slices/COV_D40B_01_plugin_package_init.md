# COV_D40B_01 - OpenClaw provider plugin package init

> **Deliverable:** D40b OpenClaw provider plugin adapter
> **Slice:** 1 of 6
> **Spec set:** [`docs/specs/coverage/D40b_openclaw_provider_plugin/`](../../specs/coverage/D40b_openclaw_provider_plugin/)
> **Precedence:** `design.md` is LOCKED and trumps this doc.

## Scope

Create the plugin/package skeleton and pin OpenClaw's actual provider plugin or capability-registration API for the selected version. No reserve/commit behavior lands in this slice.

## LOCKED design quotes

From `design.md` §4:

> The adapter exposes a factory with explicit SpendGuard configuration:
>
> `createSpendGuardOpenClawProvider(upstream: OpenClawProvider, options: OpenClawSpendGuardOptions): OpenClawProvider`

From `design.md` §8:

> `OB-V1` - Exact provider plugin/capability-registration API and imported type names for the pinned OpenClaw version.
>
> `OB-V2` - Whether `before_model_resolve` is still supported or legacy-only, and whether capability registration is required.

## Files touched

| File | Why |
|---|---|
| `integrations/openclaw-provider-plugin/package.json` | Package metadata and OpenClaw version pin. |
| `integrations/openclaw-provider-plugin/README.md` | Slice-scope package readme and trust-boundary warning. |
| `integrations/openclaw-provider-plugin/CHANGELOG.md` | Package changelog skeleton. |
| `integrations/openclaw-provider-plugin/LICENSE_NOTICES.md` | Peer/license notice skeleton. |
| `integrations/openclaw-provider-plugin/tsconfig.json` | TypeScript config. |
| `integrations/openclaw-provider-plugin/tsconfig.tests.json` | Test typecheck config. |
| `integrations/openclaw-provider-plugin/tsup.config.ts` | ESM build config. |
| `integrations/openclaw-provider-plugin/vitest.config.ts` | Unit test runner config. |
| `integrations/openclaw-provider-plugin/src/index.ts` | Placeholder public barrel. |
| `integrations/openclaw-provider-plugin/src/version.ts` | VERSION constant. |
| `integrations/openclaw-provider-plugin/src/{provider,options,errors,identity,flatten,usage}.ts` | Skeleton modules. |
| `integrations/openclaw-provider-plugin/src/openclaw-api.d.ts` | Pinned OpenClaw type shim for local skeleton gates. |
| `integrations/openclaw-provider-plugin/tests/` | Import/type skeleton tests. |

## VERIFY-AT-IMPL pins

Pin `OB-V1` and `OB-V2`.

## Test/verification plan

- TP-D40B-01 skeleton.
- Package typecheck/build skeleton.

## Anti-scope

- No reserve or commit path.
- No demo overlay.
- No docs publish.
