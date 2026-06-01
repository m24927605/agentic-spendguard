# Tokenizer Service Specification — v1alpha1 (DRAFT)

> 📝 **Status: DRAFT** (writing in design phase on branch `design/predictor-upgrade`)
> **DRAFT → LOCKED criteria**: locks together with the predictor-upgrade spec set per `predictor-architecture-spec-v1alpha1.md` §0.2; additionally requires (a) Tier 2 hot-path p99 < 1ms on commodity hardware (verified per `§10.1` benchmark), (b) Tier 1 shadow with 1% sampling rate detecting < 1% drift across all 4 supported tokenizer kinds for 30 consecutive days, (c) Tier 3 fallback rate < 0.1% in production (per `predictor-architecture-spec-v1alpha1.md` §0.2 health invariant), and (d) circuit breaker recovery chaos test green.
> **Companion specs (this set)**: `predictor-architecture-spec-v1alpha1.md` (umbrella), `audit-chain-prediction-extension-v1alpha1.md` (defines `tokenizer_tier` + `tokenizer_version_id` audit columns), `output-predictor-service-spec-v1alpha1.md` (consumer of tokenize output), `stats-aggregator-spec-v1alpha1.md` (consumes drift_alert events).
> **Pre-existing LOCKED dependencies**: `sidecar-architecture-spec-v1alpha1.md` (§3 capability_required for in-process vs gRPC tokenize), `trace-schema-spec-v1alpha1.md` (§3 identity for tokenize span), `agent-runtime-spend-guardrails-complete.md` (v1.3 strategy 三支柱).
> **Compatibility policy**: alpha — proto3 additive evolution; vendored encoder versioning via `tokenizer_versions` registry table; multi-encoder rolling refresh allowed without service restart (per §7.3).

---

## §0. Lock status & prerequisites

### 0.1 範圍

本 spec 是 **tokenizer service 完整設計**：定義 Tier 1（provider count_tokens API async shadow）、Tier 2（local exact BPE，hot path）、Tier 3（heuristic fallback）三層架構，per-provider dispatch table，drift detection，circuit breaker，`tokenizer_versions` registry table schema，vendored encoder maintenance flow。

**不在本 spec 範圍**：

- 預測值如何被 `output_predictor` 消費（推給 `output-predictor-service-spec-v1alpha1.md`）
- 預測值如何進 audit chain（推給 `audit-chain-prediction-extension-v1alpha1.md`）
- Per-provider routing 在 egress_proxy 的 wire 行為（推給 SLICE 11 multi-provider routing 與後續 `egress-proxy-multi-provider-spec.md`）

### 0.2 DRAFT → LOCKED criteria

進入 LOCKED 之前下列 6 項必達成：

1. SLICE 03 + SLICE 04 + SLICE 05 三 slices 全 merged，Tier 2 涵蓋 OpenAI / Anthropic / Gemini / Bedrock-routed models
2. Tier 2 p99 < 1ms on commodity hardware（per `§10.1` benchmark；典型 cl100k_base / o200k_base encode ~10K tokens 輸入）
3. Tier 1 shadow 跑 30 連續日 ≥ 50K samples，|T1 − T2| / T1 < 1%
4. Tier 3 hit rate < 0.1% in demo modes（per `predictor-architecture-spec-v1alpha1.md` §0.2）
5. Circuit breaker chaos test：人工把 Tier 1 endpoint 設定為 timeout，10 consecutive failures 觸發 open；5 min 後 half-open；recover 後自動 close；期間 Tier 2 hot path 持續無中斷
6. `tokenizer_versions` registry table 通過 schema migration + audit chain `verify-chain` regression

### 0.3 GA prerequisites

於 `predictor-architecture-spec-v1alpha1.md` §0.3 列出。本 spec 額外要求：

1. Vendored BPE encoders（Anthropic / Gemini / Cohere / SentencePiece for Llama）通過 conformance corpus（cross-spec ref `trace-schema-spec-v1alpha1.md` §10.6 golden corpus convention）
2. Encoder version refresh playbook 文檔化（per §7.3）+ at least 1 successful production refresh drill
3. Per-tenant Tier 1 sampling rate override 機制驗證（dashboards + control plane API + audit event）

### 0.4 何時可能需要 v2 spec

只有以下情況開啟 v2：

- 新增第 4 個 tier（罕見；可能僅在 emerging provider 需要不同 latency/accuracy tradeoff 時）
- Tier 2 latency budget 需從 <1ms 鬆綁（非預期；觸發 v2 是因 SLO 改變影響整個 §10）
- Multi-modal tokenization（image / audio embeddings 同框）—— 此時可能拆 spec

---

## §1. Context (self-contained)

### 1.1 為什麼有這份 spec

SpendGuard 對 「token_in 必須 exact、token_out 是 calibrated projection」 的承諾（per HANDOFF §1.2 thesis）裡 **token_in exact** 這半完全依賴 tokenizer service。沒有 exact tokenize：

- CJK / multilingual prompts 2-3× under-estimate（per `predictor-architecture-spec-v1alpha1.md` §2.2 failure modes table）
- `max_tokens=4096` 但 prompt 短 → estimate 100 tokens → under-reserve ~40×（同上）
- tool_calls / vision / system metadata 漏算
- 跨 provider 用單一 `chars/4` formula → 預測完全不準

Tokenizer service 用 provider-native 的 BPE 把 token_in 變 byte-exact（對 OpenAI / 多數 provider）或 ≤ 1% drift（對 Anthropic / Gemini 等需 vendored BPE）。

### 1.2 三層的職責分工（重申 HANDOFF §3.2）

| Tier | 做什麼 | 何時用 | Hot path? | Latency |
|---|---|---|---|---|
| **Tier 1** | 呼叫 provider 官方 `count_tokens` API（Anthropic `POST /v1/messages/count_tokens`, Gemini `POST /v1/models/{model}:countTokens`） | Async shadow，配置 sampling rate（default 1%），drift detection only | ❌ 嚴禁 hot path | 50–80ms 網路 roundtrip |
| **Tier 2** | Local exact BPE：`tiktoken-rs` for OpenAI；vendored BPE for Anthropic / Gemini / Cohere；SentencePiece for Llama | **All hot-path tokenization**；source of truth for reservation | ✅ | < 1ms p99 |
| **Tier 3** | `chars / 4 × 2` heuristic + 5% conservative margin | Last-resort fallback for unknown models（fine-tuned / off-list）；每次 hit 必發 metric；健康部署 < 0.1% | ✅（極少觸發） | < 1ms |

### 1.3 為什麼 Tier 2 在 hot path（鎖定論證）

HANDOFF §5.2 已 lock 該決策：

