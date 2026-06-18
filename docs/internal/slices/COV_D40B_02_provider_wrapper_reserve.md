# COV_D40B_02 - OpenClaw provider wrapper reserve path

> **Deliverable:** D40b OpenClaw provider plugin adapter
> **Slice:** 2 of 6
> **Spec set:** [`docs/specs/coverage/D40b_openclaw_provider_plugin/`](../../specs/coverage/D40b_openclaw_provider_plugin/)
> **Precedence:** `design.md` is LOCKED and trumps this doc.

## Scope

Implement options validation, prompt flattening, ID derivation, claim projection, and pre-dispatch `client.reserve()` in the provider wrapper. DENY and sidecar outage must abort before upstream provider dispatch.

## LOCKED design quotes

From `design.md` §5:

> Reserve-path errors are fail-closed. No fail-open option or env var is allowed in the adapter.

From `design.md` §6:

> Default claim projection mirrors D38/D04 discipline: at least one token, `ceil(chars/4)`, USD micros default, and explicit `unitId` + `windowInstanceId` on every claim.

## Files touched

| File | Why |
|---|---|
| `src/options.ts` | Required tuple validation. |
| `src/identity.ts` | Substrate ID helper calls only. |
| `src/flatten.ts` | OpenClaw request text flattening. |
| `src/provider.ts` | Reserve and fail-closed wrapper control flow. |
| `src/index.ts` | Export the reserve request builder for package tests. |
| `src/openclaw-api.d.ts` | Pin the `wrapStreamFn` context and stream function shim used by `OB-V3`. |
| `src/errors.ts` | Keep typed placeholder wording slice-neutral after reserve path lands. |
| `vitest.config.ts` | Resolve `@spendguard/sdk` to the local built SDK during standalone package tests. |
| `tests/provider.test.ts` | Reserve shape tests. |
| `tests/failclosed.test.ts` | DENY/outage before upstream tests. |
| `tests/identity.test.ts` | SDK identity delegation tests. |
| `tests/hashReuse.test.ts` | Prompt flattening and no local hash smoke. |

## VERIFY-AT-IMPL pins

Pin `OB-V3`.

## Test/verification plan

- TP-D40B-02..06.
- A1 package tests for touched files.

## Anti-scope

- No success/failure commit logic.
- No streaming lifecycle.
- No demo.
