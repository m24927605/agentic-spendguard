#!/usr/bin/env bash
# Recompute sha256 hashes for the vendored encoder assets.
#
# Run this after:
#   * Bumping the `tiktoken-rs` dependency version (encoder bytes may change).
#   * Swapping a vendored encoder for a different bundle.
#
# The output is then copied into `crates/spendguard-tokenizer/src/lib.rs`
# under the `pub mod asset_sha256` block. The constants there are used
# by the `Tokenizer::new_with_embedded_assets()` constructor to enforce
# spec §7.4 signed-bundle integrity at boot.
#
# Usage:
#   bash crates/spendguard-tokenizer/data/recompute_sha256.sh

set -euo pipefail

cd "$(dirname "$0")"

assets=(
  "cl100k_base.tiktoken"
  "o200k_base.tiktoken"
  "p50k_base.tiktoken"
  "anthropic-claude3/tokenizer.json"
  "gemini-1.5/tokenizer.json"
  "cohere-command-r/tokenizer.json"
  "llama-3.1/tokenizer.json"
)

for asset in "${assets[@]}"; do
  if [[ -f "$asset" ]]; then
    hash="$(shasum -a 256 "$asset" | awk '{print $1}')"
    printf '%-42s %s\n' "$asset" "$hash"
  else
    printf 'WARN: asset %s not found in data/ directory\n' "$asset" >&2
  fi
done
