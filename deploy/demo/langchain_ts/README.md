# `DEMO_MODE=langchain_ts` (COV_D04 SLICE 5)

Demo bundle that proves the **LangChain.js callback handler path**
(`SpendGuardCallbackHandler` from `@spendguard/langchain`) gates a
`ChatOpenAI.invoke()` call **before** the upstream OpenAI HTTP
request leaves the process, with a hard-cap deny short-circuit.

This is the JS/TS sibling of the Python LangChain demo
(`DEMO_MODE=agent_real_langchain`, `deploy/demo/demo/run_demo.py::run_langchain_mode`),
which uses the `SpendGuardChatModel` wrapper instead. Both paths must
keep working — design.md §6.1 (callback handler is TS idiom; subclass
wrapper is Python idiom).

## Files

| Path | Purpose |
|------|---------|
| `docker-compose.yaml` | Overlay declaring `counting-stub` + `langchain-runner` (Node 20) services |
| `README.md` | This file |

The actual Node script lives at
[`examples/langchain-ts/index.mjs`](../../../examples/langchain-ts/index.mjs)
— mounted read-only into the `langchain-runner` container at boot.

## Bring-up

```bash
make demo-up DEMO_MODE=langchain_ts
```

The `langchain-runner` container (`spendguard-langchain-runner`):

1. Stages `examples/langchain-ts/{package.json,index.mjs}` to a tmpfs
   so `npm install` can patch the SpendGuard halves to `file:` deps
   against the workspace's pre-built `sdk/typescript/dist/` +
   `sdk/typescript-langchain/dist/`.
2. Runs `npm install` (pulls `@langchain/core` + `@langchain/openai`
   from npm; resolves `@spendguard/sdk` + `@spendguard/langchain`
   locally).
3. Waits for `/var/run/spendguard/adapter.sock` to appear (sidecar
   readiness gate).
4. Connects + handshakes via `SpendGuardClient`, then drives 3
   `ChatOpenAI` calls:
   - ALLOW: small message within budget → counting-stub counter +1,
     SUCCESS commit.
   - DENY: extra body `spendguard_estimate_override=2000000000`
     blows past the seeded 1B hard-cap → contract evaluator emits
     `SPENDGUARD_DENY`; handler's `reserve()` throws
     `DecisionDenied`; counting-stub counter UNCHANGED.
   - STREAM: `streaming: true` keeps the SSE chunked path →
     counting-stub counter +1, end-of-stream commit reconciles
     real token usage.
5. Asserts ledger rows
   (`deploy/demo/verify_step_langchain_ts.sql`) and the cross-DB
   `canonical_events` enrichment (Makefile post-run block) after the
   outbox forwarder drains.

## Gates

Each step is fail-loud (Node exits 7 on failure):

- ALLOW counter increment (+1).
- DENY counter unchanged (negative control; INV-2 strict-order proof).
- STREAM counter increment (+1).
- Ledger: `reserve >= 2`, `commit_estimated >= 2`, `denied_decision >= 1`.
- canonical_events: `decision >= 2`, `outcome >= 1`.

## Success line

```
[demo] langchain_ts ALL 3 steps PASS (ALLOW + DENY + STREAM)
```

The literal text mirrors D11/6 (`litellm_guardrail`) §6.7 LOCKED
spelling — CI grep targets one canonical pattern across all demos.

## Why a separate overlay (not a base service)

Mirrors `deploy/demo/envoy_extproc/`'s "overlay carries only the
mode-specific services" convention:

- The base `compose.yaml` stays Python-only for boot speed (no Node
  layer in the standard path).
- `make demo-up DEMO_MODE=<other>` does not pay the Node-image pull
  cost.
- The `langchain-runner` service has explicit `depends_on:
  counting-stub` — the counting stub is itself overlay-scoped, so
  declaring both together is the cleanest way to avoid a stale base
  reference.

## Why pre-built dist (not `pnpm install` at boot)

The workspace's `sdk/typescript/dist/` + `sdk/typescript-langchain/dist/`
are committed-tree artefacts after SLICE 4. The `langchain-runner`
container resolves both via `file:/opt/spendguard/...` so the wire
shape exercised in the demo is byte-identical to what the production
publish bundle ships (project memory: demo-as-quality-gate). A
later slice can swap to a workspace `pnpm install` once a `node_modules`
mounting strategy lands that survives compose rebuilds.

## Related

- [`examples/langchain-ts/`](../../../examples/langchain-ts/) — standalone Node example
- [`docs/specs/coverage/D04_langchain_ts/`](../../../docs/specs/coverage/D04_langchain_ts/) — D04 spec
- [`sdk/typescript-langchain/`](../../../sdk/typescript-langchain/) — adapter source
- Sibling overlays: [`envoy_extproc/`](../envoy_extproc/), [`litellm_guardrail/`](../litellm_guardrail/)
