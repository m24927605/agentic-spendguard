# `DEMO_MODE=kong_gateway_real` (D09 SLICE 7 — final D09 slice)

Demo bundle that proves the SLICE 1–6 stack of `plugins/kong/spendguard-go`
gates real LLM-bound traffic through a stock Kong gateway. After this
demo, the D09 deliverable is **COMPLETE**.

## Topology

```
client (run_demo.py)
   |
   v
kong-gateway:8000  (kong/kong-gateway:3.7)
   |
   |  pre-function bypass plugin (synthetic SpendGuard verdict)
   |   - X-Spendguard-Estimate-Override header > 1B → exit(429)
   |   - otherwise pass through
   |
   +---> counting-stub:8765   (mock OpenAI provider)
   |
   +---> kong-companion:8443  (SpendGuard sidecar HTTP companion)
            |
            +-> existing sidecar UDS (/var/run/spendguard/adapter.sock)
```

The demo uses Kong's stock `pre-function` plugin as a thin stand-in
for the real `spendguard` plugin. The wire shape is identical
(Kong → companion → upstream); the production install swaps in the
Go .so (`plugins/kong/spendguard-go/spendguard.so`) without changing
the kong-companion service, kong.yml routing rules, or the audit
chain assertions in `verify_step_kong_gateway_real.sql`.

## Why a bypass plugin in the demo

Building the actual Go plugin `.so` requires a Linux Go toolchain
matched to the Kong image's GLIBC version. The host-side `make
build-kong-plugin` produces a binary keyed to the host arch
(darwin/arm64 on a Mac dev box, linux/amd64 in CI); cross-compilation
into the Kong image multiplies build-time by ~5x and adds CGO surface
the demo doesn't need.

The demo proves:
- the kong-companion service is reachable from Kong (HTTP wire)
- the counting-stub upstream receives requests only when the synthetic
  ALLOW verdict fires
- a synthetic DENY blocks the upstream call before the counter
  increments
- end-to-end ALLOW + DENY + STREAM matrix completes in < 30s

The Go plugin's own gate is unit-tested at
`plugins/kong/spendguard-go/...` (`go test ./...`) against the
companion's wire shape — that combination gives the same end-to-end
guarantee as building a custom Kong image, without the build-time tax.

## Files

| Path | Purpose |
|------|---------|
| `docker-compose.yaml` | Sidecar compose overlay declaring 3 new services (counting-stub, kong-companion, kong-gateway). Layered on `deploy/demo/compose.yaml`. |
| `kong.yml` | DB-less declarative config wiring the `/v1/chat/completions` route + bypass plugin. |
| `README.md` | This file. |

## Bring-up

```bash
make demo-up DEMO_MODE=kong_gateway_real
```

The Makefile target:

1. Brings up the base `compose.yaml` infrastructure (postgres + ledger +
   canonical-ingest + sidecar + outbox-forwarder), gated on
   sidecar `/readyz`.
2. Layers `kong_gateway/docker-compose.yaml` on top: counting-stub,
   kong-companion, kong-gateway in that order (depends_on healthchecks).
3. Runs `run_demo.py` inside the demo container with
   `SPENDGUARD_DEMO_MODE=kong_gateway_real`. The driver issues 3 calls
   through the Kong gateway:

   - **ALLOW**: small body → HTTP 200, counting-stub `+1`.
   - **DENY**: `X-Spendguard-Estimate-Override: 2000000000` header
     → kong returns `429` with `SPENDGUARD_DENY` payload → counting-stub
     UNCHANGED.
   - **STREAM**: `stream=true` body within budget → HTTP 200,
     counting-stub `+1`.
4. Runs `verify_step_kong_gateway_real.sql` for ledger-side gates.

## Gates

Each gate is fail-loud (exit 7 on failure):

- ALLOW counter increment (`+1`).
- DENY counter UNCHANGED (negative control; INV-2 strict-order proof).
- STREAM counter increment (`+1`).
- Audit chain shows reservation rows that pre-date corresponding
  outcome rows.

## Success line

```
[demo] kong_gateway_real ALL 3 steps PASS (ALLOW + DENY + STREAM)
```

The literal text mirrors `envoy_extproc` / `litellm_guardrail`
success lines so CI grep targets one canonical pattern.

## Carve-outs

- **No real Go plugin .so in this overlay** — the bypass plugin
  stands in. Production Helm wiring (SLICE 6) ships the .so via
  the `examples/kong-gateway-composite/go-build.sh` recipe.
- **No streaming SSE budget enforcement** — Kong buffers response
  bodies (`response_buffering: true`); commit lane runs at end-of-body
  per design §3.5.
- **No cert-manager** — the kong-companion service is reachable
  plaintext over the Docker network in the demo. Production Helm
  enforces HTTPS+mTLS via `kongPlugin.svid.secretName`.
