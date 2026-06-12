# COV_D40A_01 - OpenClaw base-URL recipe and smoke

> **Deliverable:** D40a OpenClaw base-URL recipe
> **Slice:** 1 of 2
> **Spec set:** [`docs/specs/coverage/D40a_openclaw_base_url_recipe/`](../specs/coverage/D40a_openclaw_base_url_recipe/)
> **Precedence:** `design.md` is LOCKED and trumps this doc.

## Scope

Ship the OpenClaw drop-in recipe, example config, `openclaw_base_url` demo mode, runner, and hard verify SQL. This slice proves OpenClaw traffic can be routed through the SpendGuard egress proxy without OpenClaw plugin code.

## LOCKED design quotes

From `design.md` §4:

> OpenClaw is configured to send OpenAI-compatible chat traffic to:
>
> `http://egress-proxy:9000/v1`
>
> or, outside compose:
>
> `http://localhost:9000/v1`

From `design.md` §5:

> The demo mode name is `openclaw_base_url`.
>
> The locked success line is:
>
> `[demo] openclaw_base_url ALL 3 steps PASS (ALLOW + DENY + STREAM)`

## Files touched

| File | Why |
|---|---|
| `docs/site-v2/src/content/docs/docs/drop-in/openclaw.mdx` | New recipe page. |
| `examples/openclaw-base-url/README.md` | Local operator recipe. |
| `examples/openclaw-base-url/openclaw.config.example.json` | Example config pinned by `OA-V1`. |
| `deploy/demo/openclaw_base_url/*` | Compose overlay, fixture config, runner. |
| `deploy/demo/verify_step_openclaw_base_url.sql` | Hard SQL gate. |
| `deploy/demo/Makefile` | `DEMO_MODE=openclaw_base_url` and verify target. |

## VERIFY-AT-IMPL pins

Pin `OA-V1` through `OA-V5` in this slice with the OpenClaw version/source used by the demo.

## Test/verification plan

- TP-D40A-01..09.
- TA-D40A-01..05.
- Run `make demo-down` before `make demo-up DEMO_MODE=openclaw_base_url`.

## Anti-scope

- No provider plugin or hook code.
- No SDK/proto/ledger changes.
- No frozen corpus edits.
- No README/CHANGELOG closeout; slice 2 owns it.
