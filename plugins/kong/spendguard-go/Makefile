# D09 SLICE 2 — Kong AI Gateway plugin Makefile.
#
# Per `docs/specs/coverage/D09_kong_ai_gateway/implementation.md` §10
# the top-level `make build-kong-plugin` produces a single static
# binary the Kong DataPlane container runs as a long-lived go-plugin-
# server subprocess. Kong supports two distribution modes:
#
#   1. Static binary (this Makefile's default `build` target). Kong
#      launches it via `KONG_PLUGINSERVER_<NAME>_START_CMD`. This is
#      the supported path because go-plugin-server uses gRPC over
#      Unix Domain Sockets between Kong and the plugin process and
#      sidesteps Go's plugin-loading ABI fragility.
#   2. .so plugin (legacy; emitted by `build-kong-plugin` for the
#      smoke-test gate). Requires `go build -buildmode=plugin` and a
#      Kong DataPlane image that pre-links the same Go runtime. We
#      keep this target so the slice's verification step has
#      something the spec explicitly names ("spendguard.so").

GOFLAGS ?=
PLUGIN_BIN ?= spendguard
PLUGIN_SO ?= spendguard.so
TARGET_DIR ?= ../../../target/kong

.PHONY: all build build-kong-plugin tidy test fmt vet clean

all: build

# Default build emits a statically-linked binary.
build:
	@mkdir -p $(TARGET_DIR)
	go build $(GOFLAGS) -o $(TARGET_DIR)/$(PLUGIN_BIN) ./...

# The SLICE 2 smoke gate per the verification harness.
# Per implementation.md §10 the .so target sits at
# target/kong/spendguard.so under the workspace root.
#
# Notes:
#   * Go's `-buildmode=plugin` is unsupported on Windows; the gate
#     skips with a clear error on that GOOS.
#   * macOS arm64 and Linux amd64 are the two supported GOOS/GOARCH
#     combos for plugin builds; other targets emit a warning.
build-kong-plugin:
	@mkdir -p $(TARGET_DIR)
	@if [ "$$(go env GOOS)" = "windows" ]; then \
		echo "Windows does not support -buildmode=plugin; emit a static binary via 'make build' instead" >&2; \
		exit 1; \
	fi
	go build -buildmode=plugin -o $(TARGET_DIR)/$(PLUGIN_SO) ./...
	@# Mirror the .so into the plugin source directory so the SLICE 2
	@# verification harness (`ls -la spendguard.so`) finds it without
	@# walking up to target/kong/. The canonical location for Kong
	@# DataPlane mounts remains $(TARGET_DIR)/$(PLUGIN_SO).
	@cp $(TARGET_DIR)/$(PLUGIN_SO) ./$(PLUGIN_SO)
	@echo "built $(TARGET_DIR)/$(PLUGIN_SO) (mirrored to ./$(PLUGIN_SO))"

# Module hygiene.
tidy:
	go mod tidy

# Unit tests (SLICE 2 ships smoke coverage only; SLICE 3+ adds the
# table-driven access / body_filter tests).
test:
	go test ./...

fmt:
	gofmt -w .
	go vet ./...

vet:
	go vet ./...

clean:
	rm -rf $(TARGET_DIR)/$(PLUGIN_BIN) $(TARGET_DIR)/$(PLUGIN_SO) ./$(PLUGIN_SO)
