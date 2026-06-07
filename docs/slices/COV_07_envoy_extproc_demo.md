# COV_07 — D01 Envoy ExtProc: demo mode (D01 closes)

> **Deliverable**: D01 Envoy AI Gateway ExtProc sidecar
> **Slice**: 7 of 7 (M) — FINAL D01 slice
> **Spec set**: [`docs/specs/coverage/D01_envoy_extproc/`](../specs/coverage/D01_envoy_extproc/)

## Scope

Land `make demo-up DEMO_MODE=envoy_extproc` — end-to-end demo proving the SLICES 1-6 stack against a real Envoy AI Gateway + counting stub. After this slice, D01 deliverable is COMPLETE.

Concretely:
- `deploy/demo/envoy_extproc/docker-compose.yaml` — NEW: postgres + sidecar + envoy-ai-gateway + envoy_extproc + counting-stub
- `deploy/demo/envoy_extproc/envoy-config.yaml` — Envoy AI Gateway ExtProc filter config pointing at envoy_extproc:9443
- `deploy/demo/demo/run_demo.py` — extend with `envoy_extproc` branch issuing 3 calls (ALLOW + DENY + STREAM) through Envoy + asserting sidecar reservation timestamp < stub call timestamp (INV-2)
- `deploy/demo/Makefile` — DEMO_MODE=envoy_extproc routing
- `deploy/demo/verify_step_envoy_extproc.sql` — cross-DB ledger/audit gates
- Uses `uds-dev` feature flag for the demo compose (per SLICE 6 carve-out — demo doesn't ship cert-manager)

## Files touched

| File | Why |
|------|-----|
| `deploy/demo/envoy_extproc/docker-compose.yaml` | NEW compose stack |
| `deploy/demo/envoy_extproc/envoy-config.yaml` | NEW Envoy ExtProc filter config |
| `deploy/demo/envoy_extproc/README.md` | Operator guide |
| `deploy/demo/Makefile` | DEMO_MODE routing |
| `deploy/demo/demo/run_demo.py` | envoy_extproc handler |
| `deploy/demo/verify_step_envoy_extproc.sql` | SQL gates |

## Test/verification plan

1. yaml parses
2. python compile + ruff clean
3. `make demo-up DEMO_MODE=envoy_extproc` end-to-end (may time out on docker build — declare as deviation if so, per SLICE D11/6 precedent)
4. INV-2 strict-order: sidecar reserve before stub call on ALLOW
5. DENY: stub call_count unchanged + 403 returned

## Anti-scope

- No Envoy AI Gateway control-plane changes
- No production cert-manager (demo uses uds-dev)

## Backlinks

- Spec set: [`design.md`](../specs/coverage/D01_envoy_extproc/design.md) §4 slice 7 row
- SLICE 6: [`COV_06_envoy_extproc_helm.md`](COV_06_envoy_extproc_helm.md)
