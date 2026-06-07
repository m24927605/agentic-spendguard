// Package main — Kong AI Gateway SpendGuard plugin (D09 SLICE 2 scaffold).
//
// This is the SLICE 2 entry point per
// `docs/specs/coverage/D09_kong_ai_gateway/design.md` §4 row 2 and
// `implementation.md` §3. The scaffold registers the plugin with
// Kong's go-plugin-server, declares the `Config` shape, and exposes
// empty `Access` + `BodyFilter` hooks.
//
// What this slice ships
//
//   - `New()` constructor with fail-closed defaults (config.go).
//   - `Access` hook stub. SLICE 3 wires it through to:
//     1. parse request body once,
//     2. resolve (provider, model) via `provider_route.go`,
//     3. POST /v1/tokenize → input_tokens,
//     4. POST /v1/decision → ALLOW / DENY / DEGRADE,
//     5. on DENY return `kong.response.exit(429)`,
//     6. on ALLOW store reservation_id in `kong.ctx.shared`,
//     7. on DEGRADE honor `FailOpen` per design §3.4.
//   - `BodyFilter` hook stub. SLICE 4 wires it through to:
//     1. accumulate response chunks until end-of-body,
//     2. parse provider usage,
//     3. POST /v1/trace with `LLM_CALL_POST.SUCCESS` (or RUN_ABORTED).
//
// Anti-scope (per design §5):
//
//   - No streaming SSE budget enforcement in v1.
//   - No Bedrock SigV4 mutation (Kong's `ai-proxy` handles).
//   - No Konnect (SaaS control plane) onboarding.
//
// Build: `make build-kong-plugin` runs `go build -buildmode=plugin`
// only for plugin-server distribution; the standard build emits a
// statically-linked binary that Kong runs as a long-lived subprocess
// per the go-plugin-server protocol.
//
//revive:disable:exported plugin entrypoints follow Kong PDK convention
package main

import (
	"github.com/Kong/go-pdk"
	"github.com/Kong/go-pdk/server"
)

// PluginVersion is reported to Kong's plugin-server and surfaced in
// `/metrics` labels (SLICE 6). Bump with every plugin .so revision so
// operators can correlate audit anomalies with deploys.
const PluginVersion = "0.1.0-d09-slice4"

// Priority is Kong's plugin execution-order field; higher values run
// earlier. `ai-proxy` is 770; SpendGuard MUST run before `ai-proxy`
// so the reserve happens upstream of upstream auth (review-standards
// §6.3 for the Lua port; the Go path mirrors the same ordering). 950
// keeps us above Kong's `key-auth` (1003) but below `pre-function`
// (1000000) so operators can still intercept for debugging.
const Priority = 950

// main is the plugin-server entry point. `server.StartServer` blocks
// until Kong tears down the connection. The integer `0` is the legacy
// "log verbosity" argument; we leave it at 0 and rely on Kong's
// global log level instead (review-standards §3.6 requires an
// explicit version argument; the verbosity is unrelated).
func main() {
	// review-standards §3.6: pass an explicit version string + 0
	// verbosity. Kong embeds the version in the plugin registry.
	_ = server.StartServer(New, PluginVersion, 0)
}

// Access is invoked by Kong on every request after the body has
// been buffered (we require `request_buffering: true` on the route
// per design §3.3). SLICE 3 wires the production reserve flow:
// parse body → tokenize → decision → ALLOW/DENY/DEGRADE per
// `implementation.md` §5.
//
// The production code is in `runAccess` (access.go); this method is
// a one-liner so the Kong plugin-server's reflection-based
// dispatcher can find the entry point.
func (c *Config) Access(k *pdk.PDK) {
	runAccess(k, c)
}

// BodyFilter is invoked repeatedly as Kong streams the upstream
// response back to the client. SLICE 4 accumulates chunks and emits
// the trace event on end-of-body via `runBodyFilter` (body_filter.go).
func (c *Config) BodyFilter(k *pdk.PDK) {
	runBodyFilter(k, c)
}
