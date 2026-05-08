#!/usr/bin/env bash
# verify-bundle.sh — sample sidecar-side cosign verify flow.
#
# Used during onboarding rehearsals + ad-hoc ops triage to confirm a
# bundle's provenance matches the platform CI.
#
# Usage:
#   verify-bundle.sh <bundle-oci-ref>
# e.g.:
#   verify-bundle.sh ghcr.io/<owner>/spendguard/bundles/contract_bundle/<tenant>/<name>:v1.0.0

set -euo pipefail

BUNDLE_REF="${1:?usage: verify-bundle.sh <bundle-oci-ref>}"

# GitHub user/org names match `[A-Za-z0-9](-[A-Za-z0-9]|[A-Za-z0-9])*` —
# no underscores, no leading/trailing hyphens. Anchor to a specific publish
# workflow path on the main branch so the only accepted signing identities
# are the platform CI for this repo.
EXPECTED_IDENTITY_REGEX="${SPENDGUARD_PUBLISH_IDENTITY_REGEX:-^https://github.com/[A-Za-z0-9](-?[A-Za-z0-9])*/agentic-flow-cost-evaluation/\\.github/workflows/publish-(schema|contract|pricing)-bundle\\.yml@refs/heads/main$}"
EXPECTED_OIDC_ISSUER="${SPENDGUARD_PUBLISH_OIDC_ISSUER:-https://token.actions.githubusercontent.com}"

cosign verify "${BUNDLE_REF}" \
  --certificate-identity-regexp "${EXPECTED_IDENTITY_REGEX}" \
  --certificate-oidc-issuer "${EXPECTED_OIDC_ISSUER}"

echo "✅ ${BUNDLE_REF} verified."
