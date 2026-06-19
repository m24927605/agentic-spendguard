#!/usr/bin/env bash
# Verify Cargo lock/metadata consistency and build the Rust surfaces touched by
# the predictor upgrade.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "${REPO_ROOT}"

log() { echo "[verify-cargo] $*" >&2; }

if [ "${SPENDGUARD_VERIFY_CARGO_IN_ARCHIVE:-0}" != "1" ]; then
    WORK_DIR="$(mktemp -d -t spendguard-cargo-clean.XXXXXX)"
    rm -rf "${WORK_DIR}"
    cleanup_worktree() {
        git worktree remove --force "${WORK_DIR}" >/dev/null 2>&1 || rm -rf "${WORK_DIR}"
    }
    trap cleanup_worktree EXIT
    log "creating clean detached worktree at ${WORK_DIR}"
    git worktree add --detach "${WORK_DIR}" HEAD >/dev/null
    log "re-running inside clean worktree so ignored per-crate Cargo.lock files cannot mask local drift"
    env SPENDGUARD_VERIFY_CARGO_IN_ARCHIVE=1 "${WORK_DIR}/scripts/verify-cargo-workspace.sh"
    exit $?
fi

metadata_locked() {
    local manifest="$1"
    local lock_file
    lock_file="$(dirname "${manifest}")/Cargo.lock"
    if cargo metadata --locked --manifest-path "${manifest}" --format-version 1 >/dev/null; then
        return 0
    fi

    log "metadata --locked failed for ${manifest}; checking whether Cargo would mutate ${lock_file}"
    cargo metadata --manifest-path "${manifest}" --format-version 1 >/dev/null
    if ! git diff --quiet -- "${lock_file}"; then
        log "FATAL: ${lock_file} changed after unlocked metadata; commit or revert intentional lock drift"
        git diff -- "${lock_file}" >&2
        exit 1
    fi

    cargo metadata --locked --manifest-path "${manifest}" --format-version 1 >/dev/null
}

MANIFEST_LIST="$(mktemp -t spendguard-cargo-manifests.XXXXXX)"
find . \
    -path './.git' -prune -o \
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
        metadata_locked "${manifest}"
    else
        log "metadata --no-deps ${manifest}"
        cargo metadata --no-deps --manifest-path "${manifest}" --format-version 1 >/dev/null
    fi
done <"${MANIFEST_LIST}"
log "manifests without committed Cargo.lock are built from a clean worktree; generated ignored per-crate lockfiles are not accepted as committed state"

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

TEST_COMMANDS=(
    "cargo test --manifest-path services/canonical_ingest/Cargo.toml append_events_rejects -- --nocapture"
    "cargo test --manifest-path services/control_plane/Cargo.toml audit_forwarder -- --nocapture"
    "cargo test --manifest-path services/egress_proxy/Cargo.toml multi_provider_demo_routing_works_for_all_five -- --nocapture"
    "cargo test --manifest-path services/output_predictor/Cargo.toml breaker_open_skips_predict_without_recording_extra_failure -- --nocapture"
    "cargo test --manifest-path services/run_cost_projector/Cargo.toml"
    "cargo test --manifest-path services/stats_aggregator/Cargo.toml"
    "cargo test --manifest-path services/tokenizer/Cargo.toml append_request_carries_required_observability_envelope -- --nocapture"
)

log "running ${#TEST_COMMANDS[@]} affected test commands"
for cmd in "${TEST_COMMANDS[@]}"; do
    log "${cmd}"
    bash -lc "${cmd}"
done

if ! git diff --quiet -- Cargo.lock '*/Cargo.lock'; then
    log "FATAL: Cargo.lock drift detected after verification"
    git diff -- Cargo.lock '*/Cargo.lock' >&2
    exit 1
fi

log "PASS"
