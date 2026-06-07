-- D09 SLICE 5 — Lua fallback plugin rockspec.
--
-- LuaRocks distribution metadata for the experimental Lua port of the
-- SpendGuard Kong plugin. Install on a Kong DataPlane node via:
--
--   luarocks install spendguard
--
-- then set `plugins = bundled,spendguard` in `kong.conf` and reload.
--
-- Per `docs/specs/coverage/D09_kong_ai_gateway/design.md` §3.2 this
-- distribution is **experimental** and labeled as such in the
-- README. Production deployments should prefer the Go plugin under
-- `plugins/kong/spendguard-go/`.

package = "spendguard"
version = "1.0.0-1"

source = {
  url = "git+https://github.com/m24927605/agentic-spendguard.git",
  tag = "spendguard-lua-1.0.0",
  dir = "agentic-spendguard/plugins/kong/spendguard-lua",
}

description = {
  summary = "SpendGuard reserve→commit guardrail for Kong AI Gateway (Lua port).",
  detailed = [[
    Kong Gateway plugin that adds SpendGuard pre-call budget reservations
    and post-call commits to upstream LLM API traffic. Speaks the same
    JSON-over-HTTPS+mTLS contract as the supported Go plugin under
    plugins/kong/spendguard-go/.

    This is the **experimental** Lua port for Kong 3.0–3.5 deployments
    that cannot run a `go-plugin-server` subprocess alongside the
    worker. The Go plugin is the supported production path.
  ]],
  homepage = "https://spendguard.io/docs/integrations/kong-ai-gateway",
  license = "Apache-2.0",
  maintainer = "SpendGuard team <plugins@spendguard.io>",
}

dependencies = {
  "lua >= 5.1",
  -- lua-resty-http ships with stock OpenResty + Kong 3.0+. Pinning a
  -- floor that has the `ssl_client_cert` connect option (added in
  -- 0.16.0; we want at least 0.17 for the keepalive-after-mTLS fix).
  "lua-resty-http >= 0.17",
}

build = {
  type = "builtin",
  modules = {
    ["kong.plugins.spendguard.handler"]        = "kong/plugins/spendguard/handler.lua",
    ["kong.plugins.spendguard.schema"]         = "kong/plugins/spendguard/schema.lua",
    ["kong.plugins.spendguard.sidecar_client"] = "kong/plugins/spendguard/sidecar_client.lua",
  },
}
