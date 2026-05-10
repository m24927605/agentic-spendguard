# Followup #3 — Helm chart full production env mapping

GitHub issue: https://github.com/m24927605/agentic-flow-cost-evaluation/issues/3

## Goal

Wire every required `Config::from_env()` field for 5 services into the Helm
chart so `helm install --set chart.profile=production ...` produces healthy
pods that actually boot. Today the chart fail-gates at template render
(PR #2 round 6, commit `3b9eccb`) because the templates are missing
required env vars and have wrong TLS naming for some services.

Sequencing: do this **last** after issues #5 (k8s lease), #8 (live KMS),
#10 (retention sweeper), #11 (metrics). By then most services have their
full config landscape settled and you wire all of it together.

## Files to read first

- Each affected service's `src/config.rs` — the source of truth for what
  envy reads. Required fields have no `#[serde(default = "...")]`:
  - `services/canonical_ingest/src/config.rs`
  - `services/sidecar/src/config.rs`
  - `services/outbox_forwarder/src/config.rs`
  - `services/ttl_sweeper/src/config.rs`
  - `services/webhook_receiver/src/config.rs`
- Each Helm template's current env block:
  - `charts/spendguard/templates/canonical-ingest.yaml`
  - `charts/spendguard/templates/sidecar.yaml`
  - `charts/spendguard/templates/outbox-forwarder.yaml`
  - `charts/spendguard/templates/ttl-sweeper.yaml`
  - `charts/spendguard/templates/webhook-receiver.yaml`
- `charts/spendguard/values.yaml` — knob structure
- PR #2 commit `3b9eccb` — the fail-gates that prevent prod deploy
  today; remove them once mapping is real
- `deploy/demo/compose.yaml` — the **working reference**. compose-based
  demo wires every env var correctly; mirror its env blocks into Helm

## Per-service required fields (from Codex round 6 finding)

| Service | Missing env vars or naming mismatches |
|---|---|
| canonical-ingest | `SPENDGUARD_CANONICAL_INGEST_REGION`, `SPENDGUARD_CANONICAL_INGEST_INGEST_SHARD_ID`. Remove unused `TENANT_ID` env (service doesn't read it). |
| sidecar | `ENDPOINT_CATALOG_MANIFEST_URL`, `MTLS_BOOTSTRAP_TOKEN`, contract bundle ID + hash, schema bundle ID + hash, `FENCING_INITIAL_EPOCH`, trust-root CA + SPKI |
| outbox-forwarder | Rename `TLS_CERT_PEM`/`KEY_PEM` → `TLS_CLIENT_CERT`/`TLS_CLIENT_KEY`. Add `SCHEMA_BUNDLE_ID`, `SCHEMA_BUNDLE_HASH_HEX` |
| ttl-sweeper | Same TLS rename. Add `FENCING_INITIAL_EPOCH` |
| webhook-receiver | Rename current naming → `TLS_SERVER_CERT`/`TLS_SERVER_KEY` + `TLS_CLIENT_CERT`/`TLS_CLIENT_KEY`. Add `BIND_ADDR`, `HEALTH_ADDR`, `FENCING_INITIAL_EPOCH`, per-tenant `SPENDGUARD_WEBHOOK_SECRET_*` |

## Acceptance criteria

- Each Helm template wires every field listed above via `Values` keys +
  references to `existingSecret` / configMap as appropriate
- `values.yaml` gains the required knobs with documented defaults that
  match what `deploy/demo/compose.yaml` uses for each
- `chart.profile=production` fail-gate from PR #2 round 6 (commit
  `3b9eccb`) is **removed** from each of the 5 templates. The
  `signing.profile=production` + `signing.strictVerification=true` gate
  from S8 stays
- New required-secrets section in `values.yaml` documents every
  `existingSecret` that the operator must pre-create
- `helm install` on a kind cluster with `--set chart.profile=production`
  + the appropriate secrets + `signing.profile=production` brings up all
  5 services to `Running` state with passing healthchecks within 90s
- Smoke: send a single decision through the cluster (ingress to sidecar
  via `kubectl port-forward` over UDS-impossible-on-k8s — use the
  Python SDK's TCP fallback) and observe an audit_outbox row landed
- New / updated runbook `docs/site/docs/operations/helm-production-deploy.md`
  walking through: prerequisites, secrets, install, healthcheck, first
  decision

## Pattern references

- `deploy/demo/compose.yaml` is the gold standard for env wiring. The
  compose-based demo path works end-to-end (proven by 6-mode demo
  verify in PR #6). Mirror those env blocks into Helm with appropriate
  Values substitution
- PR #2 round 1 commit `a4dea4b` — Codex P1#1 fix that consolidated
  duplicate `ports:` blocks in canonical-ingest.yaml. Watch for similar
  YAML structure traps when adding many env vars

## Verification

```bash
helm lint charts/spendguard
helm template t charts/spendguard \
  --set chart.profile=production \
  --set signing.profile=production \
  --set signing.strictVerification=true \
  --set signing.mode=ed25519 \
  --set signing.existingSecret=test \
  --set secrets.tls.existingSecret=test \
  --set secrets.bundles.existingSecret=test \
  > /tmp/rendered.yaml
# expect: 5 Deployment kinds, no `helm fail` chart.profile=production gate

# kind-based e2e
kind create cluster --name sg-prod-test
kubectl create secret generic spendguard-tls --from-file=...
kubectl create secret generic spendguard-signing --from-file=...
kubectl create secret generic spendguard-bundles --from-file=...
helm install spendguard charts/spendguard \
  --set chart.profile=production \
  --set signing.profile=production \
  --set signing.strictVerification=true \
  --set signing.mode=ed25519 \
  --set signing.existingSecret=spendguard-signing \
  --set secrets.tls.existingSecret=spendguard-tls \
  --set secrets.bundles.existingSecret=spendguard-bundles
kubectl wait --for=condition=ready pod -l app.kubernetes.io/instance=spendguard --timeout=90s
# expect: all 5 pods Ready
```

## Commit + close

```
feat(helm): full production env mapping for 5 services (followup #3)

Wires the env blocks each Rust service's Config::from_env() actually
reads. Removes the chart.profile=production fail-gate from PR #2
round 6 — operators can now deploy real production via Helm.

Compose-based demo path remains supported for development.

Tests: helm lint + template + kind-based 5-pod readiness e2e.
First-decision smoke through Python SDK TCP fallback verified the
audit_outbox round-trip works on the production-profile cluster.
```

After merge: `gh issue close 3 --comment "Shipped in <commit-sha>"`.
