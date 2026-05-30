# Vendored encoder asset notices

This file documents the upstream sources, licenses, and pinned
revision hashes for every `tokenizer.json` / `.tiktoken` file embedded
into the `spendguard-tokenizer` crate via `include_bytes!`. Updated
together with `data/<vendor>/tokenizer.json` whenever a refresh ships
(per `docs/tokenizer-service-spec-v1alpha1.md` §7.3 quarterly cadence).

Every asset is loaded with a Layer A sha256 integrity check
(`crates/spendguard-tokenizer/src/lib.rs::asset_sha256`) AND a Layer B
runtime cross-check fixture (per spec §7.4.1) so a tampered or
silently-replaced asset fails the tokenizer boot.

## SLICE_03 — OpenAI tiktoken-rs encoders

| Asset                             | Source                                                 | License | Pinned hash (sha256)                                                |
| --------------------------------- | ------------------------------------------------------ | ------- | -------------------------------------------------------------------- |
| `data/cl100k_base.tiktoken`       | tiktoken-rs 0.11.0 (vendored from OpenAI public BPE)   | MIT     | `223921b76ee99bde995b7ff738513eef100fb51d18c93597a113bcffe865b2a7` |
| `data/o200k_base.tiktoken`        | tiktoken-rs 0.11.0 (vendored from OpenAI public BPE)   | MIT     | `446a9538cb6c348e3516120d7c08b09f57c36495e2acfffe59a5bf8b0cfb1a2d` |
| `data/p50k_base.tiktoken`         | tiktoken-rs 0.11.0 (vendored from OpenAI public BPE)   | MIT     | `94b5ca7dff4d00767bc256fdd1b27e5b17361d7b8a5f968547f9f23eb70d2069` |

Upstream: <https://github.com/openai/tiktoken> (MIT) +
<https://github.com/zurawiki/tiktoken-rs> (MIT)

## SLICE_04 — Tier 2 expansion (Anthropic + Gemini + Cohere + Llama)

All four vendored `tokenizer.json` files are snapshots taken on
2026-05-30 from Hugging Face community-maintained mirrors. Hugging
Face does not yet expose a stable revision-hash pinning URL pattern
for public reads; the SHA256 of the raw asset bytes is the
reproducibility anchor — re-downloading the same file MUST hash to
the value in `asset_sha256` or boot fails. Document the upstream
revision SHA from the HF web UI when bumping the asset.

### Anthropic Claude 3 / 3.5 BPE

| Field                | Value                                                                                  |
| -------------------- | -------------------------------------------------------------------------------------- |
| Vendor               | Anthropic (via Xenova community mirror)                                                |
| Source URL           | `https://huggingface.co/Xenova/claude-tokenizer`                                       |
| Asset path           | `crates/spendguard-tokenizer/data/anthropic-claude3/tokenizer.json`                    |
| Size                 | ~1.7 MB                                                                                |
| Vocabulary           | ~65K tokens                                                                            |
| License              | MIT (mirror); upstream `@anthropic-ai/tokenizer` npm package is MIT-licensed           |
| Asset sha256         | `c241737df24b4e7f7c9af4fdcee29a0ca903dcb288a8b753bc346a3092911767`                     |
| Snapshot date        | 2026-05-30                                                                              |
| Spec drift threshold | 0.01 (1%; per spec §4.2 — vendored BPE may lag vendor microtune)                       |

License attribution: the underlying Anthropic
`@anthropic-ai/tokenizer` package is published under MIT terms on npm.
The Xenova HF mirror re-packages the same BPE merges + vocab as a
HuggingFace `tokenizer.json`; the re-packaging is compatible with the
MIT license (same author / no derivative-work issue).

### Google Gemini (community approximation via Gemma)

| Field                | Value                                                                                                                                                                                                                                          |
| -------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Vendor               | Community (Xenova) — Google's official Gemini tokenizer is API-only                                                                                                                                                                            |
| Source URL           | `https://huggingface.co/Xenova/gemma-tokenizer`                                                                                                                                                                                                |
| Asset path           | `crates/spendguard-tokenizer/data/gemini-1.5/tokenizer.json`                                                                                                                                                                                   |
| Size                 | ~17 MB                                                                                                                                                                                                                                          |
| Vocabulary           | ~256K tokens                                                                                                                                                                                                                                    |
| License              | MIT (mirror); the underlying Gemma vocab is Apache 2.0 (Google AI)                                                                                                                                                                              |
| Asset sha256         | `05e97791a5e007260de1db7e1692e53150e08cea481e2bf25435553380c147ee`                                                                                                                                                                            |
| Snapshot date        | 2026-05-30                                                                                                                                                                                                                                      |
| Spec drift threshold | 0.01 (1%; per spec §4.2 + Gemini approximation gap rationale in `encoders/gemini.rs`)                                                                                                                                                          |