1. **Latency** — Tier 1 加 50-80ms 摧毀 Contract §14 50ms p99 SLO + 每 burst benchmark 輸給 LiteLLM / Portkey
2. **Reliability** — Tier 1 依賴 provider availability；outage cascade 出 SpendGuard reservation failure
3. **Drift 仍可偵測** — 1% sample + alert 後 100% cool-down window
4. **OpenAI 無 drift** — tiktoken 公開 BPE，byte-exact

### 1.4 v1alpha1 核心哲學

> **Tier 2 是 source of truth**；Tier 1 是 drift verification；Tier 3 是 last-resort fallback。三者**不投票**；reservation 永遠用 Tier 2（或 Tier 3 fallback）。
>
> **每個 encoder 都有 `tokenizer_version_id`**；audit chain 紀錄 which encoder version tokenized this row。Refresh encoder → bump version_id → audit row trace per-version drift.
>
> **Per-provider dispatch table 是 explicit**；model string → encoder kind 對應 hardcode，不靠 fuzzy match。Unknown model → Tier 3 + metric alert。
>
> **Tier 1 shadow 嚴格 async**；任何同步 Tier 1 呼叫都是 bug —— hot path latency invariant 不可妥協。

---

## §2. Service surface

### 2.1 部署形態（兩種 co-exist）

| 形態 | 用途 | Transport |
|---|---|---|
| **(a) gRPC service**（centralized） | output_predictor、calibration_report、SDK fallback、Tier 1 shadow worker 集中 | tonic gRPC over mTLS（per Sidecar §5 internal transport） |
| **(b) Rust library**（`spendguard-tokenizer` crate） | sidecar / egress_proxy hot path | in-process function call（< 0.05ms overhead vs library load） |

(b) 是 Tier 2 hot path 主要方式（egress_proxy + sidecar 直接 link library）；(a) 用於非 hot path 場景。Library 與 service 共用同一份 `tokenizer_versions` registry 與 vendored BPE assets（per §7.2 asset bundling）。

### 2.2 gRPC proto outline

新檔案：`proto/spendguard/tokenizer/v1/tokenizer.proto`

```protobuf
// SpendGuard Tokenizer service.
//
// Spec references:
//   - tokenizer-service-spec-v1alpha1.md §3 (Tier 2 dispatch)
//   - audit-chain-prediction-extension-v1alpha1.md §2.1 (audit columns)
//   - sidecar-architecture-spec-v1alpha1.md §5 (mTLS internal transport)
//
// Transport: gRPC over mTLS (centralized form); in-process Rust library
// call (per-pod hot path form per §2.1).
//
// Compatibility: proto3, additive evolution; encoder version surfaced
// via tokenizer_version_id field in TokenizeResponse.

syntax = "proto3";
package spendguard.tokenizer.v1;
import "google/protobuf/timestamp.proto";

service Tokenizer {
  // Hot-path tokenize. Synchronous; returns Tier 2 result (or Tier 3
  // fallback) under 1ms p99.
  rpc Tokenize(TokenizeRequest) returns (TokenizeResponse);

  // Async shadow check via provider count_tokens API. Caller is
  // expected to be the shadow sampling worker, NOT the hot path.
  // Returns Tier 1 (provider-reported) count.
  rpc ShadowVerify(ShadowVerifyRequest) returns (ShadowVerifyResponse);
}

message TokenizeRequest {
  // Required. Model string as appears in the LLM request body
  // (e.g., "gpt-4o-mini", "claude-3-5-sonnet-20240620").
  string model = 1;

  // Messages array per OpenAI Chat Completions shape; tokenizer applies
  // model-specific message envelope tokens (system + user + assistant
  // role markers + content separators) where applicable.
  // For non-chat models or alternate envelope shapes, use `raw_text` instead.
  repeated Message messages = 2;
  string raw_text = 3;  // mutually exclusive with messages; for text-completion shape

  // Caller-supplied request_id for tracing/audit; mints UUIDv7 if empty.
  string request_id = 4;

  message Message {
    string role = 1;       // "system" | "user" | "assistant" | "tool"
    string content = 2;    // text content; vision/multimodal v2 will add binary content_parts

    // Tool call payload tokens count for tool-using agents. Tokenizer adds
    // function name + arguments JSON encoding to count.
    repeated ToolCall tool_calls = 3;

    message ToolCall {
      string name = 1;
      string arguments_json = 2;  // canonical JSON encoding of arguments
    }
  }
}

message TokenizeResponse {
  // Authoritative token count for the input.
  int64 input_tokens = 1;

  // Which tier was used. Always one of T2 | T3 (T1 never on hot path).
  string tier = 2;  // "T2" | "T3"

  // Encoder version that produced this count. Empty for Tier 3 (no
  // versioned encoder used).
  string tokenizer_version_id = 3;  // UUIDv7 of tokenizer_versions row

  // Encoder kind for diagnostics; redundant with version_id but cheap.
  string kind = 4;  // "OPENAI_TIKTOKEN" | "ANTHROPIC_BPE" | "GEMINI_BPE" | "COHERE_BPE" | "SENTENCEPIECE_LLAMA" | "HEURISTIC"

  // For Tier 3 fallback only: the underlying char count + conservative
  // margin applied; lets caller log / metric.
  int64 fallback_char_count = 5;
  float fallback_margin_ratio = 6;  // e.g., 1.05 for 5% conservative

  // Time spent inside Tokenize (excludes RPC overhead). Useful for SLO
  // tracking; not on audit chain.
  int64 latency_ns = 7;
}

message ShadowVerifyRequest {
  // Same shape as TokenizeRequest; shadow worker passes through.
  string model = 1;
  repeated TokenizeRequest.Message messages = 2;
  string raw_text = 3;

  // Tier 2 result for comparison.
  int64 t2_input_tokens = 4;
  string t2_tokenizer_version_id = 5;
}

message ShadowVerifyResponse {
  // Provider-reported count.
  int64 t1_input_tokens = 1;

  // Absolute drift |T1 - T2| / T1; alert threshold compare against
  // configured threshold (default 1.0%).
  float drift_ratio = 2;

  // Whether this sample triggered a drift_alert event emission.
  bool drift_alert_emitted = 3;

  // Latency to provider; for circuit-breaker metrics.
  int64 provider_latency_ms = 4;

  // Provider response identifiers for debugging.
  string provider_request_id = 5;
}
```

### 2.3 Rust library signature

```rust
// crate: spendguard-tokenizer
//
// Public surface aligned with the gRPC proto; same semantics, in-process call.

pub struct Tokenizer { /* per-process state holding encoder cache */ }

impl Tokenizer {
    pub fn new(asset_dir: &Path) -> Result<Self, TokenizerError>;
    pub fn tokenize(&self, req: &TokenizeRequest) -> Result<TokenizeResponse, TokenizerError>;
    // No ShadowVerify in library; shadow path always goes through gRPC + provider HTTP client.
}
```

---

## §3. Tier 2 — hot path exact tokenization

### 3.1 Per-provider dispatch table

