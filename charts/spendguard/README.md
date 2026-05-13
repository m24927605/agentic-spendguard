# Agentic SpendGuard Helm Chart

Deploy the SpendGuard sidecar topology to a Kubernetes cluster.

## What this chart deploys

- `ledger` Deployment + Service (mTLS gRPC :50051)
- `canonical-ingest` Deployment + Service (mTLS gRPC :50052)
- `sidecar` DaemonSet (UDS at hostPath; one pod per node)
- `webhook-receiver` Deployment + Service (mTLS HTTPS :8443 + healthz :8080)
- `outbox-forwarder` Deployment (no port; polls audit_outbox)
- `ttl-sweeper` Deployment (no port; polls expired reservations)
- `migrate` Job (post-install hook; placeholder — see "Migrations" below)

## What this chart does NOT deploy

- **Postgres**. Bring your own RDS / Cloud SQL / Azure DB / managed
  Postgres. Pass connection strings via `postgres.ledgerUrl` and
  `postgres.canonicalUrl`.
- **PKI certificates**. Pre-create a Secret matching
  `secrets.tls.existingSecret` with these keys:
    `ca.crt`, `ledger.crt`, `ledger.key`, `canonical-ingest.crt`,
    `canonical-ingest.key`, `sidecar.crt`, `sidecar.key`,
    `webhook-receiver.crt`, `webhook-receiver.key`,
    `outbox-forwarder.crt`, `outbox-forwarder.key`,
    `ttl-sweeper.crt`, `ttl-sweeper.key`.
  Use cert-manager + ClusterIssuer for production rotation.
- **Contract bundle**. Pre-create the bundles Secret (matching
  `secrets.bundles.existingSecret`) with
    `contract_bundle/<id>.tgz`, `contract_bundle/<id>.metadata.json`,
    `schema_bundle/<id>.tgz`, `runtime.env`, `pricing.env`.
- **Webhook HMAC secret**. Pre-create a Secret named per
  `webhookReceiver.hmacSecretName` with key `hmac`.

## Quickstart (local kind cluster)

```bash
kind create cluster --name spendguard

# Pre-create the required Secrets (use your own real values; this is
# a smoke-test set — do NOT use in production):
kubectl create secret generic spendguard-tls \
  --from-file=ca.crt=./local-pki/ca.crt \
  --from-file=ledger.crt=./local-pki/ledger.crt \
  ...

kubectl create secret generic spendguard-bundles \
  --from-file=runtime.env=./local-bundles/runtime.env \
  ...

kubectl create secret generic spendguard-webhook-hmac \
  --from-literal=hmac=$(openssl rand -hex 32)

helm install spendguard ./charts/spendguard \
  --set postgres.ledgerUrl="postgres://..." \
  --set postgres.canonicalUrl="postgres://..."

kubectl get pods -l app.kubernetes.io/name=spendguard
```

## POC limits enforced by chart defaults

- **Single-pod replicas** for `sidecar`, `outboxForwarder`,
  `ttlSweeper` (multi-pod = `producer_sequence` races; GA gate per
  Phase 2B Checkpoint §3.1).
- **Migration hook is placeholder.** Production deployments should
  override `migrations.ledgerImage` + the job's `args` to apply your
  preferred migration tool (sqitch / flyway / golang-migrate). See
  `services/ledger/migrations/*.sql` and
  `services/canonical_ingest/migrations/*.sql` for the sources.
- **No Postgres bundled.** This is intentional — production users
  always provide their own. For local kind testing, run a separate
  Postgres pod and point `postgres.ledgerUrl` at it.

## Local validation

```bash
helm lint charts/spendguard
helm template spendguard charts/spendguard --namespace test | kubectl apply --dry-run=client -f -
```

A `kind create cluster` + `helm install` end-to-end is the next
validation layer (deferred to operator docs).

## Versioning

Chart `version` follows the chart's own semver. `appVersion` tracks
the SpendGuard service image versions.
