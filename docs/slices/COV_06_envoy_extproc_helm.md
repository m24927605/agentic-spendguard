# COV_06 — D01 Envoy ExtProc: Helm sub-chart + mTLS-over-TCP transport switch

> **Deliverable**: D01 Envoy AI Gateway ExtProc sidecar
> **Slice**: 6 of 7 (S)
> **Spec set**: [`docs/specs/coverage/D01_envoy_extproc/`](../specs/coverage/D01_envoy_extproc/)

## Scope

Land the Helm sub-chart for envoy_extproc + hard-switch transport from UDS (SLICES 1-5 carve-out) to mTLS-over-TCP per design §3.3 + §3.4. Production deployment LOCKED at SLICE 6 per the spec carve-out.

Concretely:
- `charts/spendguard/templates/envoy_extproc.yaml` — NEW Helm template per design §4 slice 6 row:
  - Deployment + Service + ServiceAccount + ServiceMonitor + NetworkPolicy
  - SVID mount via cert-manager (mirrors `charts/spendguard/templates/output_predictor_plugin_svid.yaml` SPIFFE pattern from HARDEN_08)
  - NetworkPolicy ingress restricted to `app.kubernetes.io/name: envoy-ai-gateway`
  - ServiceMonitor for Prometheus scrape (envoy_extproc metrics surface)
- `charts/spendguard/values.yaml` — extend with envoy_extproc section:
  - replicas, image, resources, sidecar URL, SVID config
- `services/envoy_extproc/src/sidecar_client.rs` — hard-switch transport:
  - Add mTLS-over-TCP connect path with SPIFFE URI SAN pinning
  - Connect path determined by env var SPENDGUARD_EXTPROC_TRANSPORT={uds|tcp}; default tcp in production
  - UDS path preserved for dev/test (gated behind `#[cfg(test)]` or env var; production grep gate rejects UDS in src/)
- `services/envoy_extproc/src/main.rs` — wire `/readyz` HTTP probe (deferred from SLICE 1 per the spec carve-out — SLICE 6 lands it alongside Helm)
- Helm validation: `helm lint charts/spendguard/` + `helm template charts/spendguard/ --set envoyExtproc.enabled=true | kubectl apply --dry-run=server -f -` (if kubectl available)
- Production grep gate: `grep -rn "UnixListener\|UnixStream" services/envoy_extproc/src/` returns ONLY test or cfg(test)-gated lines per design §7.1 (carry-over: review-standards review check)

## Files touched

| File | Why |
|------|-----|
| `charts/spendguard/templates/envoy_extproc.yaml` | NEW Helm template |
| `charts/spendguard/values.yaml` | envoy_extproc section |
| `services/envoy_extproc/src/sidecar_client.rs` | mTLS-over-TCP connect path |
| `services/envoy_extproc/src/main.rs` | /readyz HTTP probe |
| `services/envoy_extproc/src/config.rs` | SPENDGUARD_EXTPROC_TRANSPORT env var |

## Test/verification plan

1. `cargo build --manifest-path services/envoy_extproc/Cargo.toml` clean
2. `cargo test --manifest-path services/envoy_extproc/Cargo.toml` — 90 SLICE 4 + 12 SLICE 5 conformance = 102 baseline + new transport unit tests
3. helm lint `charts/spendguard/` clean
4. helm template render with envoyExtproc.enabled=true produces valid Deployment + Service + ServiceMonitor + NetworkPolicy + SVID
5. Production UDS grep gate: zero hits in src/ outside cfg(test) / dev-paths

## Anti-scope

- No new demo mode — SLICE 7
- No production cert-manager bootstrap (assume cluster operator provides cert-manager + ClusterIssuer)
- No Envoy AI Gateway control-plane changes

## Backlinks

- Spec set: [`design.md`](../specs/coverage/D01_envoy_extproc/design.md) §3.3 transport carve-out, §3.4 fail-closed, §4 slice 6 row, §7 production grep gate
- SLICE 5: [`COV_05_envoy_extproc_conformance.md`](COV_05_envoy_extproc_conformance.md)
- HARDEN_08 SVID pattern: [[project_harden_08_shipped]] — per-tenant SVID minting carries over to envoy_extproc
