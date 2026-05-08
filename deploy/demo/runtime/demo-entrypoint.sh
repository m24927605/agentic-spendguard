#!/bin/sh
# Demo runtime entrypoint.
#
# Sources the bundle runtime.env (which the bundles-init container wrote
# at compose-up time, with the actual sha256 of the bundle bytes)
# so that SPENDGUARD_PRICE_SNAPSHOT_HASH_HEX matches the contract bundle
# the sidecar loaded.
set -eu

if [ -f /var/lib/spendguard/bundles/runtime.env ]; then
    set -a
    . /var/lib/spendguard/bundles/runtime.env
    set +a
else
    echo "[demo-entrypoint] FATAL: bundles runtime.env not found" >&2
    exit 1
fi

exec python /usr/local/bin/run_demo.py