The dispatch table is maintained in
`crates/spendguard-tokenizer/src/dispatch.rs::RAW_ENTRIES` (plus the
feature-gated `COHERE_ENTRIES`). The structural rule is **first-match
wins; anchored regex**; new providers / model families ship as
PR-time additive edits to those constants.

The R2 M7 verbatim enumeration below mirrors the implementation as of
SLICE_04 R2. The source-of-truth is `dispatch.rs`; this section is
kept in sync via the spec-amendment commits.

```rust
// === OpenAI (via tiktoken-rs) ===
// Most specific patterns first per first-match-wins semantics.
Entry { pattern: r"^gpt-4o-mini(-\d{4}-\d{2}-\d{2})?$",            kind: Kind::OpenAiTiktoken,    encoder: "o200k_base"     },
Entry { pattern: r"^gpt-4o(-\d{4}-\d{2}-\d{2})?$",                 kind: Kind::OpenAiTiktoken,    encoder: "o200k_base"     },
Entry { pattern: r"^gpt-4(-\d{4})?-preview$",                      kind: Kind::OpenAiTiktoken,    encoder: "cl100k_base"    },
Entry { pattern: r"^gpt-4-turbo(-preview)?(-\d{4}-\d{2}-\d{2})?$", kind: Kind::OpenAiTiktoken,    encoder: "cl100k_base"    },
Entry { pattern: r"^gpt-4(-\d{4})?(-\d{4}-\d{2}-\d{2})?$",         kind: Kind::OpenAiTiktoken,    encoder: "cl100k_base"    },
Entry { pattern: r"^gpt-3\.5-turbo(-\d{4})?(-\d{2}k)?$",           kind: Kind::OpenAiTiktoken,    encoder: "cl100k_base"    },
Entry { pattern: r"^gpt-3\.5-turbo-instruct(-\d{4})?$",            kind: Kind::OpenAiTiktoken,    encoder: "p50k_base"      },
Entry { pattern: r"^text-davinci-(002|003)$",                      kind: Kind::OpenAiTiktoken,    encoder: "p50k_base"      },
Entry { pattern: r"^code-davinci-(001|002)$",                      kind: Kind::OpenAiTiktoken,    encoder: "p50k_base"      },

// === Anthropic native (Claude 3 / 3.5 family) ===
Entry { pattern: r"^claude-3-5-(sonnet|haiku|opus)(-\d{8})?$",     kind: Kind::AnthropicBpe,      encoder: "anthropic-v3-bpe" },
Entry { pattern: r"^claude-3-(haiku|sonnet|opus)(-\d{8})?$",       kind: Kind::AnthropicBpe,      encoder: "anthropic-v3-bpe" },

// === Anthropic Bedrock (with cross-region inference profile prefix) ===
Entry { pattern: r"^(?:[a-z][a-z0-9-]*\.)?anthropic\.claude-3-5-(sonnet|haiku|opus)(-\d{8})?-v\d+:\d+$",
                                                                   kind: Kind::AnthropicBpe,      encoder: "anthropic-v3-bpe" },
Entry { pattern: r"^(?:[a-z][a-z0-9-]*\.)?anthropic\.claude-3-(haiku|sonnet|opus)(-\d{8})?-v\d+:\d+$",
                                                                   kind: Kind::AnthropicBpe,      encoder: "anthropic-v3-bpe" },

// === Gemini native (vendored Gemma approximation per §7.1 R2 M5) ===
Entry { pattern: r"^gemini-2\.0-flash(-exp)?$",                    kind: Kind::GeminiBpe,         encoder: "gemini-1.5-bpe" },
Entry { pattern: r"^gemini-1\.5-(flash|pro)(-\d{3})?$",            kind: Kind::GeminiBpe,         encoder: "gemini-1.5-bpe" },

// === Llama Bedrock (Llama 3.1+ family with cross-region prefix) ===
Entry { pattern: r"^(?:[a-z][a-z0-9-]*\.)?meta\.llama3(-\d+)?-\d+b-instruct-v\d+:\d+$",
                                                                   kind: Kind::SentencepieceLlama, encoder: "llama-sentencepiece" },

// === Cohere — FEATURE-GATED `cohere` per §7.1 R2 M6 ===
//   * Patterns below are present in the dispatch table ONLY when the
//     spendguard-tokenizer crate is built with `--features cohere`.
//   * When the feature is OFF (default), Cohere model IDs fall to
//     Tier 3 with 5% margin + `tokenizer_unknown_model` metric.
//   * `command-light` is INTENTIONALLY omitted per R2 Backend F4
//     (different vocab; routing to command-r would silently
//     under-count by ~5-20%).
#[cfg(feature = "cohere")]
Entry { pattern: r"^command-r-plus(-\d{8})?$",                     kind: Kind::CohereBpe,         encoder: "cohere-v2-bpe"  },
#[cfg(feature = "cohere")]
Entry { pattern: r"^command-r(-\d{8})?$",                          kind: Kind::CohereBpe,         encoder: "cohere-v2-bpe"  },
#[cfg(feature = "cohere")]
Entry { pattern: r"^(?:[a-z][a-z0-9-]*\.)?cohere\.command(-r)?(-plus)?-v\d+:\d+$",
                                                                   kind: Kind::CohereBpe,         encoder: "cohere-v2-bpe"  },
```

每個 entry 對應一筆 `tokenizer_versions` registry row（per §6 schema）。增加 provider / model 透過 PR 補 entries + 對應 vendored asset。

**Structural rule**: first-match-wins with dated suffixes optional and
cross-region inference profile prefixes optional on Bedrock routes.
Specific-before-general ordering is enforced by unit tests (per
`pattern_ordering_specific_before_general`, `command_r_plus_pattern_
precedes_command_r`, etc.).

#### 3.1 amendments (SLICE_04 R2)

The implementation deliberately differs from the loose "catch-all"
Bedrock patterns the v1alpha1 draft sketched (`^anthropic\.claude-.*$`,
`^cohere\..*$`, `^meta\.llama.*$`). Two structural rules govern the
divergence:

**(R2 B1) Cross-region inference profile prefixes.** AWS Bedrock since
2024-09 routes major models via cross-region inference profiles that
prepend a region prefix (`us`, `eu`, `apac`, `us-gov`, and future
regions AWS may add). Examples:

- `us.anthropic.claude-3-5-sonnet-20240620-v1:0`
- `eu.anthropic.claude-3-haiku-20240307-v1:0`
- `apac.anthropic.claude-3-5-sonnet-20241022-v1:0`
- `us.cohere.command-r-plus-v1:0`
- `us.meta.llama3-1-70b-instruct-v1:0`

The implementation admits any lowercase region prefix via the optional
group `(?:[a-z][a-z0-9-]*\.)?` so AWS adding a new region routes
automatically. Without this, cross-region IDs would silently fall to
Tier 3 with the 5% conservative margin — which under-counts CJK input
by ~2× per the predictor accuracy analysis.

