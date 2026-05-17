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

## Operator-supplied Secrets

The chart consumes seven Secrets that the operator must pre-create.
Each row lists the default name (overridable via `values.yaml`), the
required keys, and the producer / verifier role.

| Secret (default name)             | Required keys                                                                                                                                                                                                                                                  | Why                                                                                                                                       |
|-----------------------------------|----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|-------------------------------------------------------------------------------------------------------------------------------------------|
| `spendguard-tls`                  | `ca.crt`, `ledger.crt`, `ledger.key`, `canonical-ingest.crt`, `canonical-ingest.key`, `sidecar.crt`, `sidecar.key`, `webhook-receiver.crt`, `webhook-receiver.key`, `outbox-forwarder.crt`, `outbox-forwarder.key`, `ttl-sweeper.crt`, `ttl-sweeper.key` | mTLS workload certs. Production: provision via cert-manager + a workload-identity ClusterIssuer so rotation is automatic.                |
| `spendguard-bundles`              | `contract_bundle_tgz`, `contract_bundle_sig`, `contract_bundle_metadata_json`, `schema_bundle_tgz`, `runtime.env`, `pricing.env`                                                                                                                                | Pre-pulled signed contract + schema bundles + the env files sidecar/canonical-ingest read at startup.                                    |
| `spendguard-webhook-hmac`         | `hmac` (binary, ≥32 bytes)                                                                                                                                                                                                                                     | Webhook receiver verifies provider POSTs with this shared secret.                                                                        |
| `spendguard-manifest-verify-key`  | `manifest_verify_key.pub.pem` (ed25519 public key)                                                                                                                                                                                                             | Sidecar pins the endpoint-catalog manifest signer's public key here; refuses to boot without it.                                          |
| `spendguard-signing-keys`         | `ledger.pem`, `sidecar.pem`, `webhook-receiver.pem`, `ttl-sweeper.pem` (ed25519 private keys, PKCS8 PEM)                                                                                                                                                       | Per-producer signing keys for the audit chain. KMS-backed mode (S6) replaces this with `signing.kms.kmsKeyArn` per service.              |
| `spendguard-trust`                | `ca.pem`                                                                                                                                                                                                                                                       | Trust-root pin. The chart additionally checks `sidecar.trustRootSpkiSha256Hex` against this PEM's SubjectPublicKeyInfo sha256.            |
| `spendguard-mtls-bootstrap`       | `token` (binary, one-shot, ≥32 bytes)                                                                                                                                                                                                                          | Bootstrap token consumed by the cert-manager external issuer at first boot to provision workload certs.                                  |

Secret names are overridable via:
- `secrets.tls.existingSecret`
- `secrets.bundles.existingSecret`
- `webhookReceiver.hmacSecretName`
- `sidecar.manifestVerifyKey.existingSecret`
- `signing.existingSecret`
- `sidecar.trustSecret.name`
- `sidecar.mtlsBootstrapTokenSecret.name`

For a working example of all seven Secrets being created with structurally-valid contents (self-signed CA + ed25519 keys + deterministic bundle .tgz), see
[`../../scripts/helm-validate-kind.sh`](../../scripts/helm-validate-kind.sh).
That script is also wired into CI via
[`.github/workflows/helm-validate.yml`](../../.github/workflows/helm-validate.yml).

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
