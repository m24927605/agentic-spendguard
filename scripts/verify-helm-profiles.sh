#!/usr/bin/env bash
# Render the SpendGuard chart across supported profiles and security-relevant
# values combinations.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "${REPO_ROOT}"

OUT_DIR="${OUT_DIR:-/tmp/spendguard-helm-verify}"
mkdir -p "${OUT_DIR}"

log() { echo "[verify-helm] $*" >&2; }

render() {
    local name="$1"
    shift
    log "render ${name}"
    helm template spendguard charts/spendguard "$@" >"${OUT_DIR}/${name}.yaml"
}

render demo --set chart.profile=demo
render demo-networkpolicy --set chart.profile=demo --set networkPolicy.enabled=true
render production -f scripts/helm-validate-test-values.yaml
render production-networkpolicy -f scripts/helm-validate-test-values.yaml --set networkPolicy.enabled=true --set networkPolicy.acknowledgeNoNetworkPolicy=false
render production-kms -f scripts/helm-validate-test-values.yaml \
    --set signing.mode=kms \
    --set signing.kms.ledgerArn=arn:aws:kms:us-east-1:111122223333:key/ledger \
    --set signing.kms.sidecarArn=arn:aws:kms:us-east-1:111122223333:key/sidecar \
    --set signing.kms.webhookReceiverArn=arn:aws:kms:us-east-1:111122223333:key/webhook \
    --set signing.kms.ttlSweeperArn=arn:aws:kms:us-east-1:111122223333:key/ttl \
    --set signing.kms.controlPlaneArn=arn:aws:kms:us-east-1:111122223333:key/control-plane

if grep -R "postgres://[^[:space:]\"']*" "${OUT_DIR}" >/dev/null; then
    log "FATAL: rendered manifests contain plaintext postgres URL"
    grep -R "postgres://[^[:space:]\"']*" "${OUT_DIR}" >&2
    exit 1
fi

if grep -n "spendguard-signing-keys" "${OUT_DIR}/production-kms.yaml" >/dev/null; then
    log "FATAL: KMS render still references local signing Secret"
    grep -n "spendguard-signing-keys" "${OUT_DIR}/production-kms.yaml" >&2
    exit 1
fi

for required in "runAsUser: 65532" "readOnlyRootFilesystem: true" "drop:" "ALL"; do
    if ! grep -R "${required}" "${OUT_DIR}" >/dev/null; then
        log "FATAL: rendered manifests missing security baseline token: ${required}"
        exit 1
    fi
done

log "PASS outputs=${OUT_DIR}"
