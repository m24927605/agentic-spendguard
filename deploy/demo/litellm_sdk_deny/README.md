# `DEMO_MODE=litellm_sdk_deny` (COV_D12 SLICE 7)

Companion to `DEMO_MODE=litellm_sdk_real`. Where `litellm_sdk_real`
exercises the happy ALLOW path, `litellm_sdk_deny` exercises the
3 fail-closed sub-steps that prove INV-1 (DENY never reaches the
upstream provider) under each operator-visible failure mode the
shim is documented to handle.

## Sub-step matrix

| # | Sub-step | Trigger | Expected |
|---|----------|---------|----------|
| 1 | ALLOW positive control | normal call within budget | sidecar reserves + commits; stub counter +1 |
| 2 | DENY budget exhausted | install_shim with a non-existent budget_id | sidecar returns DENY (binding validator); `DecisionDenied` raised; stub counter UNCHANGED |
| 3 | DENY sidecar unreachable | install_shim with a bogus UDS path | `SidecarUnavailable` / `HandshakeError` raised; stub counter UNCHANGED |

Sub-steps 2 + 3 are the load-bearing INV-1 negative controls: a
buggy shim that lets EITHER failure mode through silently would
corrupt the budget ledger.

## Bring-up

```bash
make demo-up DEMO_MODE=litellm_sdk_deny
```

## Files

| Path | Purpose |
|------|---------|
| `docker-compose.yaml` | Overlay (counting-stub + litellm-sdk-shim-runner DENY variant) |
| `run_litellm_sdk_deny_demo.py` | 3-substep driver |
| `README.md` | This file |

## Gates

- ALLOW positive control counter increment (+1).
- DENY sub-step 2 + 3 counter UNCHANGED.
- Ledger: `denied_decision >= 1` (sub-step 2 reaches the sidecar and
  surfaces DENY). Sub-step 3 fails before the sidecar so no ledger row
  on that side.
- canonical_events recorded for the ALLOW + DENY paths.

## Success line

```
[litellm-sdk-deny-runner] litellm_sdk_deny ALL 3 sub-steps PASSED
```

LOCKED for CI grep automation.
