# ECR mirror flow (AWS customers)

For AWS-hosted SpendGuard customers, GHCR pull is replaced by an
in-account ECR pull-through cache mirroring `ghcr.io/<owner>/spendguard/bundles/*`.
Sidecars verify cosign signatures against the same Helm-pinned trust root
regardless of the actual registry endpoint.

## Setup (per customer AWS account)

```hcl
resource "aws_ecr_pull_through_cache_rule" "spendguard_bundles" {
  ecr_repository_prefix = "spendguard-bundles"
  upstream_registry_url = "ghcr.io/<owner>/spendguard/bundles"
  credential_arn        = aws_secretsmanager_secret.ghcr_pat.arn
}

resource "aws_secretsmanager_secret" "ghcr_pat" {
  name = "spendguard/ghcr-readonly-pat"
}
```

The customer (or platform onboarding script) populates the secret with a
read-only GitHub PAT scoped to `read:packages`.

## Sidecar config

In the endpoint catalog `bundle_registry_endpoints` entry served to this
tenant, set:

```json
{
  "endpoint_url": "<account>.dkr.ecr.<region>.amazonaws.com/spendguard-bundles",
  "registry_kind": "ecr",
  "global_replicated": false,
  "namespaces": ["schema_bundle", "contract_bundle", "pricing_bundle"]
}
```

Sidecars use EKS Pod Identity (or IRSA) to authenticate the pull. The
`imagePullSecret` falls back to a kubelet credentials provider attached
to the cluster.

## Verification trust

cosign verify flow stays GHCR-centric:

```
cosign verify <ecr-mirror-ref> \
  --certificate-identity-regexp "^https://github.com/<owner>/agentic-flow-cost-evaluation/" \
  --certificate-oidc-issuer "https://token.actions.githubusercontent.com"
```

ECR is just a CDN; signing identity remains the GitHub Actions OIDC
ephemeral cert recorded in Rekor.
