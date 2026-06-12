#!/bin/sh
set -eu

bundle="dist/index.js"
limit_bytes=51200

if [ ! -f "$bundle" ]; then
  echo "[size] missing $bundle; run package build first" >&2
  exit 1
fi

bytes=$(wc -c < "$bundle" | tr -d ' ')
if [ "$bytes" -gt "$limit_bytes" ]; then
  echo "[size] $bundle ${bytes} bytes exceeds ${limit_bytes} byte budget" >&2
  exit 1
fi

for token in \
  "node:crypto" \
  "@noble/hashes" \
  "createHash" \
  "blake2" \
  "failOpen" \
  "degradeOnUnavailable" \
  "SPENDGUARD_DISABLE"
do
  if grep -F "$token" "$bundle" >/dev/null; then
    echo "[size] forbidden token '$token' found in $bundle" >&2
    exit 1
  fi
done

echo "[size] openclaw-provider-plugin $bundle ${bytes} bytes <= ${limit_bytes}"