**(R2 B2) Narrow patterns by design (Option A).** Each Bedrock vendor
family is narrowed to the specific model generation whose BPE we have
vendored. This is deliberate so that wrong-vocab models do NOT silently
route to the wrong encoder:

- Pre-Claude-3 IDs (`anthropic.claude-instant-v1`, `anthropic.claude-v2`,
  `anthropic.claude-v2:1`) fall to Tier 3 — different vocab from Claude 3.
- Cohere embedding models (`cohere.embed-english-v3`,
  `cohere.embed-multilingual-v3`) fall to Tier 3 — different vocab from
  command-r.
- Pre-Llama-3 IDs (`meta.llama2-13b-chat-v1`, `meta.llama2-70b-chat-v1`)
  fall to Tier 3 — different SentencePiece config from Llama 3.

Each Tier 3 fallback emits the `tokenizer_unknown_model` metric per
§3.3 so operators see the gap and PR a tracked follow-up. The rationale:
dispatching wrong-vocab encoders produces silent ~5-20% under-counts;
falling to Tier 3 produces a 5% conservative margin + a visible metric.

**(R2 Backend F4) `command-light` is omitted intentionally.** Cohere's
`command-light` uses a different BPE vocabulary than `command-r`.
Routing `command-light` to the `cohere-v2-bpe` encoder would silently
under-count tokens by ~5-20%. The dispatch table does NOT include a
`command-light` row; the model falls to Tier 3 until a separate
`command-light` tokenizer asset is vendored in a future SLICE_NN.

The full enumeration of current patterns lives in
`crates/spendguard-tokenizer/src/dispatch.rs::RAW_ENTRIES`; that file is
the structural source of truth, this section is the policy intent.

### 3.2 Encoder cache

`Tokenizer::new` 在啟動時 eager-load 所有 dispatch table 中的 encoder assets 到 process memory。Encoder objects 是 immutable + thread-safe（per tiktoken-rs API contract；vendored BPE 用 `Arc<EncoderRef>` 包裝）。Hot path tokenize 無 lock contention。

Asset 來源 per §7.2。Asset 大小總和 typically < 50 MB（tiktoken cl100k_base ~10 MB；其他類似量級）。

### 3.3 Exact model string normalization

Pattern matching 採 regex（per §3.1 dispatch table）。不做 fuzzy match —— `gpt-4o-mini-foo-bar` 是「unknown model」走 Tier 3，**不**默默用 cl100k_base。理由：fuzzy match 在新 provider release 時可能誤分類，造成 silent under-estimate。

Dispatch lookup 不命中 → return Tier 3 fallback + emit metric `tokenizer_unknown_model{ model="..." }`. Operator 看 metric 後手動 PR 補 entry。

### 3.4 Message envelope token accounting

對 OpenAI chat shape：

- Per-message envelope tokens：tiktoken's published rules（gpt-4o: 3 tokens per message + 1 token for role + content tokens）
- tool_calls: function name + arguments_json 都按 model 的 tokenizer encode
- system / user / assistant role markers: model-specific token sequences

對 non-OpenAI vendors（Anthropic / Gemini / Cohere / Llama）：each vendored
encoder ships its own [`ChatEnvelope`] policy + BOS rule per the
amendments below (R2 M3 + R2 M4 + R3 N1).

#### 3.4.1 Per-vendor chat envelope table（R2 M3 amendment, R2 commit `4d6f96b`）

`ChatEnvelope { per_message, per_turn_boundary, reply_priming }` policy
per encoder. The per-request total is

```text
chat_total = Σ_messages (per_message + per_turn_boundary + tokens(role) +
                         tokens(content) + Σ_tool_calls tool_overhead)
             + reply_priming
             + (BOS only if request uses raw_text path; see §3.4.2)
```

| Encoder                   | per_message | per_turn_boundary | reply_priming | Source-of-truth                                       |
|---------------------------|-------------|-------------------|---------------|-------------------------------------------------------|
| OpenAI cl100k / o200k     | 3           | 0                 | 3             | `encoders/openai.rs::OpenAiEncoder::envelope_overhead` |
| OpenAI gpt-3.5-turbo-0301 | 4           | 0                 | 3             | `encoders/openai.rs::count_tokens` (model-specific arm) |
| Anthropic                 | 0           | 4                 | 0             | `encoders/anthropic.rs::ANTHROPIC_ENVELOPE`           |
| Gemini                    | 0           | 0                 | 0             | `encoders/gemini.rs::GEMINI_ENVELOPE`                 |
| Cohere                    | 3           | 0                 | 0             | `encoders/cohere.rs::COHERE_ENVELOPE`                 |
| Llama                     | 5           | 0                 | 0             | `encoders/llama.rs::LLAMA_ENVELOPE`                   |

Rationale per row:

- **OpenAI cl100k / o200k**: tiktoken cookbook convention (3 tokens / msg
  for role + content separator markers; 3 tokens for assistant reply
  priming). `gpt-3.5-turbo-0301` snapshot is the documented quirk
  (per_message=4 for the legacy implicit "name" position).
- **Anthropic**: chat shape uses `\n\nHuman:` / `\n\nAssistant:` turn
  boundary markers (≈ 4 tokens per turn) rather than OpenAI-style
  per-message framing; no reply priming.
- **Gemini**: API takes a `contents` array where role is a structured
  field, not a prompt token; envelope is all-zero.
- **Cohere**: Command-R chat uses `<|START_OF_TURN|>` / `<|END_OF_TURN|>`
  framing (≈ 3 tokens per turn).
- **Llama**: Llama 3.1 chat uses the full header template
  `<|begin_of_text|><|start_header_id|>{role}<|end_header_id|>\n\n
  {content}<|eot_id|>` (≈ 5 tokens per turn for the marker headers).

Tool-call accounting is uniform across encoders: `+1` overhead per
tool_call + `tokens(name)` + `tokens(arguments_json)` (per spec §3.4
SLICE_03 line + per-vendor encoder).

#### 3.4.2 Per-vendor BOS token table（R2 M4 amendment, R2 commit `4d6f96b`；R3 N1 fix amendment, R3 commit `edbbfab`）

`bos_token_count` is added once per non-empty `raw_text` encode (chat-shape
requests do NOT add BOS — the per-turn header markers in §3.4.1 already
include any leading marker the vendor prepends).

| Encoder    | bos_token_count                       | Applies to                                       |
|------------|---------------------------------------|--------------------------------------------------|
| OpenAI     | 0                                     | All paths (cl100k / o200k / p50k have no BOS in `encode_with_special_tokens` output) |
| Anthropic  | 1 (Bedrock routing); 0 (native API)   | R3 N1 gate via `is_bedrock_routing(model)` substring match on `anthropic.` |
| Gemini     | 0                                     | All paths (Gemma `countTokens` reports no BOS)   |
| Cohere     | 1                                     | Bedrock only (feature-gated; native API path absent until R2 M6 legal review widens scope) |
| Llama      | 1                                     | Bedrock only (SLICE_04 ships only `meta.llama3-*-instruct-v1:0` patterns, all Bedrock) |

