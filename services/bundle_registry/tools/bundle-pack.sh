#!/usr/bin/env bash
# bundle-pack.sh — local helper for building an OCI artifact from a bundle
# source directory. Mirrors what the GHA workflows do, for dev iteration.
#
# Usage:
#   bundle-pack.sh schema|contract|pricing <source-dir> <oci-ref> [--push]
#
# Push requires `oras login` already done.

set -euo pipefail

KIND="${1:?usage: bundle-pack.sh schema|contract|pricing <src> <ref> [--push]}"
SRC="${2:?source directory required}"
REF="${3:?oci ref required}"
PUSH_FLAG="${4:-}"

case "${KIND}" in
  schema)   ARTIFACT_TYPE="application/vnd.spendguard.schema-bundle.v1alpha1+tar" ;;
  contract) ARTIFACT_TYPE="application/vnd.spendguard.contract-bundle.v1alpha1+tar" ;;
  pricing)  ARTIFACT_TYPE="application/vnd.spendguard.pricing-bundle.v1alpha1+tar" ;;
  *) echo "unknown bundle kind: ${KIND}"; exit 1;;
esac

TMP=$(mktemp -d)
TGZ="${TMP}/bundle.tgz"
tar -czf "${TGZ}" -C "${SRC}" .

if [[ "${PUSH_FLAG}" == "--push" ]]; then
  oras push "${REF}" \
    --artifact-type "${ARTIFACT_TYPE}" \
    "${TGZ}:application/gzip"
  echo "Pushed: ${REF}"
else
  echo "Local pack only:"
  echo "  tarball: ${TGZ}"
  echo "  artifact-type: ${ARTIFACT_TYPE}"
  echo "  intended ref: ${REF}"
  echo "Run with --push to publish."
fi
