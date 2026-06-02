#!/bin/bash
# Sidecar runtime entrypoint.
#
# bash (not /bin/sh) so the TCP-wait loop can use the `/dev/tcp/<host>/<port>`
# builtin without a netcat dependency.
#
# Compose mounts:
#   /etc/ssl/spendguard/      ← pki-data volume (ca.crt, sidecar.crt, sidecar.key,
#                               ca.spki.sha256.hex, ledger.crt, etc.)
#   /var/lib/spendguard/bundles ← bundles-data volume (contract + schema bundles
#                               plus runtime.env with computed bundle hash)
#   /etc/spendguard/          ← manifest-data volume (manifest_verify_key.pub.pem)
#   /var/run/spendguard/      ← sidecar-uds volume (UDS socket dir)
#
# The sidecar binary expects mTLS material at
# /var/run/secrets/spendguard/{tls.crt,tls.key,ca.crt} (per
# services/sidecar/src/clients/mtls.rs::MTlsPaths::default). The image
# pre-creates symlinks from /etc/ssl/spendguard to that path before switching
# to USER 65532; this entrypoint only verifies the mounted files are readable.
#
# Bundle hash + price snapshot hash are computed by the bundles-init
# container and exported in /var/lib/spendguard/bundles/runtime.env;
# we source that file so the sidecar config gets the live values
# instead of the placeholder strings in compose.yaml.
set -eu

# 1. Verify mTLS material where the sidecar binary expects it.
for path in \
    /var/run/secrets/spendguard/tls.crt \
    /var/run/secrets/spendguard/tls.key \
    /var/run/secrets/spendguard/ca.crt
do
    if [ ! -r "$path" ]; then
        echo "[sidecar-entrypoint] FATAL: unreadable mTLS material at $path" >&2
        exit 1
    fi
done

# 2. Source bundle runtime.env to populate
#    SPENDGUARD_SIDECAR_CONTRACT_BUNDLE_HASH_HEX with the actual
#    bundle sha256 (the bundles-init container wrote this).
if [ -f /var/lib/spendguard/bundles/runtime.env ]; then
    set -a
    . /var/lib/spendguard/bundles/runtime.env
    set +a
else
    echo "[sidecar-entrypoint] FATAL: bundles runtime.env missing" >&2
    exit 1
fi

# 3. Inject the trust root + SPKI pin.
#    Helm supplies these as env vars from Secret/values, while compose
#    mounts them under /etc/ssl/spendguard. Accept both boot paths.
if [ -n "${SPENDGUARD_SIDECAR_TRUST_ROOT_CA_PEM:-}" ] && \
   [ -n "${SPENDGUARD_SIDECAR_TRUST_ROOT_SPKI_SHA256_HEX:-}" ]; then
    export SPENDGUARD_SIDECAR_TRUST_ROOT_CA_PEM
    export SPENDGUARD_SIDECAR_TRUST_ROOT_SPKI_SHA256_HEX
elif [ -r /etc/ssl/spendguard/ca.crt ] && [ -r /etc/ssl/spendguard/ca.spki.sha256.hex ]; then
    SPENDGUARD_SIDECAR_TRUST_ROOT_CA_PEM=$(cat /etc/ssl/spendguard/ca.crt)
    SPENDGUARD_SIDECAR_TRUST_ROOT_SPKI_SHA256_HEX=$(cat /etc/ssl/spendguard/ca.spki.sha256.hex)
    export SPENDGUARD_SIDECAR_TRUST_ROOT_CA_PEM
    export SPENDGUARD_SIDECAR_TRUST_ROOT_SPKI_SHA256_HEX
elif [ -r /var/run/secrets/spendguard/ca.crt ] && \
     [ -n "${SPENDGUARD_SIDECAR_TRUST_ROOT_SPKI_SHA256_HEX:-}" ]; then
    SPENDGUARD_SIDECAR_TRUST_ROOT_CA_PEM=$(cat /var/run/secrets/spendguard/ca.crt)
    export SPENDGUARD_SIDECAR_TRUST_ROOT_CA_PEM
    export SPENDGUARD_SIDECAR_TRUST_ROOT_SPKI_SHA256_HEX
else
    echo "[sidecar-entrypoint] FATAL: trust root CA or SPKI hash missing" >&2
    exit 1
fi

# 4. Wait for ledger + canonical-ingest gRPC listeners to be reachable.
#    Sidecar's startup tries to open mTLS channels to both; if either
#    listener is still binding, sidecar exits. Compose `service_started`
#    only ensures the container is running, not that its socket is open.
#    Probe via /dev/tcp (bash builtin; no nc dependency).
#    Timeout: 60s — tonic + sqlx warm-up can be a few seconds.
wait_for_port() {
    host=$1; port=$2; deadline=$(( $(date +%s) + 60 ))
    while [ $(date +%s) -lt $deadline ]; do
        if (echo > "/dev/tcp/$host/$port") >/dev/null 2>&1; then
            echo "[sidecar-entrypoint] $host:$port reachable"
            return 0
        fi
        sleep 1
    done
    echo "[sidecar-entrypoint] FATAL: $host:$port not reachable after 60s" >&2
    return 1
}

wait_for_port ledger 50051 || exit 1
wait_for_port canonical-ingest 50052 || exit 1

# 5. Make sure the UDS parent directory is writable by USER 65532.
if [ ! -d /var/run/spendguard ] || [ ! -w /var/run/spendguard ]; then
    echo "[sidecar-entrypoint] FATAL: /var/run/spendguard is not writable" >&2
    exit 1
fi

echo "[sidecar-entrypoint] launching spendguard-sidecar"
exec /usr/local/bin/spendguard-sidecar