Rationale per row:

- **OpenAI**: tiktoken vocabularies have no BOS in the encode output
  (the `<|endoftext|>` token is appended at model-emit time, not on input).
- **Anthropic R3 N1 gate**: Bedrock invokeModel prepends `<|begin_of_text|>`
  before forwarding; the Anthropic native `/v1/messages` API does NOT.
  Unconditional BOS=1 over-counts native API calls by exactly 1 token,
  which crosses the §4.2 1% drift threshold on every 100-token request.
  Detection: model string contains the `anthropic.` substring (matches
  every Bedrock dispatch entry per §3.1, including cross-region prefixes
  `us.anthropic.…`, `eu.anthropic.…`, `apac.anthropic.…`, `us-gov.anthropic.…`).
- **Gemini**: Gemma vocab has no BOS in `countTokens` semantics.
- **Cohere**: Bedrock invokeModel prepends `<|START_OF_TURN|>`. Cohere
  native API path is feature-gated (per R2 M6); when the feature is on,
  the same Bedrock-only BOS=1 policy applies because all dispatch entries
  are still Bedrock-shaped IDs (`cohere.command-*-v1:0`) or
  `command-r{,-plus}{,-DATE}` SDK names that route via the same encoder.
- **Llama**: Bedrock invokeModel prepends `<|begin_of_text|>`. SLICE_04
  ships only Bedrock model patterns for Llama (no native SDK pattern).

#### 3.4.3 Source-of-truth note

The authoritative envelope + BOS values live in the Rust constants under
`crates/spendguard-tokenizer/src/encoders/*.rs` (`*_ENVELOPE` and
`*_BOS_COUNT*` consts surfaced via `Encoder::envelope_overhead` +
`Encoder::bos_token_count` trait methods). The tables in §3.4.1 / §3.4.2
above are illustrative — when the implementation amends a row, the spec
amendment commit MUST update the corresponding cell in the same PR (per
the §10 spec-impl sync rule introduced in R2 M5).

各 model 的 envelope + BOS 規則直接由 per-encoder trait method 提供（per
`crates/spendguard-tokenizer/src/encoders/mod.rs::Encoder`）；dispatch
table 不再 carry an `envelope_tokens` helper. The earlier sketch of a
`dispatch::message_envelope_tokens(kind, role)` helper is **superseded**
by the per-encoder trait shape — see R2 M3 implementation for the
migration rationale (one helper per kind kept envelope code adjacent to
the encoder's own vocab knowledge, eliminating a centralised `match` arm
that would have to grow with every new vendor row).

### 3.5 Output cap accounting

`max_tokens` parameter 來自 LLM request body。Strategy A reservation 算式 per `predictor-architecture-spec-v1alpha1.md` §3.1：

```
reservation = min(request.max_tokens, model.context_window - input_tokens) × price_per_token
```

`model.context_window` 從 `model_context_window` lookup table（per `output-predictor-service-spec-v1alpha1.md` §3）。Tokenizer service **不**做這個 lookup —— 只回 `input_tokens`；reservation 算式由 output_predictor 進行。

---

## §4. Tier 1 — async shadow drift detection

### 4.1 採樣機制

```rust
// Pseudocode for shadow worker in services/tokenizer/src/shadow_worker.rs

async fn shadow_loop(rx: Receiver<TokenizeEvent>) {
    let mut sample_rates: HashMap<(TenantId, ModelStr), f32> = load_from_control_plane().await;

    while let Some(event) = rx.recv().await {
        let rate = sample_rates
            .get(&(event.tenant, event.model.clone()))
            .copied()
            .unwrap_or(0.01);  // default 1%

        if random_float() > rate { continue; }

        // Async path: provider count_tokens API
        let t1_result = call_provider_count_tokens(&event).await;
        let drift_ratio = (t1_result.tokens - event.t2_tokens).abs() as f32 / t1_result.tokens as f32;

        if drift_ratio > DRIFT_THRESHOLD {
            emit_drift_alert(event.tenant, event.model.clone(), drift_ratio).await;
            raise_cool_down_sample_rate(event.tenant, event.model, 1.0).await;
            // 100% sampling for 1 hour after first alert, then revert
        }

        record_shadow_sample(&event, &t1_result).await;  // write to tokenizer_t1_samples table per §4.4
    }
}
```

### 4.2 Drift alert threshold

