# D09 SLICE 5 — Lua plugin make targets.
#
# Two test surfaces:
#
#   make test          — runs busted (requires luarocks install busted)
#   make test-structure — shell-based structure/syntax checks (no Lua needed)
#
# CI runs `make test-structure` so the plugin is gated even on runners
# without OpenResty toolchain. The `make test` busted surface runs
# inside the demo container (deploy/demo/kong_gateway/) where stock
# Kong's OpenResty + busted are available.

.PHONY: test test-structure pack lint

# Busted spec — requires `luarocks install busted` + Kong's
# `spec.helpers` (ships with kong-2.x rocks). Run inside the demo
# container, not on the host.
test:
	cd $(CURDIR) && busted --verbose spec/

# Shell-based structure + syntax-like checks. Runs without any Lua
# toolchain installed. Verifies:
#   - schema fields match the Go plugin's Config struct field names
#   - handler exports PRIORITY=950 (matches Go plugin)
#   - sidecar_client implements all three /v1/* endpoints
#   - context-key strings match the Go plugin's constants
#   - no `os.execute`, `loadstring`, `dofile` — the plugin must not
#     shell out at runtime (review-standards §10.2)
test-structure:
	@bash ./scripts/test-structure.sh

# Pack a LuaRock tarball for distribution.
pack:
	luarocks pack ./spendguard-1.0.0-1.rockspec

# Run luacheck against the plugin sources. Requires `luarocks install
# luacheck`. Treats warnings as errors so unused locals, etc., fail.
lint:
	luacheck kong/ spec/
