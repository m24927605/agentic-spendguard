# Round-2 #3 — Helm production env mapping

GitHub issue: #3. Original prompt: `../08-issue-3-helm-prod-env-mapping.md`.

## Why round 2

Original prompt's verification required kind-cluster e2e (5 services, all
healthy, first-decision smoke). Outside autonomous-session capability.

**Round-2 strategy**: ship the env wiring as a code-only PR. Operator
validates on their cluster post-merge. The chart-level `chart.profile=production`
fail-gate is **kept** until operator confirms — flipping it off is a
separate one-line PR after their kind-cluster validation.

## Per-PR shape (one service per PR — same split as round-2 #11)

For service `S`:

1. Read `services/S/src/config.rs` — list every `#[serde(default = ...)]` and
   every required field (no default).
2. Update `charts/spendguard/templates/<S>.yaml` env block to wire every
   required field. Use Values knobs that mirror the env names; reference
   secrets via `existingSecret`.
3. `charts/spendguard/values.yaml` — document the new knobs in the
   "Required by the operator before install" header at top.
4. Smoke: `helm lint` clean + `helm template ... --set chart.profile=demo`
   renders cleanly (production-profile fail-gate stays asserted until
   the operator-side kind validation lands separately).

## Acceptance per PR

- [ ] `helm lint charts/spendguard` clean
- [ ] `helm template t charts/spendguard --set chart.profile=demo ...`
  renders without error
- [ ] PR body lists each new env wired + which Config field it backs
- [ ] PR body explicitly defers the kind-cluster validation to operator
  + leaves the `chart.profile=production` fail-gate in place

## Ship order

(Smallest mismatches first):
- ttl-sweeper (TLS naming + FENCING_INITIAL_EPOCH)
- outbox-forwarder (TLS naming + SCHEMA_BUNDLE_ID + SCHEMA_BUNDLE_HASH_HEX)
- canonical-ingest (REGION + INGEST_SHARD_ID; remove unused TENANT_ID)
- webhook-receiver (TLS_SERVER + TLS_CLIENT + BIND/HEALTH_ADDR + per-tenant secrets)
- sidecar (largest — contract bundle + schema bundle + MTLS_BOOTSTRAP_TOKEN
  + ENDPOINT_CATALOG_MANIFEST_URL + trust-root pins)

## Reference

`deploy/demo/compose.yaml` is the working ground truth for env names per
service. Every required field gets exercised in the compose path; mirror
that into Helm.
