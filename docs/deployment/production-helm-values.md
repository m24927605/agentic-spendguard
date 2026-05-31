# Production Helm Values

This guide defines the production Helm values contract for SpendGuard. The
reference file is `charts/spendguard/values-production.example.yaml`; validate it
with `scripts/release/validate-production-helm-values.sh`.

## Install Posture

Production installs must set:

```yaml
chart:
  profile: production
```

The production profile is fail-closed at render time for missing database Secret
references, unsigned audit mode, placeholder bundle hashes, missing mTLS/SVID
settings, disabled NetworkPolicy without explicit acknowledgement, and missing
control-plane JWT issuer configuration.

## Secret Contract

Do not put secret material in Helm values. Pre-create Kubernetes Secrets and
reference them by name/key.

| Values path | Secret purpose |
|---|---|
| `postgres.existingSecret` | Database URLs for ledger, canonical ingest, control plane, tokenizer, output predictor, run cost projector, and stats aggregator |
| `secrets.tls.existingSecret` | Workload mTLS material for core SpendGuard services |
| `secrets.bundles.existingSecret` | Signed contract/schema bundles and runtime env payloads |
| `signing.existingSecret` | Ed25519 audit signing keys and canonical ingest trust store |
| `sidecar.manifestVerifyKey.existingSecret` | Endpoint catalog signing public key |
| `sidecar.trustSecret.name` | Trust-root CA PEM used with the SPKI hash pin |
| `sidecar.mtlsBootstrapTokenSecret.name` | One-shot bootstrap token for workload certificate minting |
| `tokenizer.providerSecretName` | Tier 1 provider count_tokens API keys |
| `outputPredictor.pluginClientSvid.bindings[].secretName` | Optional override for per-tenant predictor-client SVID Secret names |

Database URL keys are rendered only as `valueFrom.secretKeyRef`. The validator
fails if a production values file or rendered manifest contains a literal
Postgres URL.

## SVID And Strategy C

If `outputPredictor.pluginEndpointDatabaseEnabled=true`, Strategy C plugin
delegation is active. Production values must then set:

```yaml
outputPredictor:
  pluginClientSvid:
    enabled: true
    bindings:
      - tenantId: "00000000-0000-4000-8000-000000000001"
        clientCertId: tenant-0001
```

Each binding renders a cert-manager `Certificate` with URI SAN:

```text
spiffe://spendguard.platform/predictor-client/<tenantId>
```

The plugin endpoint registry must use the matching `client_cert_id`, and the
plugin service validates that the presented client certificate subject matches
the tenant on every request.

## NetworkPolicy

The production example enables NetworkPolicy:

```yaml
networkPolicy:
  enabled: true
```

This renders default-deny egress for app pods labelled
`spendguard.io/enforced=true`, allows those pods to reach only the egress proxy
and DNS, and allows the egress proxy to reach provider CIDRs. Operators may use
`networkPolicy.externalProviderCidrs` and `networkPolicy.postgresCidrs` to
tighten egress to private-link or managed database ranges.

Setting `networkPolicy.enabled=false` in production requires
`networkPolicy.acknowledgeNoNetworkPolicy=true`; that posture is advisory L2,
not enforced L2.

## Security Context

All rendered workload containers must retain the SpendGuard container security
baseline:

- `runAsNonRoot: true`
- effective `runAsUser: 65532`
- `readOnlyRootFilesystem: true`
- `allowPrivilegeEscalation: false`
- `capabilities.drop: [ALL]`

The validator parses rendered Kubernetes manifests and fails if a workload loses
that baseline.

## Validation

Run:

```bash
scripts/release/validate-production-helm-values.sh
helm template spendguard charts/spendguard --set chart.profile=demo
helm template spendguard charts/spendguard -f charts/spendguard/values-production.example.yaml
```

The validation script includes negative checks for:

- plaintext database URLs in values
- Strategy C enabled without per-tenant SVID bindings
- rendered manifests missing the container security baseline

## Operator Fill-In Checklist

- Replace example image tags with the exact GA release images.
- Pre-create every referenced Secret in the release namespace.
- Replace placeholder bundle and trust-root hashes with byte-exact SHA-256 hex
  values from the release bundle.
- Set `networkPolicy.externalProviderCidrs` and `networkPolicy.postgresCidrs` to
  environment-specific ranges.
- Confirm cert-manager issuer names and SVID Secret names match the customer
  plugin endpoint registry.
