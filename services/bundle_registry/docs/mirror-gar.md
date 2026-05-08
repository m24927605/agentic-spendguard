# GAR mirror flow (GCP customers)

For GCP-hosted SpendGuard customers, GHCR pull is replaced by an
in-project Artifact Registry remote repository mirroring `ghcr.io/<owner>/spendguard/bundles/*`.

## Setup (per customer GCP project)

```hcl
resource "google_artifact_registry_repository" "spendguard_bundles" {
  location      = "us-west1"
  repository_id = "spendguard-bundles"
  format        = "DOCKER"
  mode          = "REMOTE_REPOSITORY"

  remote_repository_config {
    docker_repository {
      custom_repository {
        uri = "https://ghcr.io/<owner>/spendguard/bundles"
      }
    }
    upstream_credentials {
      username_password_credentials {
        username                = "spendguard-readonly"
        password_secret_version = google_secret_manager_secret_version.ghcr_pat.name
      }
    }
  }
}
```

## Sidecar config

```json
{
  "endpoint_url": "us-west1-docker.pkg.dev/<project>/spendguard-bundles",
  "registry_kind": "gar",
  "global_replicated": false,
  "namespaces": ["schema_bundle", "contract_bundle", "pricing_bundle"]
}
```

GKE Workload Identity gives sidecar pods the GAR `roles/artifactregistry.reader`
permission to pull.

## Verification trust

Same cosign verification as GHCR/ECR — signature identity is GitHub
Actions OIDC; GAR is just a mirror.
