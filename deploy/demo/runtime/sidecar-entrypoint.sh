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
# services/sidecar/src/clients/mtls.rs::MTlsPaths::default). We symlink
# /etc/ssl/spendguard to that path so the sidecar's wait_for_workload_cert
# loop succeeds without code changes.
#
# Bundle hash + price snapshot hash are computed by the bundles-init
# container and exported in /var/lib/spendguard/bundles/runtime.env;
# we source that file so the sidecar config gets the live values
# instead of the placeholder strings in compose.yaml.
set -eu

# 1. Place mTLS material where the sidecar binary expects it.
mkdir -p /var/run/secrets/spendguard
ln -sf /etc/ssl/spendguard/sidecar.crt /var/run/secrets/spendguard/tls.crt
ln -sf /etc/ssl/spendguard/sidecar.key /var/run/secrets/spendguard/tls.key
ln -sf /etc/ssl/spendguard/ca.crt     /var/run/secrets/spendguard/ca.crt

# 2. Inject CA into OS trust store so reqwest's https_only catalog fetch
#    accepts the endpoint-catalog cert (signed by /etc/ssl/spendguard/ca.crt).
mkdir -p /usr/local/share/ca-certificates
cp /etc/ssl/spendguard/ca.crt /usr/local/share/ca-certificates/spendguard-demo-root.crt
update-ca-certificates 2>/dev/null

# 3. Source bundle runtime.env to populate
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

# 4. Inject the trust root + SPKI pin from the PKI volume.
if [ -f /etc/ssl/spendguard/ca.crt ] && [ -f /etc/ssl/spendguard/ca.spki.sha256.hex ]; then
    SPENDGUARD_SIDECAR_TRUST_ROOT_CA_PEM=$(cat /etc/ssl/spendguard/ca.crt)
    SPENDGUARD_SIDECAR_TRUST_ROOT_SPKI_SHA256_HEX=$(cat /etc/ssl/spendguard/ca.spki.sha256.hex)
    export SPENDGUARD_SIDECAR_TRUST_ROOT_CA_PEM
    export SPENDGUARD_SIDECAR_TRUST_ROOT_SPKI_SHA256_HEX
else
    echo "[sidecar-entrypoint] FATAL: PKI ca.crt or SPKI hash missing" >&2
    exit 1
fi

# 5. Wait for ledger + canonical-ingest gRPC listeners to be reachable.
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

# 6. Make sure the UDS parent directory exists + is writable.
mkdir -p /var/run/spendguard
chmod 0755 /var/run/spendguard

echo "[sidecar-entrypoint] launching spendguard-sidecar"
exec /usr/local/bin/spendguard-sidecar
