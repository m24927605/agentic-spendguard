# SpendGuard Kong AI Gateway plugin (Lua) — EXPERIMENTAL

`spendguard` is a Kong Gateway plugin that adds SpendGuard reserve →
commit guardrails to upstream LLM API calls. This directory is the
**Lua** port — experimental, intended only for Kong 3.0–3.5
deployments that cannot host a `go-plugin-server` subprocess
alongside the worker.

> **Use the Go plugin (`plugins/kong/spendguard-go/`) for production.**
> Per [`docs/specs/coverage/D09_kong_ai_gateway/design.md`](../../../docs/specs/coverage/D09_kong_ai_gateway/design.md)
> §3.2 the Lua port covers only `access` (reserve) + `body_filter`
> (commit), does not get the conformance-test guarantee, and is not
> what we recommend operators ship in production.

## Status

| Phase         | Lua  | Go (production) |
|---------------|------|-----------------|
| `access`      | YES  | YES             |
| `body_filter` | YES  | YES             |
| Streaming SSE | NO   | NO (design §3.5)|
| Conformance   | NO   | YES (D09 SLICE 4) |

## Why an experimental Lua port

Some Kong DataPlane images (Alpine slim, vendor-locked OSS Kong)
cannot launch the `go-plugin-server` subprocess that the Go plugin
requires. Those images can still load a pure-Lua plugin. The Lua port
exists so SpendGuard remains installable on those topologies; it is
not a recommended path.

## Install

```bash
# 1. LuaRocks install on every Kong DataPlane node.
luarocks install spendguard

# 2. Enable in kong.conf (or KONG_PLUGINS env).
plugins = bundled,spendguard

# 3. Reload Kong.
kong reload
```

Or via declarative config (`kong.yml` for DB-less mode):

```yaml
plugins:
  - name: spendguard
    config:
      sidecar_url: https://spendguard-kong-companion.spendguard.svc.cluster.local:8443
      sidecar_ca_file: /var/run/secrets/spendguard/ca.crt
      client_cert_file: /var/run/secrets/spendguard/tls.crt
      client_key_file: /var/run/secrets/spendguard/tls.key
      tenant_id: 00000000-0000-4000-8000-000000000001
      fail_open: false
      timeout_ms: 500
```

## Configuration

The schema is field-equivalent to the Go plugin's `Config` struct;
both distributions accept the same `KongPlugin` CRD.

| Key | Required | Default | Purpose |
|-----|----------|---------|---------|
| `sidecar_url` | YES | — | HTTPS URL of the SpendGuard sidecar HTTP companion. Must start with `https://`. |
| `sidecar_ca_pem` / `sidecar_ca_file` | YES (one of) | — | CA bundle that signs the sidecar's workload cert. |
| `client_cert_pem` / `client_cert_file` | YES (one of) | — | Workload cert (SVID URI SAN encodes the tenant). |
| `client_key_pem` / `client_key_file` | YES (one of) | — | Matching private key. |
| `tenant_id` | YES | — | Tenant UUID; must match the SVID URI SAN tenant. |
| `fail_open` | YES | `false` | Fail-closed default per design §3.4. When `true`, sidecar errors degrade-to-allow with a `kong.log.warn`. |
| `timeout_ms` | YES | `500` | Per-request HTTP timeout to the sidecar (50–30000ms). |
| `budget_id` | NO | — | Optional explicit budget binding for multi-budget tenants. |
| `prompt_class` | YES | `general` | Contract evaluator prompt-class string. |

## Plugin priority

`PRIORITY = 950`, identical to the Go plugin. Above `ai-proxy`
(770) so the reserve fires **before** any upstream-auth plugin. See
[`docs/specs/coverage/D09_kong_ai_gateway/review-standards.md`](../../../docs/specs/coverage/D09_kong_ai_gateway/review-standards.md)
§6.3 for the rationale.

## Limitations vs Go plugin

- **No conformance gate** — the Go plugin runs against the D09
  conformance harness on every PR; this port does not.
- **No streaming SSE** — neither plugin caps mid-stream, but the Lua
  port also never even tries to inspect intermediate SSE frames.
- **No per-tenant SVID rotation hot-reload** — the Lua client caches
  the parsed cert per worker; a rotation requires `kong reload`.
  The Go plugin watches the cert files via inotify in SLICE 6+.
- **No Prometheus `/metrics` surface** — counters live in
  `kong.ctx.shared` only and don't ship to Prometheus. The Go plugin
  exposes the metrics surface via the plugin-server gRPC channel.

## Repository contract

- License: Apache-2.0 (matches the Go plugin).
- DCO sign-off required.
- No upstream Kong PR; the rockspec lives in this repo and gets
  published to LuaRocks under the `spendguard` package name.

## Demo

The Lua port is included in the Kong gateway demo for parity:

```bash
make demo-up DEMO_MODE=kong_gateway_real PLUGIN=lua
```

The default `PLUGIN=go` is the supported path. The `PLUGIN=lua`
override boots a Kong worker with this rockspec installed and runs
the same ALLOW + DENY + STREAM matrix the Go plugin runs.