| Tokenizer kind | DRIFT_THRESHOLD | Rationale |
|---|---|---|
| `OPENAI_TIKTOKEN` | 0.0 (any drift) | tiktoken byte-exact；任何 drift 等於 vendor bug，立刻 alert |
| `ANTHROPIC_BPE` | 0.01 (1%) | vendored BPE 可能落後 vendor 微調；1% threshold tolerate noise |
| `GEMINI_BPE` | 0.01 (1%) | **R2 M5 honest disclosure**: vendored asset is Gemma approximation (Google's official Gemini tokenizer is API-only); 1% threshold absorbs the approximation gap. SLICE_05 shadow worker measures the actual delta against `countTokens` API in production; spec will revise the threshold per measured drift (or switch Gemini to Tier 1-only if the gap exceeds the SpendGuard accuracy promise). |
| `COHERE_BPE` | 0.015 (1.5%) | Cohere tokenizer 較不穩定；threshold 略寬 |
| `SENTENCEPIECE_LLAMA` | 0.005 (0.5%) | SentencePiece 配置精確；嚴格 threshold |

threshold per-kind 寫在 `dispatch.rs`；可在 control plane API 對特定 tenant 暫時 override（用於 incident response）。

### 4.3 100% cool-down window

當任一 (tenant, model) 觸發 drift alert：

- Sample rate 自動拉到 100% 持續 1 hour（cool-down window）
- 期間每筆 Tier 2 tokenize 都跑 Tier 1 shadow
- 若 1 hour 內無新 alert（drift 已恢復）→ sample rate 自動降回 baseline（default 1% 或 per-tenant config）
- 若 1 hour 內 ≥ 3 次新 alert → 持續維持 100% 並 page on-call

機制目的：drift 多半是 vendor tokenizer update；100% sampling 期間捕捉更多 sample 證實 / 否認 drift 持續存在。

### 4.4 `tokenizer_t1_samples` table

Tier 1 shadow 結果寫到獨立 table（**不**進 audit chain，因 T1 是 verification only 不是 enforcement source of truth）。

```sql
CREATE TABLE tokenizer_t1_samples (
    sample_id        UUID PRIMARY KEY,  -- UUIDv7
    tenant_id        UUID NOT NULL,
    model            TEXT NOT NULL,
    sampled_at       TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    t1_input_tokens  INT NOT NULL,
    t2_input_tokens  INT NOT NULL,
    t2_tokenizer_version_id UUID NOT NULL REFERENCES tokenizer_versions(tokenizer_version_id),
    drift_ratio      REAL NOT NULL,
    drift_alert_emitted BOOLEAN NOT NULL,
    provider_request_id TEXT
);

CREATE INDEX tokenizer_t1_samples_alert_idx
  ON tokenizer_t1_samples (tenant_id, model, sampled_at DESC)
  WHERE drift_alert_emitted = TRUE;
```

Migration source of truth: `services/ledger/migrations/0051_tokenizer_t1_samples.sql`
uses `tokenizer_t1_samples_alert_idx`; any older `drift_idx` spelling is
stale draft text.

Retention：90 日（per tenant policy override allowed）。Drift 分析 / vendor update 偵測用。Calibration-report CLI 不讀此表 —— 它的數據在 `audit_outbox` prediction columns。

### 4.5 Circuit breaker

Per (tenant, model) 級別 circuit breaker for Tier 1 endpoint：

- 10 consecutive failures（timeout / 5xx / connection refused）→ open state
- Open 5 min → half-open (1 probe request)
- Probe success → close
- Probe fail → re-open 5 min

Open 期間：Tier 1 shadow 對該 (tenant, model) skip；不影響 Tier 2 hot path（hot path 不知道 circuit breaker 狀態 —— shadow worker 自管）。

---

## §5. Tier 3 — heuristic fallback

### 5.1 Formula

```
tier3_input_tokens = ceil(total_chars / 4 × 1.05)
```

`1.05` 是 5% conservative margin —— 故意比 v1alpha1 之前的 17 行 heuristic（`chars / 4 × 2`）窄；因為 Tier 3 只在 unknown model 時觸發，operator 應該主動 PR 補 dispatch entry，不該長期靠 Tier 3 撐。寬 margin 會讓 operator 缺乏動機修。

The implementation counts `total_chars` with Rust's Unicode-aware
`text.chars().count()`. CJK-heavy unknown-model traffic is not fail-closed:
many CJK characters are close to one token each, while this fallback still
divides by four and applies only the 5% margin. Operators must treat CJK Tier
3 hits as an accuracy-risk signal and add a real dispatch entry instead of
letting this path persist. The risk is acceptable only because Tier 3 is a
last-resort path with the §5.3 health invariant
(`tokenizer_tier3_hit_total / total_tokenize_calls < 0.001`).

對於可能輸出 reasoning tokens 的 unknown model：不額外加倍。Tier 3 是 input-side fallback only；output projection 由 output_predictor 的 Strategy A 用 `max_tokens` 處理。

### 5.2 每次 hit 必發 metric

```
metric: tokenizer_tier3_hit_total
labels: { tenant, model, request_id_prefix }
```

Tenant + model 維度 → calibration-report CLI 識別「哪 customer 的哪 model 還沒被 dispatch table 覆蓋」並產生「需要補的 dispatch entry 清單」recommendation。

### 5.3 Health invariant

> 健康部署應有 `tokenizer_tier3_hit_total / total_tokenize_calls < 0.001` (0.1%).

突破 0.1% → control plane alert + 推 SLICE-extra PR 補 dispatch entries。

### 5.4 為什麼 Tier 3 仍然存在（不直接 fail-closed）

考慮過直接「unknown model → fail-closed reservation refuse」。**拒絕**：

- 客戶 fine-tune model（OpenAI / Anthropic 都允許）名稱無法預先進 dispatch table
- demo / experiment 場景常用未發表 internal model 名稱
- Fail-closed 對 first-call experience 太 brittle

Tier 3 + metric + alert 是 graceful degradation：可用但被監測，operator 知道要補。

---

## §6. `tokenizer_versions` registry table

### 6.1 Schema（per audit-chain-extension §4.1 placeholder 升 final）

```sql
CREATE TABLE tokenizer_versions (
    tokenizer_version_id UUID PRIMARY KEY,  -- UUIDv7
    kind                 TEXT NOT NULL CHECK (kind IN (
        'OPENAI_TIKTOKEN', 'ANTHROPIC_BPE', 'GEMINI_BPE',
        'COHERE_BPE', 'SENTENCEPIECE_LLAMA', 'HEURISTIC'
    )),
    encoder_name         TEXT NOT NULL,        -- e.g., "cl100k_base", "anthropic-v3-bpe"
    version_string       TEXT NOT NULL,        -- e.g., "tiktoken-0.7.0", "anthropic-bpe-2026-03"
    asset_sha256         TEXT NOT NULL,        -- 64-char hex; integrity check
    registered_at        TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    retired_at           TIMESTAMPTZ,           -- NULL = active; non-NULL = retired (still verify-able for old audit rows)
    UNIQUE (kind, encoder_name, version_string)
);

CREATE INDEX tokenizer_versions_active_idx
  ON tokenizer_versions (kind, encoder_name)
  WHERE retired_at IS NULL;
```

### 6.2 何時 register new version

- 升 `tiktoken-rs` crate dependency → 新 row（kind=OPENAI_TIKTOKEN, version_string=tiktoken-x.y.z）
- Refresh vendored Anthropic / Gemini / Cohere BPE → 新 row（per kind）
- Encoder hot-reload（per §7.3）→ 新 row 但**不 retire 舊 row**（舊 audit row 仍引用 old version_id 必須可 verify）

### 6.3 Retire 條件

舊 version 在沒有任何 audit_outbox row 引用 ≥ 7 年（per SOX retention）後可標 `retired_at`。Retire ≠ delete；row 永遠保留供未來 audit 查詢。

### 6.4 對 audit chain 影響

per `audit-chain-prediction-extension-v1alpha1.md` §2.1：每筆 decision audit row 寫 `tokenizer_version_id` FK 至本 table。FK constraint 強制 referential integrity。Tier 3 fallback → `tokenizer_version_id` NULL（per audit-chain extension §2.1 nullable rules）。

The `kind='HEURISTIC'` seed row is deliberately not reachable through
the `audit_outbox.tokenizer_version_id` FK for Tier 3 rows. It exists for
diagnostic and calibration-report joins that need a stable registry row
for the fallback kind; audit rows still encode the fallback with NULL /
empty-string sentinel semantics.

---

## §7. Vendored encoder maintenance

### 7.1 來源

| Kind | 來源 | License |
|---|---|---|
| `OPENAI_TIKTOKEN` | `tiktoken-rs` crate（Cargo dep） | MIT (OpenAI public) |
| `ANTHROPIC_BPE` | 從 `@anthropic-ai/tokenizer` JS package port，或 reconstruct from Anthropic published BPE merges | MIT (Anthropic public) |
| `GEMINI_BPE` | **Community Gemma approximation** (per R2 M5) — Google's official Gemini tokenizer is API-only; no vendored BPE merges file is available. We ship the Xenova/gemma-tokenizer mirror which exposes the open-source Gemma vocabulary. Spec §4.2 0.01 drift threshold accommodates the approximation gap; SLICE_05 shadow worker measures the residual against the official `countTokens` API. | Apache 2.0 (Gemma upstream) / MIT (Xenova mirror) |
| `COHERE_BPE` | **OPT-IN per R2 M6** — Xenova/c4ai-command-r-v01-tokenizer mirror. Underlying Cohere model is CC-BY-NC-4.0 (research-only) and the tokenizer-only redistribution terms are uncited; safe default ships the encoder behind a `cohere` Cargo feature flag (default OFF). Deployments enable `--features cohere` after their own legal review. See LICENSE_NOTICES.md and `7.1 R2 M6` subsection below. | MIT (Xenova mirror); model CC-BY-NC-4.0 |
| `SENTENCEPIECE_LLAMA` | Meta-released SentencePiece model files | Llama 2 Community License |

#### 7.1 R2 M5 — Gemini approximation honest disclosure

**Source URL** (pinned in `LICENSE_NOTICES.md`):
`https://huggingface.co/Xenova/gemma-tokenizer`

**License**: MIT (Xenova mirror) / Apache 2.0 (Gemma upstream).

**R2 M5 honest disclosure**:

* Google's official Gemini tokenizer is API-only (`countTokens` REST
  endpoint); Google does not publish a vendorable Gemini BPE merges file.
