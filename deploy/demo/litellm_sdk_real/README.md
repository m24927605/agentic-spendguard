# `DEMO_MODE=litellm_sdk_real` (COV_D12 SLICE 7)

Demo bundle that proves the **LiteLLM SDK monkey-patch shim**
(`spendguard.integrations.litellm_sdk_shim`) gates direct
`litellm.acompletion()` calls — and transitive callers (CrewAI, DSPy,
SmolAgents, Strands, BeeAI, AutoGen, Atomic Agents) — **before** the
upstream OpenAI HTTP request leaves the process.

This is distinct from:

- `DEMO_MODE=litellm_real` (legacy proxy-mode `CustomLogger` callback —
  `deploy/demo/litellm_proxy/`).
- `DEMO_MODE=litellm_guardrail` (newer proxy-mode `CustomGuardrail`
  registry — `deploy/demo/litellm_guardrail/`).
- `DEMO_MODE=litellm_direct` (explicit `SpendGuardDirectAcompletion`
  wrapper — D11 SLICE A3).

D12 closes the gap [LiteLLM Issue #8842](https://github.com/BerriAI/litellm/issues/8842)
leaves open: `async_pre_call_hook` only fires on the proxy path; direct
SDK callers have no pre-call gate. The shim monkey-patches the SDK
entry points (`acompletion`, `completion`, `atext_completion`,
`text_completion`, `Router.acompletion`) so SpendGuard reserves
BEFORE the provider HTTP request leaves the process.

## Files

| Path | Purpose |
|------|---------|
| `docker-compose.yaml` | Overlay that adds `counting-stub` (mock OpenAI provider) + `litellm-sdk-shim-runner` (Python 3.12 calling real `litellm` after `install_shim`) |
| `run_litellm_sdk_demo.py` | Driver: 3-step matrix (ALLOW + STREAM + TRANSITIVE/CrewAI) |
| `README.md` | This file |

## Bring-up

```bash
make demo-up DEMO_MODE=litellm_sdk_real
```

The Makefile target:

1. Boots the base stack (`postgres + sidecar + ledger + canonical-ingest +
   outbox-forwarder`) using `deploy/demo/compose.yaml`.
2. Adds the `counting-stub` (mock OpenAI provider) and the
   `litellm-sdk-shim-runner` service from `docker-compose.yaml`.
3. Runs the 3-step driver and asserts INV-1 (stub counter delta) +
   INV-2 (ledger order: reserve before outcome).
4. Verifies the ledger gates in `verify_step_litellm_sdk_real.sql`.

## Driver matrix

The runner script installs the shim with `install_shim(...)` and then:

- **Step A — ALLOW**: `await litellm.acompletion(model="gpt-4o-mini", ...)`.
  The shim reserves on the sidecar; the counting-stub answers as the
  OpenAI upstream; the shim commits real usage from `response.usage`.
  Stub counter MUST increment by exactly 1.
- **Step B — STREAM**: same shape but `stream=True`. The shim's
  end-of-stream commit path runs; stub counter increments by 1.
- **Step C — TRANSITIVE / CrewAI**: build a `crewai.Agent` + `Task` +
  `Crew` and call `kickoff_async()`. CrewAI uses `litellm.acompletion`
  under the hood, so the shim gates each call **without any CrewAI
  code changes**. This proves the D12 thesis for the 7 frameworks
  (CrewAI / DSPy / SmolAgents / Strands / BeeAI / AutoGen / Atomic
  Agents) that all route through litellm.

If `crewai` is not importable, Step C is a clean SKIP — the demo still
passes on Steps A + B.

## Gates

Each gate is fail-loud (driver exits non-zero on failure):

- ALLOW counter increment (+1).
- STREAM counter increment (+1).
- Ledger: `reserve >= 2` (ALLOW + STREAM), `commit_estimated >= 2`,
  `audit_outbox` carries `decision_context.mode='sdk'` for at least
  one row.
- canonical_events `source_integration='litellm'` (after the outbox
  forwarder drains).

## Success line

```
[litellm-sdk-runner] litellm_sdk_real ALL 3 steps PASSED
```

The literal text is LOCKED for CI grep automation.

## Distribution

D12 ships **via the existing `spendguard-sdk` PyPI package** — no new
package, no new extras. The shim lives at
`spendguard.integrations.litellm_sdk_shim` and is available wherever
`spendguard-sdk>=0.5.1` is installed (the `litellm` extra still drags
in `litellm[proxy]` for the legacy callback path, which the shim does
not need but does not conflict with either).
