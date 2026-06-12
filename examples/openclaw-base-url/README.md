# SpendGuard + OpenClaw - base-URL recipe

D40a covers the durable OpenClaw path: configure a custom
OpenAI-compatible provider whose `baseUrl` points at the SpendGuard
egress proxy. SpendGuard enforcement happens in the egress proxy and
sidecar before the provider call. OpenClaw is not modified, and no
OpenClaw plugin is installed.

Spec:
[`docs/specs/coverage/D40a_openclaw_base_url_recipe/design.md`](../../docs/specs/coverage/D40a_openclaw_base_url_recipe/design.md)

Docs page:
[`docs/site-v2/src/content/docs/docs/drop-in/openclaw.mdx`](../../docs/site-v2/src/content/docs/docs/drop-in/openclaw.mdx)

## Config

Copy `openclaw.config.example.json` into your OpenClaw config path and
adjust the proxy URL for your topology:

| Topology | `models.providers.spendguard.baseUrl` |
|---|---|
| SpendGuard on the same host | `http://localhost:9000/v1` |
| Docker compose network | `http://spendguard-egress-proxy:9000/v1` |
| SpendGuard service | `https://spendguard.example.com/v1` |

The trailing `/v1` is mandatory. OpenClaw appends the
OpenAI-compatible request path behind the provider adapter.

The example `apiKey` is a non-secret placeholder used only to satisfy
OpenClaw's non-empty provider-field requirement. Do not put a real upstream
provider key in this OpenClaw config for local smoke tests or production use of
this recipe; the SpendGuard egress proxy owns upstream credentials and
substitutes the outbound authorization header.

## OpenClaw keys pinned by D40a

Primary OpenClaw docs at implementation time document:

- `agents.defaults.model.primary` with a `provider/model` ref
- `models.providers.<id>.baseUrl`
- `models.providers.<id>.apiKey`
- `models.providers.<id>.api: "openai-completions"`
- `models.providers.<id>.timeoutSeconds`
- `models.providers.<id>.models[].id`

The demo fixture uses provider id `spendguard` and model ref
`spendguard/gpt-4o-mini`.

## Verify in this repo

```bash
make demo-down
make demo-up DEMO_MODE=openclaw_base_url
make -C deploy/demo demo-verify-openclaw-base-url
```

The hard gate is local and does not require a live OpenAI key. The demo
routes SpendGuard's upstream call to a counting stub and proves:

- ALLOW increments the counting stub once.
- DENY returns a SpendGuard block and the counting stub counter is
  unchanged.
- STREAM returns Server-Sent Events and commits at stream close.

## What this is not

D40a is a configuration recipe. It is not OpenClaw provider plugin
coverage. D40b owns the plugin package and in-process adapter surface.
