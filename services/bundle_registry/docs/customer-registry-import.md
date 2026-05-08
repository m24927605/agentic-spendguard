# Customer-managed registry import (Phase 2+)

For customers with strict registry-allowlists (or air-gapped sites), the
platform supports a `cosign copy` import flow that copies the bundle
artifact + signature + transparency log entry into a customer-controlled
registry.

## Flow

1. Customer ops (or the platform onboarding pipeline) authenticates to
   both GHCR (read) and the customer registry (write).
2. Run `cosign copy ghcr.io/<owner>/spendguard/bundles/.../<tag> <customer-ref>`.
3. Sidecars in the customer cluster pull from `<customer-ref>` and verify
   the cosign signature — verification still resolves to the original
   GitHub OIDC identity recorded in Rekor.

## Constraints

- Customer registry MUST support OCI 1.1 referrer api (cosign attaches
  signatures as referrers, not tags).
- Air-gapped sites need an explicit Rekor mirror or a `--insecure-skip-tlog-verify`
  ops exception (only acceptable when the customer has an offline
  attestation chain alternative).

## Phase

Deferred to Phase 2+. POC default = GHCR (with optional ECR / GAR
pull-through cache).
