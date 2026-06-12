# COV_D40A_01 - OpenClaw base-URL recipe and smoke

> **Deliverable:** D40a OpenClaw base-URL recipe
> **Slice:** 1 of 2
> **Spec set:** [`docs/specs/coverage/D40a_openclaw_base_url_recipe/`](../specs/coverage/D40a_openclaw_base_url_recipe/)
> **Precedence:** `design.md` is LOCKED and trumps this doc.

## Scope

Ship the OpenClaw drop-in recipe, example config, `openclaw_base_url` demo mode, runner, and hard verify SQL. This slice proves OpenClaw-shaped traffic for the pinned `openai-completions` provider config can be routed through the SpendGuard egress proxy without OpenClaw plugin code.

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
| `deploy/demo/init/bundles/generate.sh` | Default-preserving demo hard-cap threshold env for deterministic local DENY. |
| `deploy/demo/verify_step_openclaw_base_url.sql` | Hard SQL gate. |
| `deploy/demo/Makefile` | `DEMO_MODE=openclaw_base_url` and verify target. |
| `services/egress_proxy/src/forward.rs` | Demo/test upstream override for local counting-stub hard gate; default routing unchanged. |

## VERIFY-AT-IMPL pins

Pin `OA-V1` through `OA-V5` in this slice with the OpenClaw version/source used by the demo.

Pinned implementation notes:

- `OA-V1`: OpenClaw primary source `docs/concepts/model-providers.md` at
  `openclaw/openclaw@d4819948f37d45fe8f1428401316eaae456cdf16` documents
  `models.providers.<id>.baseUrl`, `apiKey`, `api: "openai-completions"`,
  `timeoutSeconds`, `models[].id`, and
  `agents.defaults.model.primary: "provider/model"`.
- `OA-V2`: OpenClaw primary source `docker-compose.yml` at the same commit
  documents `OPENCLAW_CONFIG_PATH`, `OPENCLAW_CONFIG_DIR`,
  `OPENCLAW_STATE_DIR`, and `OPENCLAW_WORKSPACE_DIR`; D40a uses a committed
  config fixture and does not drive the GUI.
- `OA-V3`: OpenClaw primary source `package.json` at the same commit pins npm
  package `openclaw` version `2026.6.2` and Node `>=22.19.0`; full gateway
  runtime embedding is deferred to D40b per design §11.
- `OA-V4`: OpenClaw primary source `docs/concepts/model-providers.md` at the
  same commit documents `api: "openai-completions"` for local proxy/custom
  provider traffic. D40a therefore pins the SpendGuard-bound request/response
  shape to OpenAI Chat Completions, including streaming SSE:
  `stream: true`, `Content-Type: text/event-stream`, `data:` frames, final
  usage-bearing frame, and `[DONE]`. The fixture runner validates this
  proxy-facing shape; full gateway-runtime observation remains D40b.
- `OA-V5`: OpenClaw primary source `docs/concepts/model-providers.md` at the
  same commit documents provider-level `apiKey` for the custom provider. D40a
  verifies the SpendGuard boundary by sending that configured key as inbound
  `Authorization: Bearer <apiKey>` to the egress proxy; the local counting stub
  ignores auth. D40a does not assert additional gateway-side auth stripping or
  rewriting because the full OpenClaw gateway runtime is D40b scope.

## Test/verification plan

- TP-D40A-01..11.
- TA-D40A-01..05.
- Run `make demo-down` before `make demo-up DEMO_MODE=openclaw_base_url`.

## Anti-scope

- No provider plugin or hook code.
- No SDK/proto/ledger changes.
- No frozen corpus edits.
- No README/CHANGELOG closeout; slice 2 owns it.
