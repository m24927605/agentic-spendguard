# DEMO_MODE=openclaw_base_url

D40a's local hard gate for the OpenClaw base-URL recipe. The runner validates
the pinned OpenClaw custom-provider config shape and then emits matching
OpenAI-compatible traffic through the SpendGuard proxy; it does not embed the
full OpenClaw gateway binary.

Topology:

```text
openclaw-runner
  -> committed OpenClaw custom-provider config fixture
  -> http://egress-proxy:9000/v1/chat/completions
  -> sidecar over UDS/gRPC
  -> ledger + audit chain
  -> counting-stub
```

The runner validates the OpenClaw config keys pinned by the primary
OpenClaw docs, then drives three OpenAI-compatible calls:

- ALLOW: provider counter increments once.
- DENY: this overlay lowers the demo hard-cap threshold to 100 atomic units;
  a normal `max_tokens: 256` request is blocked before dispatch and the
  provider counter remains unchanged.
- STREAM: SSE frames include usage and commit after stream close.

Run from repo root:

```bash
make demo-down
make demo-up DEMO_MODE=openclaw_base_url
make -C deploy/demo demo-verify-openclaw-base-url
```

Successful runner line:

```text
[demo] openclaw_base_url ALL 3 steps PASS (ALLOW + DENY + STREAM)
```
