#!/bin/sh
# Source bundle runtime metadata so the control-plane audit forwarder
# uses the same schema bundle hash canonical-seed registered.
set -eu

if [ -f /var/lib/spendguard/bundles/runtime.env ]; then
    set -a
    . /var/lib/spendguard/bundles/runtime.env
    set +a
fi

if [ -z "${SPENDGUARD_CONTROL_PLANE_AUDIT_SCHEMA_BUNDLE_HASH_HEX:-}" ]; then
    export SPENDGUARD_CONTROL_PLANE_AUDIT_SCHEMA_BUNDLE_HASH_HEX="${SPENDGUARD_SCHEMA_BUNDLE_HASH_HEX:-}"
fi

exec /usr/local/bin/spendguard-control-plane "$@"
