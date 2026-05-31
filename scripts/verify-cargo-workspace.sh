#!/usr/bin/env bash
# Verify Cargo lock/metadata consistency and build the Rust surfaces touched by
# the predictor upgrade.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "${REPO_ROOT}"

log() { echo "[verify-cargo] $*" >&2; }

MANIFEST_LIST="$(mktemp -t spendguard-cargo-manifests.XXXXXX)"
find . \
    -path './.git' -prune -o \
    -path './.ait' -prune -o \
    -path '*/target' -prune -o \
    -name Cargo.toml -print \
    | sort >"${MANIFEST_LIST}"

MANIFEST_COUNT="$(wc -l <"${MANIFEST_LIST}" | tr -d ' ')"
if [ "${MANIFEST_COUNT}" -eq 0 ]; then
    log "FATAL: no Cargo.toml files found"
    exit 1
fi

log "checking metadata for ${MANIFEST_COUNT} manifests"
while IFS= read -r manifest; do
    dir="$(dirname "${manifest}")"
    if [ -f "${dir}/Cargo.lock" ]; then
        log "metadata --locked ${manifest}"
        cargo metadata --locked --manifest-path "${manifest}" --format-version 1 >/dev/null
    else
        log "metadata --no-deps ${manifest}"
        cargo metadata --no-deps --manifest-path "${manifest}" --format-version 1 >/dev/null
    fi
done <"${MANIFEST_LIST}"

BUILD_MANIFESTS=(
    "benchmarks/predictor-upgrade/Cargo.toml"
    "services/canonical_ingest/Cargo.toml"
    "services/control_plane/Cargo.toml"
    "services/egress_proxy/Cargo.toml"
    "services/ledger/Cargo.toml"
    "services/output_predictor/Cargo.toml"
    "services/run_cost_projector/Cargo.toml"
    "services/sidecar/Cargo.toml"
    "services/stats_aggregator/Cargo.toml"
    "services/tokenizer/Cargo.toml"
)

log "building ${#BUILD_MANIFESTS[@]} predictor-upgrade manifests"
for manifest in "${BUILD_MANIFESTS[@]}"; do
    log "cargo build --manifest-path ${manifest}"
    cargo build --manifest-path "${manifest}"
done

if ! git diff --quiet -- Cargo.lock '*/Cargo.lock'; then
    log "FATAL: Cargo.lock drift detected after verification"
    git diff -- Cargo.lock '*/Cargo.lock' >&2
    exit 1
fi

log "PASS"