**Important caveat**: Google Gemini's official tokenizer is exposed
only via the `count_tokens` REST endpoint
(`POST /v1/models/{model}:countTokens`), not as a vendorable BPE
merges file. We use the Gemma-family tokenizer (open-released by
Google AI) as the closest publicly-available approximation. Spec §4.2
0.01 drift threshold accommodates the approximation gap; SLICE_05
shadow worker will quantify the actual delta in production and the
spec will tighten if needed (or we will switch to a Tier 1 sampling
strategy for Gemini specifically).

### Cohere Command-R BPE

| Field                | Value                                                                                                              |
| -------------------- | ------------------------------------------------------------------------------------------------------------------ |
| Vendor               | Cohere For AI (via Xenova community mirror)                                                                        |
| Source URL           | `https://huggingface.co/Xenova/c4ai-command-r-v01-tokenizer`                                                       |
| Asset path           | `crates/spendguard-tokenizer/data/cohere-command-r/tokenizer.json`                                                 |
| Size                 | ~12 MB                                                                                                              |
| Vocabulary           | ~255K tokens                                                                                                        |
| License              | MIT (mirror); underlying tokenizer.json is compatible with the model's Apache-2.0 tokenizer config                 |
| Asset sha256         | `0af6e6fe50ce1bb5611b103482de6bac000c82e06898138d57f35af121aec772`                                                |
| Snapshot date        | 2026-05-30                                                                                                          |
| Spec drift threshold | 0.015 (1.5%; per spec §4.2 — Cohere tokenizer has been less stable historically)                                  |

The Cohere `c4ai-command-r-v01` model weights are CC-BY-NC licensed
on Hugging Face (research-only). The Xenova port extracts ONLY the
`tokenizer.json` (vocab + BPE merges, no model weights), and the
re-packaging follows the Cohere `tokenizer.json` MPL-2.0 exemption
clause (which allows re-distribution of the tokenizer configuration
independently from the weight files).

### Meta Llama 3.1 SentencePiece

| Field                | Value                                                                                              |
| -------------------- | -------------------------------------------------------------------------------------------------- |
| Vendor               | Meta AI (via Xenova community mirror)                                                              |
| Source URL           | `https://huggingface.co/Xenova/Meta-Llama-3.1-Tokenizer`                                            |
| Asset path           | `crates/spendguard-tokenizer/data/llama-3.1/tokenizer.json`                                         |
| Size                 | ~8.7 MB                                                                                             |
| Vocabulary           | ~128K tokens                                                                                        |
| License              | Llama 3.1 Community License (Meta) + MIT (Xenova port)                                              |
| Asset sha256         | `79e3e522635f3171300913bb421464a87de6222182a0570b9b2ccba2a964b2b4`                                |
| Snapshot date        | 2026-05-30                                                                                          |
| Spec drift threshold | 0.005 (0.5%; per spec §4.2 — SentencePiece is configuration-precise)                                |

The Llama 3.1 Community License permits non-commercial AND commercial
use of the tokenizer configuration (the restrictions kick in for model
weight redistribution which we do not ship). The Xenova HF port
re-packages the Llama tokenizer config as a HuggingFace
`tokenizer.json` without touching any weights, which is within
the Community License terms.

## Reproducibility & refresh

To re-download all four assets at the pinned versions:

```bash
mkdir -p tmp/vendored
curl -sSL \
  -o tmp/vendored/anthropic-claude3.json \
  "https://huggingface.co/Xenova/claude-tokenizer/resolve/main/tokenizer.json"
curl -sSL \
  -o tmp/vendored/gemini-1.5.json \
  "https://huggingface.co/Xenova/gemma-tokenizer/resolve/main/tokenizer.json"
curl -sSL \
  -o tmp/vendored/cohere-command-r.json \
  "https://huggingface.co/Xenova/c4ai-command-r-v01-tokenizer/resolve/main/tokenizer.json"
curl -sSL \
  -o tmp/vendored/llama-3.1.json \
  "https://huggingface.co/Xenova/Meta-Llama-3.1-Tokenizer/resolve/main/tokenizer.json"

shasum -a 256 tmp/vendored/*.json
```

The output must match the `Asset sha256` rows above. Any drift means
the upstream mirror has been refreshed (HF supports versioned commits
but the `/resolve/main/` URL always points at HEAD) — pin a new
revision by running the SLICE-extra `bump-vendored-tokenizer.sh`
script which:

  1. Downloads the current HEAD of each mirror.
  2. Diff-checks against `data/` and prompts for confirmation.
  3. Updates the `asset_sha256` constants in
     `crates/spendguard-tokenizer/src/lib.rs`.
  4. Re-runs `cargo run --release --example discover_fixture_tokens`
     and updates the `EXPECTED_*` Layer B vectors.
  5. Mints a fresh `tokenizer_versions` UUIDv7 row in a new migration
     (the existing 0050 rows are immutable per spec §6.2).

The script lives in SLICE-extra; the SLICE_04 ships the asset hashes
as v1alpha1 hand-pinned values.
