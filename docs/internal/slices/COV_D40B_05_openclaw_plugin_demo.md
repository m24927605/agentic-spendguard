# COV_D40B_05 - OpenClaw provider plugin demo

> **Deliverable:** D40b OpenClaw provider plugin adapter
> **Slice:** 5 of 6
> **Spec set:** [`docs/specs/coverage/D40b_openclaw_provider_plugin/`](../../specs/coverage/D40b_openclaw_provider_plugin/)
> **Precedence:** `design.md` is LOCKED and trumps this doc.

## Scope

Add live demo mode `openclaw_provider_plugin`, runner, compose overlay, and hard verify SQL. The demo proves ALLOW, DENY, STREAM, and PROVIDER_ERROR against local stubs.

## LOCKED design quotes

From `implementation.md` §5:

> Locked success line:
>
> `[demo] openclaw_provider_plugin ALL 4 steps PASS (ALLOW + DENY + STREAM + PROVIDER_ERROR)`

From `acceptance.md` §2:

> runner DENY assertion - counting-stub counter unchanged across DENY.

## Files touched

| File | Why |
|---|---|
| `deploy/demo/openclaw_provider_plugin/*` | Demo overlay and runner. |
| `deploy/demo/verify_step_openclaw_provider_plugin.sql` | Hard SQL gate. |
| `deploy/demo/Makefile` | Demo mode and verify target. |

## VERIFY-AT-IMPL pins

Pin `OB-V6`.

## Test/verification plan

- `make demo-down`
- `make demo-up DEMO_MODE=openclaw_provider_plugin`
- `make -C deploy/demo demo-verify-openclaw-provider-plugin`

## Anti-scope

- No package public surface changes unless demo exposes a blocker.
- No D40a demo changes.
