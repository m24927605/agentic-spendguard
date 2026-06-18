# Slice 04 — Tokenizer Tier 2 expansion (Anthropic + Gemini)

> **Branch**: `slice/SLICE_04_tokenizer_tier2_anthropic_gemini`
> **Status**: draft
> **Spec ancestor(s)**: `tokenizer-service-spec-v1alpha1.md` (primary §3, §7), `audit-chain-prediction-extension-v1alpha1.md`
> **Depends on prior slices**: SLICE_03 (tokenizer skeleton + dispatch table)
> **Blocks subsequent slices**: SLICE_05 (Tier 1 shadow needs Anthropic / Gemini Tier 2 baseline for comparison), SLICE_11 (multi-provider routing)
> **Estimated PR size**: medium (vendored BPE for 3 providers + dispatch expansion; ~1000 LOC + ~30 MB assets)

---

## §0. TL;DR

Add vendored BPE encoders for Anthropic (claude-v3-bpe), Gemini (gemini-1.5-bpe), Cohere (cohere-v2-bpe), and SentencePiece for Llama. Extend dispatch table per spec §3.1. Per-kind drift thresholds (per §4.2). Bedrock routing logic (per §3.1 entries). All as Tier 2 hot-path.

---

## §1. Architectural context

per `tokenizer-service-spec-v1alpha1.md` §3.1 (dispatch table expansion); §7.1 (asset sources + licenses). Serves Q2 (Tier 2 source of truth across all providers).

---

## §2. Scope (must-do)

- Vendored Anthropic BPE assets (port from `@anthropic-ai/tokenizer` JS or reconstruct from public merges)
- Vendored Gemini BPE assets (from Google AI tokenizer files)
- Vendored Cohere BPE assets
- SentencePiece model files for Llama
- Dispatch table entries for: Claude 3 / 3.5 family; Gemini 1.5 family; Bedrock routing patterns (anthropic.claude-*, cohere.*, meta.llama*)
- Per-kind drift thresholds (per spec §4.2 table): OpenAI 0.0, Anthropic 0.01, Gemini 0.01, Cohere 0.015, SentencePiece 0.005
- Add tokenizer_versions rows for each new encoder kind + version
- Encoder asset signing + bundle integrity check (consistent with SLICE_03)

---

## §3. Out of scope

| 項目 | 推給 |
|---|---|
| Tier 1 shadow comparison logic | SLICE_05 |
| Egress proxy multi-provider routing wire | SLICE_11 |
| Encoder hot-reload | post-launch |

---

## §4. File-level change list

### 4.1 New files

- `crates/spendguard-tokenizer/data/anthropic-v3-bpe/` (vendored asset directory)
- `crates/spendguard-tokenizer/data/gemini-1.5-bpe/`
- `crates/spendguard-tokenizer/data/cohere-v2-bpe/`
- `crates/spendguard-tokenizer/data/llama-sentencepiece/`
- `crates/spendguard-tokenizer/src/encoders/anthropic.rs`
- `crates/spendguard-tokenizer/src/encoders/gemini.rs`
- `crates/spendguard-tokenizer/src/encoders/cohere.rs`
- `crates/spendguard-tokenizer/src/encoders/sentencepiece.rs`

### 4.2 Modified files

- `crates/spendguard-tokenizer/src/dispatch.rs` — extend dispatch entries
- `services/tokenizer/migrations/00XX_anthropic_gemini_versions.sql` (or extend SLICE_03 migration)
- `LICENSE_NOTICES.md` — add per §7.1 license attributions

---

## §5. Schema / proto changes

Schema: tokenizer_versions rows added (multiple INSERTs in migration). No proto changes.

---

## §6. Audit-chain impact

- New `tokenizer_version_id` values for Anthropic / Gemini / Cohere / SentencePiece encoders
- Tier 2 hit rate for these models becomes > 0%; Tier 3 fallback rate should drop

---

## §7. Failure mode coverage

| 場景 | 行為 |
|---|---|
| Vendored BPE asset corrupted | refuse-to-start |
| Vendored BPE version drift from upstream | Tier 1 shadow catches (SLICE_05) |
| New Bedrock model not in dispatch | Tier 3 + metric |
| Anthropic dated suffix `-20240620` | regex match correctly per `tokenizer-service-spec-v1alpha1.md` §3.1 |

---

## §8. Acceptance criteria

- Each vendored encoder produces correct count for 50 golden samples per kind (200 total)
- Dispatch table covers Claude 3.5 Sonnet/Haiku/Opus dated suffixes + Bedrock routing
- Tier 3 hit rate < 0.1% on demo modes including `agent_real_anthropic`
- p99 latency < 1ms for all new encoders (library form)

---

## §9. Slice-specific adversarial review checklist

1. Where are the vendored assets sourced? Reproducible? URL-stable?
2. Each license verified against `LICENSE_NOTICES.md`?
3. Bedrock routing dispatch handles `anthropic.claude-3-5-sonnet-20240620-v1:0` (full Bedrock model id)?
4. Per-kind drift thresholds in dispatch.rs match spec §4.2 table verbatim?
5. SentencePiece model file integrity sig check?
6. Memory footprint: total asset size + decoded encoder RAM measured?
7. Migration adds tokenizer_versions rows or relies on runtime registration?
8. `tokenizer_version_id` consistency: same encoder version always produces same UUID v7?

---

## §10. Out-of-scope deferrals

| 項目 | 推給 |
|---|---|
| Cohere model envelope rules | follow up if needed |
| Vendored asset refresh | quarterly cadence; not slice-bound |

---

## §11. Risk / rollback plan

- Risk: vendored BPE mismatches actual vendor tokenizer (sub 1%)
- Mitigation: SLICE_05 drift detection catches in production
- Rollback: revert dispatch entries; affected models fall to Tier 3 fallback

---

## §12. Review Execution Notes

- Recommended reviewer profile: Backend Architect
- Review depth: deep
- Expected rounds: 2-3

---

## §13. Adoption history (filled during review)

| Round | Reviewer | 採納率 | 主要產出 |
|---|---|---|---|
| (placeholder) | (placeholder) | (placeholder) | (placeholder) |

---

## §14. Merge checklist

- [ ] §8 acceptance green
- [ ] §9 specific clear
- [ ] universal §1.2 + §1.8 green
- [ ] All license attributions present
- [ ] PR references `tokenizer-service-spec-v1alpha1.md` §3.1 §7

---

*Slice version: SLICE_04_tokenizer_tier2_anthropic_gemini v1alpha1 (draft) | Spec ancestor: tokenizer-service-spec-v1alpha1.md §3 §7 | Depends: SLICE_03 | Branch: `slice/SLICE_04_tokenizer_tier2_anthropic_gemini`*