* SpendGuard vendors the Xenova community Gemma tokenizer as the closest
  publicly available approximation.
* There is no citable Google-published Gemma-vs-Gemini parity table; the
  actual gap must be measured by the SLICE_05 shadow worker against the
  official `countTokens` API.
* Spec §4.2 carries a 1% drift threshold to absorb the approximation gap.
* If production shadow data shows >1% drift, the threshold widens to the
  measured value or Gemini switches to Tier 1-only. Either path requires
  an operator-visible spec update.

The `tokenizer_versions` `kind=GEMINI_BPE` row's `version_string`
remains `gemini-1.5-bpe-2026-05` as a SpendGuard-internal asset id.

#### 7.1 R2 M6 — Cohere encoder opt-in feature flag

The R1 LICENSE_NOTICES claimed a "MPL-2.0 exemption" allowing
re-distribution of the Cohere `tokenizer.json` independently from the
CC-BY-NC-4.0 model weights. This claim is **uncited and legally
ambiguous**; the safe path until Cohere clarifies tokenizer-only
redistribution terms (or a separately-licensed encoder asset is
vendored) is to ship the Cohere encoder behind an opt-in Cargo feature.

The `spendguard-tokenizer` crate exposes a `cohere` feature flag,
**default OFF**:

* **OFF (default)**: the `cohere.rs` module is not compiled, the
  `data/cohere-command-r/tokenizer.json` asset is not embedded, and
  the dispatch table omits Cohere patterns. Cohere model IDs
  (`command-r`, `command-r-plus`, `cohere.command*-v\d+:\d+`) fall to
  Tier 3 with the 5% conservative margin + the `tokenizer_unknown_model`
  metric.

* **ON (via `--features cohere`)**: the encoder loads at boot with the
  full two-layer integrity check (Layer A sha256 + Layer B cross-check
  fixture per §7.4.1), and the dispatch routes Cohere model IDs to the
  Tier 2 BPE encoder.

Stock deployments that have not completed their own legal review use
the default OFF path. Deployments that need Cohere Tier 2 accuracy
explicitly opt in. `services/tokenizer` (the centralized form per
§2.1(a)) enables `cohere` by default so its golden corpus tests pass;
`services/sidecar` and `services/egress_proxy` (the in-process library
form per §2.1(b)) default to OFF.

`crates/spendguard-tokenizer/LICENSE_NOTICES.md` carries the current
legal disclosure; any future revision of the Cohere model license OR
a separately-licensed encoder asset will be tracked in that file with
the §7.1 row updated to match.

### 7.2 Asset bundling

Encoder assets（BPE merges / vocab files）打包進 `spendguard-tokenizer` crate 的 `data/` directory；build.rs 在 cargo build 時把 assets embed 進 binary（用 `include_bytes!`）。

優點：

- 部署只需要 binary，不需 separate asset distribution
- Asset 與 code version pinned（升 crate 同時升 assets）
- 啟動時無 file I/O 等待

缺點：

- Binary size 加 ~50 MB
- Asset 更新需要 crate rebuild + redeploy（per §7.3 mitigated by hot-reload）

### 7.3 Refresh cadence + hot-reload

| Trigger | Cadence | Action |
|---|---|---|
| Drift alert sustained > 24h | Reactive | Operator PR refresh vendored asset for the drifting kind |
| Quarterly check | 每季 | Tokenizer service maintainer 主動 diff vendor's latest tokenizer 與 vendored version |
| Provider major model release | Reactive | PR 補 dispatch entry + 若需要新 encoder asset 一併 ship |

Hot-reload 機制：

- Control plane API `POST /tokenizer/versions/{id}/activate` 加載新 version 至 in-memory cache（不 restart service）
- 舊 version 仍在 cache（用於正在處理的 in-flight requests + reproduce historical decisions）
- 切換後新 request 用新 version，old version_id 仍可被 `verify-chain` reproduce
- `tokenizer_versions` 新 row INSERT 同時 trigger CloudEvent `tokenizer.version_activated`（audit event；signed; per Trace §7.5）

### 7.4 Signed bundle 機制

每個 vendored asset 在 release 時由 SpendGuard release pipeline ed25519 簽章；asset_sha256 + signature 進 `tokenizer_versions` row。`Tokenizer::new` 啟動時 verify signature；篡改 → refuse to load → fail-closed start。

#### 7.4.1 Dual-copy design + cross-check guard (SLICE_03 R2 amendment, 2026-05-30)

實作層面有一個微妙的 soundness gap 需要明確記錄。tiktoken-rs upstream 用 `include_str!` 把 `assets/*.tiktoken` embed 進 crate；我們也用 `include_bytes!` 把 *同樣的* `.tiktoken` files mirror 進 `crates/spendguard-tokenizer/data/`。`asset_sha256` constants 只覆蓋我們這份 copy；hot path 真正執行的是 `tiktoken_rs::*_singleton()`，它讀的是 upstream crate 自己 embed 的 bytes。如果 tiktoken-rs 的 vendor branch 被 tamper（或 supply-chain 攻擊把 upstream crate 換掉），我們的 sha256 check 不會察覺。

