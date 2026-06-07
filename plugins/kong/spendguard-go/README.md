# SpendGuard Kong AI Gateway plugin (Go)

`spendguard` is a Kong Gateway plugin that adds SpendGuard reserve →
commit guardrails to upstream LLM calls. It runs in Kong's `access`
phase (reserve) and `body_filter` phase (commit), and speaks to the
SpendGuard sidecar over **mTLS-only HTTPS**.

This directory is the **Go** implementation, the supported production
path per `docs/specs/coverage/D09_kong_ai_gateway/design.md` §3.2.
A Lua fallback lives in `plugins/kong/spendguard-lua/` (lands SLICE
5) and is explicitly labelled experimental.

## Status

- **SLICE 2 (this slice)** — scaffold only. `Access` + `BodyFilter`
  are no-ops; the plugin loads cleanly and Kong's `kong reload`
  shows it in the registry.
- SLICE 3 implements the `Access` reserve flow.
- SLICE 4 implements the `BodyFilter` commit flow.
- SLICE 6 ships the Helm chart + `KongPlugin` CRD examples.
- SLICE 7 ships `DEMO_MODE=kong_gateway_real` against real OpenAI.

## Layout

| File | Purpose |
|------|---------|
| `main.go` | Plugin entry point; registers `Access` + `BodyFilter` hooks via `github.com/Kong/go-pdk/server`. |
| `config.go` | `Config` struct — `SidecarURL`, `TenantID`, `FailOpen`, `TimeoutMS`, mTLS PEM paths. |
| `Makefile` | `make build` for the static binary, `make build-kong-plugin` for the legacy `.so` smoke gate. |
| `go.mod` | Pins `github.com/Kong/go-pdk v0.11.0` (review-standards §3.2). |

## Build

```bash
# Static binary (Kong launches it as a plugin-server subprocess).
make build

# Legacy .so (D09 SLICE 2 verification gate).
make build-kong-plugin

# Both targets emit into ../../../target/kong/ at the workspace root.
ls ../../../target/kong/
```

`make build-kong-plugin` produces `target/kong/spendguard.so` on Linux
amd64 and macOS arm64. Windows is unsupported because Go's
`-buildmode=plugin` does not work there; the Makefile fails fast with
a clear error on Windows.

## Configuration

Plugin config keys live on the `KongPlugin` CRD (SLICE 6 ships the
reference example):

| Key | Default | Purpose |
|-----|---------|---------|
| `sidecar_url` | _(required)_ | HTTPS URL of the sidecar HTTP companion (`https://spendguard-kong-companion.<ns>.svc.cluster.local:8443`). |
| `sidecar_ca_pem` / `sidecar_ca_file` | _(required, mutually exclusive)_ | CA bundle that signs the sidecar's workload cert. |
| `client_cert_pem` / `client_cert_file` | _(required)_ | Plugin workload cert (SVID-style URI SAN). |
| `client_key_pem` / `client_key_file` | _(required)_ | Matching private key. |
| `tenant_id` | _(required)_ | Tenant assertion. Must match the SVID URI SAN tenant. |
| `fail_open` | `false` | Fail-closed default per design §3.4. When `true`, sidecar errors degrade-to-allow with a log warning. |
| `timeout_ms` | `500` | Per-request HTTP timeout to the sidecar. Timeouts map to DEGRADE, not hard error. |

## Plugin priority

`spendguard` runs at priority **950**, above `key-auth` (1003) but
below `ai-proxy` (770) wait, actually above — Kong's "higher number
runs earlier" semantics mean 950 fires **before** `ai-proxy` (770),
so the reserve happens upstream of upstream auth. See
`docs/specs/coverage/D09_kong_ai_gateway/review-standards.md` §6.3
for the constraint.

## SLICE 2 scope guardrail

This slice deliberately:

- Does **not** make any HTTP calls to the sidecar.
- Does **not** parse provider payloads (OpenAI / Anthropic shape
  detection lands in SLICE 3).
- Does **not** ship `KongPlugin` CRD manifests (SLICE 6).
- Does **not** ship the Lua fallback (SLICE 5).

The Go module compiles, `make build-kong-plugin` produces a loadable
`.so` (where the toolchain supports it), and Kong's plugin-server
registers `spendguard` in both `Access` and `BodyFilter` phases —
that's the entire slice gate per review-standards §3.1-§3.6.

## Repository contract

- License: Apache-2.0 (matches the rest of the customer plugin
  template).
- DCO sign-off required on contributions (matches the
  `customer-plugin-template/` policy).
- No upstream Kong PR. Plugin lives in this repo; distribution is
  via the `KongPlugin` CRD (SLICE 6) or `kong.conf` snippets.
