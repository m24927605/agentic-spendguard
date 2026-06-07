# `DEMO_MODE=envoy_extproc` (COV_07 — D01 final slice)

Demo bundle that proves the SLICE 1-6 stack of `services/envoy_extproc`
gates real LLM-bound traffic through a stock Envoy + ExtProc filter
chain. After this demo, the D01 deliverable is COMPLETE.

## Topology

```
client (run_demo.py)
   |
   v
envoy-gateway:10000  (envoyproxy/envoy:v1.31-latest)
   |  ExtProc filter (failure_mode_allow=false)
   |
   +---> envoy-extproc:9443      (services/envoy_extproc, uds-dev)
   |        |
   |        +-> sidecar UDS at /var/run/spendguard/adapter.sock
   |
   +---> counting-stub:8765      (mock OpenAI provider, counts calls)
```

Envoy applies the SpendGuard decision (CONTINUE / immediate_response 4xx)
BEFORE forwarding to the counting-stub upstream. The Strategy A
reservation lives on the sidecar side; envoy_extproc is a translation
layer per design §3.1.

## Files

| Path | Purpose |
|------|---------|
| `docker-compose.yaml` | Sidecar compose overlay declaring the 3 new services (counting-stub, envoy-extproc, envoy-gateway). Layered on `deploy/demo/compose.yaml`. |
| `envoy-config.yaml` | Envoy v3 bootstrap config wiring the ExtProc filter for `/v1/chat/completions`. |
| `README.md` | This file. |

## Bring-up

```bash
make demo-up DEMO_MODE=envoy_extproc
```

The Makefile target:

1. Brings up the base `compose.yaml` infrastructure (postgres + ledger +
   canonical-ingest + sidecar + outbox-forwarder), gated on
   sidecar `/readyz`.
2. Layers `envoy_extproc/docker-compose.yaml` on top: counting-stub,
   envoy-extproc, envoy-gateway in that order (depends_on healthchecks).
3. Runs `run_demo.py` inside the demo container with `SPENDGUARD_DEMO_MODE=envoy_extproc`.
   The driver issues 3 calls through the Envoy gateway:

   - **ALLOW**: small estimate → HTTP 200, counting-stub `+1`.
   - **DENY**: `spendguard_estimate_override=2_000_000_000` → blows past
     the seeded 1B hard-cap → SpendGuard ExtProc returns
     `immediate_response 4xx` → counting-stub counter UNCHANGED.
   - **STREAM**: `stream=true` body within budget → HTTP 200,
     counting-stub `+1` (response-body BUFFERED commit at end-of-stream).
4. Runs `verify_step_envoy_extproc.sql` for ledger-side gates plus a
   cross-DB canonical_events check inside the Makefile.

## Gates

Each gate is fail-loud (exit 7 on failure):

- ALLOW counter increment (`+1`).
- DENY counter unchanged (negative control; INV-2 strict-order proof:
  sidecar reserve happens BEFORE any upstream call).
- STREAM counter increment (`+1`).
- Ledger: `reserve >= 2`, `commit_estimated >= 2`, `denied_decision >= 1`.
- canonical_events: `decision >= 2`, `outcome >= 1`.

## Success line

```
[demo] envoy_extproc ALL 3 steps PASS (ALLOW + DENY + STREAM)
```

The literal text is review-standards §8.1 + §6.7-aligned LOCKED spelling
mirroring the litellm_guardrail demo — CI automation greps for it.

## Carve-outs

- **uds-dev transport** — the demo doesn't ship cert-manager so
  envoy-extproc is built `--features uds-dev` and dials the sidecar
  over UDS. Production Helm wiring (SLICE 6) builds the binary
  `--no-default-features` and routes through mTLS-over-TCP. See
  design §3.3 and review-standards §7.1.
- **No AI Gateway control plane** — the demo runs stock
  `envoyproxy/envoy:v1.31-latest`. Customers running Envoy AI Gateway
  v0.6 configure the ExternalProcessor CR themselves; the
  `envoy-config.yaml` here is a reference snippet.
- **No streaming SSE budget enforcement** — Response-Body mode is
  `BUFFERED`; the commit lane runs at end-of-stream per design §3.5.
