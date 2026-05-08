#!/bin/sh
# Generate the demo PKI: a local root CA + per-service workload certs.
#
# Outputs (under /pki, mounted from a named volume):
#   ca.crt                                      — root CA cert (PEM)
#   ca.key                                      — root CA private key
#   ca.spki.sha256.hex                          — SPKI fingerprint (sidecar pin)
#   <service>.crt, <service>.key                — per-service workload cert
#                                                 (CN/SAN matches docker DNS name)
#
# Services: ledger, canonical_ingest, sidecar, endpoint_catalog
#
# All certs sign the same root, so each service trusts the others'
# certs via the shared `ca.crt`. mTLS server (ledger / canonical_ingest)
# pins this root for client cert validation; sidecar pins via SPKI hash
# in its trust_root_spki_sha256_hex config.
#
# Idempotent: re-running with existing /pki/ca.crt skips generation.

set -eu

OUT=/pki
mkdir -p "$OUT"

# Per-cert idempotency (Codex webhook r2 P2.2):
#   Top-level skip used to be all-or-nothing. New certs added later
#   (e.g. webhook_receiver) would never be minted on existing volumes.
#   Now CA + each workload cert is generated independently if missing.

# 1. Root CA --------------------------------------------------------------
if [ -f "$OUT/ca.crt" ] && [ -f "$OUT/ca.key" ] && [ -f "$OUT/ca.spki.sha256.hex" ]; then
    echo "[pki] existing CA detected, skipping CA generation"
else
echo "[pki] minting root CA..."
openssl genrsa -out "$OUT/ca.key" 4096 2>/dev/null
openssl req -x509 -new -nodes \
    -key "$OUT/ca.key" \
    -sha256 -days 365 \
    -subj "/CN=spendguard-demo-root" \
    -out "$OUT/ca.crt"

# DER cert fingerprint (sha256 of the entire DER-encoded leaf cert).
# The sidecar's trust::verify_root_ca_pin (services/sidecar/src/
# bootstrap/trust.rs) hashes the FIRST CERT'S DER bytes — NOT the
# SubjectPublicKeyInfo — so we must compute the same way here. (Codex
# Round 1 caught the prior SPKI-based hash as a boot blocker.)
openssl x509 -in "$OUT/ca.crt" -outform DER \
  | openssl dgst -sha256 -binary \
  | xxd -p -c 256 \
  | tr -d '\n' > "$OUT/ca.spki.sha256.hex"

echo "[pki] CA DER cert sha256 (sidecar trust pin): $(cat $OUT/ca.spki.sha256.hex)"
fi

# 2. Per-service workload certs ------------------------------------------
# Per-cert idempotent: skip individual cert if already minted. This lets
# new services (e.g. webhook_receiver) get certs on existing pki-data
# volumes without rotating the others.
for svc in ledger canonical_ingest sidecar endpoint_catalog webhook_receiver ttl_sweeper; do
    if [ -f "$OUT/$svc.crt" ] && [ -f "$OUT/$svc.key" ]; then
        echo "[pki] $svc cert already exists, skipping"
        continue
    fi
    echo "[pki] minting $svc workload cert..."
    openssl genrsa -out "$OUT/$svc.key" 2048 2>/dev/null

    # Service DNS aliases used in compose: 'ledger', 'canonical-ingest',
    # 'sidecar', 'endpoint-catalog'. Need both underscore (config var) and
    # dash (DNS) forms in the SAN list so SNI matches whichever the
    # client uses. For sidecar the cert is used as a CLIENT cert (mTLS
    # client into ledger / canonical_ingest); SAN doesn't gate that, but
    # the CN identifies the workload.
    case "$svc" in
        canonical_ingest) dns_alias="canonical-ingest" ;;
        endpoint_catalog) dns_alias="endpoint-catalog" ;;
        webhook_receiver) dns_alias="webhook-receiver" ;;
        ttl_sweeper)      dns_alias="ttl-sweeper" ;;
        *)                dns_alias="$svc" ;;
    esac

    cat > /tmp/${svc}.cnf <<EOF
[req]
distinguished_name = req_dn
req_extensions     = v3_ext
prompt             = no

[req_dn]
CN = $dns_alias.spendguard.internal

[v3_ext]
subjectAltName = @alt_names
keyUsage = critical, digitalSignature, keyEncipherment
extendedKeyUsage = serverAuth, clientAuth

[alt_names]
# Both the underscore form (Rust env-var convention) and the dash form
# (docker compose alias / sidecar default_sni). Sidecar's canonical
# default_sni uses dash + ".spendguard.internal", so DNS.1 carries that
# form. Add the underscore form too for any caller using the env-var
# convention. localhost is for in-container probes.
DNS.1 = $dns_alias.spendguard.internal
DNS.2 = $svc.spendguard.internal
DNS.3 = $svc
DNS.4 = $dns_alias
DNS.5 = localhost
EOF

    openssl req -new \
        -key "$OUT/$svc.key" \
        -out "/tmp/$svc.csr" \
        -config "/tmp/$svc.cnf"

    openssl x509 -req \
        -in "/tmp/$svc.csr" \
        -CA "$OUT/ca.crt" \
        -CAkey "$OUT/ca.key" \
        -CAcreateserial \
        -days 365 \
        -sha256 \
        -extensions v3_ext \
        -extfile "/tmp/$svc.cnf" \
        -out "$OUT/$svc.crt" 2>/dev/null

    rm -f "/tmp/$svc.csr" "/tmp/$svc.cnf"
done

# 3. Tighten file permissions --------------------------------------------
# (Idempotent re-runs are fine: chmod always sets the same target perms.)
chmod 0644 "$OUT"/*.crt "$OUT/ca.spki.sha256.hex"
chmod 0640 "$OUT"/*.key

echo "[pki] generated artifacts:"
ls -la "$OUT"
