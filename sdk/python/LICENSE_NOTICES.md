# Vendored encoder asset notices — Python SDK

This file documents the upstream sources, licenses, and pinned
sha256 hashes for every vendored `tokenizer.json` shipped inside the
`spendguard-sdk` wheel. Mirrors the Rust crate notices at
`crates/spendguard-tokenizer/LICENSE_NOTICES.md`; both SDKs MUST
ship byte-identical assets so SDK-side and server-side token counts
agree.

Every asset is verified at first use with a sha256 integrity check
in the corresponding estimator module
(`spendguard/estimators/{anthropic,gemini}.py::_load_tokenizer`).
Mismatch ⇒ raise at estimator construction (fail-fast; an asset swap
is a wire-protocol violation per `tokenizer-service-spec-v1alpha1.md`
§7.4.1).

## SLICE_12 — Tier 2 Python SDK vendored assets

### Anthropic Claude 3 / 3.5 BPE

| Field                | Value                                                                                  |
| -------------------- | -------------------------------------------------------------------------------------- |
| Vendor               | Anthropic (via Xenova community mirror)                                                |
| Source URL           | `https://huggingface.co/Xenova/claude-tokenizer`                                       |
| Asset path           | `src/spendguard/data/anthropic_claude3_tokenizer.json`                                 |
| Size                 | ~1.7 MB                                                                                |
| Vocabulary           | ~65K tokens                                                                            |
| License              | MIT (mirror); upstream `@anthropic-ai/tokenizer` npm package is MIT-licensed           |
| Asset sha256         | `c241737df24b4e7f7c9af4fdcee29a0ca903dcb288a8b753bc346a3092911767`                     |
| Snapshot date        | 2026-05-30                                                                              |
| Spec drift threshold | 0.01 (1%; per `tokenizer-service-spec-v1alpha1.md` §4.2)                               |

License attribution: the underlying Anthropic
`@anthropic-ai/tokenizer` package is published under MIT terms on npm.
The Xenova HF mirror re-packages the same BPE merges + vocab as a
HuggingFace `tokenizer.json`; the re-packaging is compatible with the
MIT license (same author / no derivative-work issue).

### Google Gemini (community approximation via Gemma)

| Field                | Value                                                                                                                                       |
| -------------------- | ------------------------------------------------------------------------------------------------------------------------------------------- |
| Vendor               | Community (Xenova) — Google's official Gemini tokenizer is API-only                                                                         |
| Source URL           | `https://huggingface.co/Xenova/gemma-tokenizer`                                                                                             |
| Asset path           | `src/spendguard/data/gemini_1_5_tokenizer.json`                                                                                             |
| Size                 | ~17 MB                                                                                                                                       |
| Vocabulary           | ~256K tokens                                                                                                                                 |
| License              | MIT (mirror); the underlying Gemma vocab is Apache 2.0 (Google AI)                                                                          |
| Asset sha256         | `05e97791a5e007260de1db7e1692e53150e08cea481e2bf25435553380c147ee`                                                                          |
| Snapshot date        | 2026-05-30                                                                                                                                   |
| Spec drift threshold | 0.01 (1%; SLICE_04 R2 M5 honest disclosure — community approximation gap absorbed by drift threshold)                                       |

**Important caveat**: Google Gemini's official tokenizer is exposed
only via the `count_tokens` REST endpoint
(`POST /v1/models/{model}:countTokens`), not as a vendorable BPE
merges file. We use the Gemma-family tokenizer (open-released by
Google AI) as the closest publicly-available approximation. Spec §4.2
1% drift threshold accommodates the approximation gap; SLICE_05
shadow worker on the server side will quantify the actual delta in
production and the spec will tighten if needed.

## Reproducibility & sha256 verification

To re-download the assets at the pinned versions:

```bash
mkdir -p tmp/vendored
curl -sSL \
  -o tmp/vendored/anthropic-claude3.json \
  "https://huggingface.co/Xenova/claude-tokenizer/resolve/main/tokenizer.json"
curl -sSL \
  -o tmp/vendored/gemini-1.5.json \
  "https://huggingface.co/Xenova/gemma-tokenizer/resolve/main/tokenizer.json"

shasum -a 256 tmp/vendored/*.json
```

The output MUST match the `Asset sha256` rows above. The Python SDK
and the Rust crate share these assets — if either drifts, the
server-side audit row and SDK-side estimated row will disagree by
the divergence delta.

The SDK estimator modules verify sha256 at first call and raise on
mismatch; this is the SDK-side equivalent of the Rust crate's Layer
A asset integrity check per spec §7.4.1.

## Out-of-scope vendored assets (server-side only)

The Rust crate also vendors Llama 3.1 SentencePiece and (optionally,
feature-gated) Cohere Command-R BPE assets. The Python SDK
intentionally does NOT ship these:

* **Llama** — Bedrock-only model family; Python SDK callers route via
  egress_proxy + the server-side tokenizer service. Including the
  ~8.7 MB asset in the wheel would slow `pip install` for users who
  don't need it.
* **Cohere** — server-side Rust crate ships behind `--features cohere`
  pending legal review of the Cohere tokenizer asset's redistribution
  terms (R2 M6 + Security F5). The Python SDK omits Cohere entirely
  for SLICE_12; future SLICE_NN may add a separate `cohere` extra
  after legal review.

For both families, the SDK estimator dispatch falls back to chars/4
with a `warnings.warn` so the operator sees "model recognised but no
SDK estimator" rather than a silent mis-route.
