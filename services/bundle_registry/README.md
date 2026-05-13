# spendguard-bundle-registry

OCI-based bundle distribution for the Agentic SpendGuard Phase 1 first-customer
(K8s SaaS-managed) POC.

This is **not** a custom service binary. It is a *distribution pipeline*
built on existing OCI registry + Sigstore cosign infrastructure. The
SpendGuard control plane signs bundles in CI and publishes them to a
registry; sidecars pull + verify cosign signatures against a Helm-pinned
trust root.

## Spec map

- Stage 2 §5 (Bundle Registry: 1 service, 3 namespaces)
- Stage 2 §5.3 (multi-path distribution: GHCR / ECR mirror / GAR mirror /
  customer registry import)
- Trace §12 (schema bundle distribution)
- Contract §17 (contract bundle signing)
- Ledger §13 (pricing bundle 三層 freeze)

## Three namespaces

| OCI repo path | Bundle kind | Spec |
|---|---|---|
| `<registry>/spendguard/bundles/schema_bundle/<id>:<tag>` | canonical schema + mapping profiles | Trace §12 |
| `<registry>/spendguard/bundles/contract_bundle/<tenant>/<name>:<tag>` | Contract DSL bundle + model capability matrix + frozen pricing snapshot | Contract §17 + Ledger §13 |
| `<registry>/spendguard/bundles/pricing_bundle/<tag>` | pricing_versions + fx_rate_versions + unit_conversion_versions | Ledger §13 |

## POC default: GHCR + cosign keyless

POC publishes via GitHub Actions OIDC keyless cosign signing. No private
keys live in the platform repo; signing identities are GitHub Actions
ephemeral OIDC tokens, recorded in the public Rekor transparency log.

```
workflows/                          # SOURCE templates kept here
├── publish-schema-bundle.yml       schema_bundle namespace publisher
├── publish-contract-bundle.yml     contract_bundle (per tenant)
└── publish-pricing-bundle.yml      pricing_bundle (global)
```

**Adoption step (operator)**: GitHub Actions only runs workflows under
the repo's `.github/workflows/` directory. When adopting these templates:

```bash
# From repo root:
mkdir -p .github/workflows
cp services/bundle_registry/workflows/*.yml .github/workflows/
git add .github/workflows/
```

The templates assume bundle source directories at the repo root: create
`bundles/schema/`, `bundles/pricing/<version>/`, and `contracts/<name>/`
before triggering the workflows. The fetch-pricing-snapshot helper is
at `services/bundle_registry/tools/fetch-pricing-snapshot.sh`.

## Multi-path distribution (Stage 2 §5.3)

| Path | Use case | Phase |
|---|---|---|
| **GHCR (POC default)** | first design partner; customer accepts GitHub PAT pull | Phase 1 |
| **ECR mirror** | AWS customers — pull-through cache from GHCR to per-customer ECR | Phase 1 if AWS |
| **GAR mirror** | GCP customers — pull-through cache to per-customer GAR | Phase 1 if GCP |
| **Customer registry import** | strict customer or air-gapped — `cosign copy` to customer registry | Phase 2+ |

See `docs/mirror-ecr.md` and `docs/mirror-gar.md` for per-cloud setup.

## Verification flow (sidecar side)

At sidecar startup + on bundle pull:

1. Sidecar pulls `<bundle-ref>@sha256:<digest>` from the registry endpoint
   announced in the catalog (per Sidecar §8 `bundle_registry_endpoints`).
2. Sidecar runs `cosign verify --certificate-identity-regexp ... --certificate-oidc-issuer ...`
   against the Helm-pinned trust root.
3. On success, the bundle digest + cosign verification metadata are
   cached locally with TTL = 1h.
4. On verification failure, sidecar emits `audit_event` (bundle signature
   failure) and refuses to load the bundle (fail_closed for enforcement
   routes).

Sidecars NEVER trust unsigned bundles.

## Files in this directory

```
workflows/
├── publish-schema-bundle.yml       publishes a schema_bundle artifact
├── publish-contract-bundle.yml     publishes a contract_bundle artifact
└── publish-pricing-bundle.yml      publishes a pricing_bundle artifact
tools/
├── bundle-pack.sh                  local: tar bundle source dir into OCI artifact
└── verify-bundle.sh                local: sample sidecar-side cosign verify
docs/
├── mirror-ecr.md                   AWS ECR mirror setup
├── mirror-gar.md                   GCP GAR mirror setup
└── customer-registry-import.md     customer-managed registry flow (Phase 2+)
examples/
├── schema_bundle/v1alpha1/         sample schema bundle source
├── contract_bundle/quickstart/     sample contract bundle source
└── pricing_bundle/focus_v1_2/      sample pricing bundle source
```

See per-file headers for setup details.