為堵住這個 gap，`Tokenizer::new` 在 sha256 check 之後執行 **runtime cross-check**：tokenize 一個固定的 fixture string (`CROSS_CHECK_FIXTURE = "spendguard-cross-check-fixture-v1alpha1"`)，跟 hard-coded `EXPECTED_*` token vectors 比對。任何 mismatch → `TokenizerError::AssetSignatureMismatch` → boot-fail 跟 sha256 mismatch 同 surface。

**Cross-check fixture maintenance**: bumping tiktoken-rs version 時，`cargo run --example discover_fixture_tokens` 重新印出新版的 token vectors，更新 `encoder_cache.rs` 的 `EXPECTED_*` arrays，跟 `asset_sha256` rotation 同時做（per §6.2）。

---

## §8. Failure modes

| 情境 | 嚴重性 | 行為 |
|---|---|---|
| Tier 1 endpoint timeout / 5xx | 低 | Circuit breaker open；shadow 對該 (tenant, model) skip；hot path 不受影響 |
| Tier 1 provider returns different schema | 中 | Shadow worker emit `provider_count_tokens_schema_drift` event；skip sample；下次 schema check |
| Tier 2 encoder load failure（asset 損壞 / signature 不對）| **高** | Tokenizer service **refuse to start**（fail-fast at boot）；既有 tokenizer instances 繼續服務 |
| Tier 2 encoder panic during tokenize | **高** | Hot path 上拋 error → sidecar fail-closed reservation（不允許 silently fallback to Tier 3 —— panic 可能代表 input 異常需要 escalate） |
| Tier 3 hit | 低 | 正常運作；emit metric；calibration-report 識別需補 entry |
| Dispatch table 不命中（unknown model） | 中 | Tier 3 fallback；emit metric `tokenizer_unknown_model{ model=... }` |
| `tokenizer_versions` table FK lookup fail | 中 | Stale `tokenizer_version_id` cache 與 DB 不一致；refresh cache；若仍 fail emit `tokenizer_versions_integrity_alert` |
| Hot-reload mid-tokenize | 低 | 用 `Arc<EncoderRef>` swap；in-flight request 持有舊 Arc 仍可完成；無 race |
| Library crate version mismatch with service | 中 | Boot-time check：service binary 與 library binary 的 `spendguard-tokenizer` version 必須一致；不一致 refuse-to-start |

---

## §9. Audit chain impact

每次 hot-path tokenize 結果寫入對應 audit row 的兩個欄位（per `audit-chain-prediction-extension-v1alpha1.md` §2.1）：

- `tokenizer_tier`：`T2` (絕大多數) 或 `T3` (fallback)
- `tokenizer_version_id`：FK to `tokenizer_versions`；Tier 3 fallback 時 NULL

CloudEvent proto mirror（per audit-chain extension §3.2）對應 tags 306-307。

**No new immutability trigger update needed** beyond audit-chain extension §5.2 already covering these two columns. Tokenizer service 本身**不直接寫 audit_outbox** —— 透過 sidecar 流程在 reserve stage 包裝寫入。

---

## §10. SLO

### 10.1 Hot path latency

| Tier | p50 | p99 | p99.9 |
|---|---|---|---|
| Tier 2 (library form, in-process) | < 0.1 ms | **< 1 ms** | < 5 ms |
| Tier 2 (gRPC form, mTLS roundtrip) | < 0.5 ms | < 3 ms | < 10 ms |
| Tier 3 (heuristic) | < 0.01 ms | < 0.05 ms | < 0.1 ms |
| Tier 1 (shadow, off hot path) | N/A | N/A | N/A |

Benchmark methodology：對 10K-token average input、所有 supported models、commodity hardware (8 vCPU, 16 GB RAM, c5/c6 EC2 baseline)。Bench harness 在 SLICE 03 acceptance 必含 + 持續 CI run。

Tier 2 p99 < 1ms 是 GA prerequisite #2 of this spec。

### 10.2 Shadow availability

Tier 1 shadow path SLO：**no hot-path SLO**（async；hot path 不依賴）。但 shadow worker 自己的 SLO：

- Sample queue lag p99 < 30s（積壓 backpressure threshold）
- 每 (tenant, model) drift_ratio 統計每日 emit health report event

---

## §11. GA prerequisites

於 `§0.3` 列出。本 spec 額外要求 chaos test：

### 11.1 Chaos test suite

```yaml
chaos_test_scenarios:
  - name: tier1_endpoint_outage
    inject: anthropic_count_tokens API 全部 timeout
    expected: circuit breaker open within 10 failures；shadow 對該 (tenant, anthropic) skip；hot path Tier 2 持續 < 1ms p99
    duration: 30 min
  - name: tier1_endpoint_recovery
    inject: outage 30 min 後恢復
    expected: half-open probe 成功；circuit breaker close；shadow 恢復 sample
  - name: encoder_hot_reload_under_load
    inject: 同時 100 RPS hot-path tokenize + control plane API activate new tokenizer_version
    expected: 0 errors；in-flight requests 完成 with old version_id；new requests 用 new version_id；audit chain verify 全綠
  - name: tier3_burst
    inject: 大量 unknown model 名稱 burst（模擬 customer fine-tune model rollout）
    expected: Tier 3 hit rate 短期高於 0.1% 但 hot path latency 不變；metrics emit；alert 觸發
  - name: drift_alert_cool_down
    inject: 人為 introduce 2% drift in Anthropic Tier 2 vs T1
    expected: 第一次 alert 觸發；sample rate 立即拉到 100%；維持 1 hour；若 drift 持續另一 alert 持續 100%；若 drift 消失 sample rate 回 1%
```

---

## §12. Adoption history

| Round | Reviewer | 採納率 | 主要產出 |
|---|---|---|---|
| (placeholder) | (placeholder) | (placeholder) | (placeholder — filled during Codex / panel adversarial review rounds per HANDOFF §9) |

---

## §13. Lock 後的下一步

1. SLICE 03 PR：tokenizer service skeleton (Tier 2 OpenAI only) + `spendguard-tokenizer` library crate + dispatch table for OpenAI models + `tokenizer_versions` table + assets bundling
2. SLICE 04 PR：Tier 2 expansion (Anthropic + Gemini + Bedrock routing) + per-kind drift thresholds
3. SLICE 05 PR：Tier 1 shadow worker + provider clients + circuit breaker + `tokenizer_t1_samples` table + drift alert
4. Hot-reload mechanism deferred to SLICE-extra (post first 3 slices land)；POC 階段 manual restart 接受
5. Bench harness in `benchmarks/tokenizer/` + CI integration

---

*Document version: tokenizer-service-spec-v1alpha1 (DRAFT) | Drafted: 2026-05-29 | Critical surface: §3.1 dispatch table；§4.2 per-kind drift thresholds；§6 tokenizer_versions registry；§7.4 signed bundle | Hot-path SLO: Tier 2 p99 < 1ms (library form) | Branch: `design/predictor-upgrade`*
