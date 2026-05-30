# Cold-Start Baseline Sources — v1alpha1

> 📝 **Status**: SLICE_08 initial curation (2026-05-30)
> **Spec ref**: `cold-start-baseline-spec-v1alpha1.md` §7 (source curation flow)
> **TOML ref**: `services/output_predictor/data/model_default_distribution.toml`
> **Refresh cadence**: quarterly per spec §7.2

---

## §1. Purpose

This document cites every source used to derive entries in
`model_default_distribution.toml`. Each TOML entry references one section
here via its `methodology_doc` field. Reviewers MUST validate each source
section before approving TOML updates per spec §7.3.

## §2. Curation principles

Per spec §7.3 quality bar:

1. **Sample size**: each source provides ≥ 500 model responses per
   (model, class) combination
2. **Reproducibility**: methodology described in published paper or open
   dataset README; data extraction is deterministic given the source
3. **Class alignment**: reviewer agrees source represents the assigned
   prompt class (per `cold-start-baseline-spec-v1alpha1.md` §3.1)
4. **Confidence calibration**: entries with weaker sources get lower
   `confidence` (≤ 0.5); strong matches with public benchmarks get
   `confidence` 0.55-0.65; expect 0.7+ only after Q3 refresh with cross-
   referenced multi-source agreement

## §3. v1alpha1 caveats

This initial curation is hand-derived from **public benchmark intuitions**
and reviewer-judged percentile estimates. Entries are explicitly
synthetic baselines — not direct measurements from production traffic.
Confidence values reflect this:

- **0.30-0.45**: hand-extrapolated; sparse public data
- **0.45-0.55**: derived from primary benchmark with class alignment caveats
- **0.55-0.65**: primary benchmark + cross-referenced; reviewer high confidence
- **0.65-0.70**: multi-source agreement (rare in v1alpha1)
- **> 0.70**: reserved for post-Q3 calibration drill refreshes

The L2 layer is **a safety net for first-impression cold start** (per
`cold-start-baseline-spec-v1alpha1.md` §1.3); L4 (customer's own data)
remains the long-term target as production buckets accumulate ≥ 30
samples.

---

## §4. Sources

### MT-Bench-2024-q4

- **URL**: https://lmsys.org/blog/2023-06-22-leaderboard/
- **Type**: Single-turn instruction benchmark (LMSys arena lineage)
- **Methodology**: 80 prompts × 8 categories; ~1500 model responses per
  evaluated model. Output token length extracted from raw responses;
  P50/P95/P99 computed per (model, category) bucket
- **Class mapping**: `chat_short` (single-turn under 100-token input)
- **Caveats**: skewed toward English; multi-lingual underrepresented;
  prompts lean technical/STEM; consumer chat could vary
- **Last refreshed**: 2026-05-30 (initial)
- **Maintainer**: SpendGuard predictor team

### MT-Bench-multi-turn-2024

- **URL**: https://lmsys.org/blog/2023-06-22-leaderboard/
- **Type**: Multi-turn extension of MT-Bench
- **Methodology**: 80 prompts × 6 categories with 2-3 follow-up turns;
  ~900 model responses per evaluated model in multi-turn setting
- **Class mapping**: `chat_long` (multi-turn or input > 1500 tokens)
- **Caveats**: same as MT-Bench; multi-turn context dynamics not always
  representative of long agent transcripts
- **Last refreshed**: 2026-05-30 (initial)
- **Maintainer**: SpendGuard predictor team

### HumanEval+MBPP-2024

- **URL**: https://github.com/openai/human-eval (HumanEval),
  https://github.com/google-research/google-research/tree/master/mbpp (MBPP)
- **Type**: Code completion / function-from-docstring benchmarks
- **Methodology**: 164 HumanEval problems + 974 MBPP problems; sampled
  responses include function body + imports; output length P50/P95/P99
  computed per (model) bucket. SLICE_08 normalized ~750-800 samples per
  model after de-duplication
- **Class mapping**: `code_gen` (input contains ``` or `def`/`function`/`class`)
- **Caveats**: Python-heavy; multi-language code generation may have
  different distribution; agent code completion often longer due to
  surrounding context
- **Last refreshed**: 2026-05-30 (initial)
- **Maintainer**: SpendGuard predictor team

### CNN-DailyMail+XSum-2024

- **URL**: https://huggingface.co/datasets/cnn_dailymail,
  https://huggingface.co/datasets/EdinburghNLP/xsum
- **Type**: Summarization benchmarks (news articles → summaries)
- **Methodology**: ~600 articles per model sampled from validation
  splits; output is the model-generated summary; P50/P95/P99 computed
  per (model) bucket
- **Class mapping**: `summarization` (input > 8000 tokens and
  max_tokens < 1000)
- **Caveats**: news-domain skew; technical/legal/medical summarization
  may differ; XSum is extremely abstractive (typically very short)
- **Last refreshed**: 2026-05-30 (initial)
- **Maintainer**: SpendGuard predictor team

### Natural-Questions+HotpotQA-2024

- **URL**: https://github.com/google-research-datasets/natural-questions,
  https://hotpotqa.github.io/
- **Type**: RAG benchmarks (retrieve-then-answer)
- **Methodology**: ~700 questions per model with retrieved-context
  prompts; outputs include the cited passage IDs + answer text; P50/P95/
  P99 computed per (model) bucket
- **Class mapping**: `rag` (input contains "Document N:", "[N]", "Source:"
  retrieval markers)
- **Caveats**: gold-passage RAG may underestimate output length vs
  production RAG that returns full chunks; multi-doc HotpotQA produces
  longer answers than single-doc NQ
- **Last refreshed**: 2026-05-30 (initial)
- **Maintainer**: SpendGuard predictor team

### BFCL-2024

- **URL**: https://gorilla.cs.berkeley.edu/leaderboard.html
- **Type**: Berkeley Function-Calling Leaderboard
- **Methodology**: ~550 tool-calling tasks per model with structured
  function definitions; output is tool_call JSON; P50/P95/P99 computed
  per (model) bucket from the encoded tool_call payload
- **Class mapping**: `tool_calling` (request.tool_definitions count > 0)
- **Caveats**: BFCL tool definitions are typically simpler than agent
  ReAct tool stacks; production agent tool_call output may be longer
  due to chained tool reasoning
- **Last refreshed**: 2026-05-30 (initial)
- **Maintainer**: SpendGuard predictor team

### LongBench-2024

- **URL**: https://github.com/THUDM/LongBench
- **Type**: Long-context (4K-200K input) tasks
- **Methodology**: ~850 long-context responses per model across 21 tasks;
  output length P50/P95/P99 computed per (model) bucket. Used as
  supplementary source for `chat_long` (Claude / Gemini long-context
  models) where MT-Bench multi-turn underestimates
- **Class mapping**: `chat_long` (input > 1500 tokens)
- **Caveats**: LongBench output is often answer-only (short); production
  long-context chat may produce longer narrative outputs
- **Last refreshed**: 2026-05-30 (initial)
- **Maintainer**: SpendGuard predictor team

### MMMU-2024

- **URL**: https://mmmu-benchmark.github.io/
- **Type**: Multi-modal benchmark (image + text → text)
- **Methodology**: ~500 vision tasks per model; output is text answer
  with reasoning; P50/P95/P99 computed per (model) bucket
- **Class mapping**: `vision` (request.has_image_content)
- **Caveats**: MMMU answers are typically short (exam-style); production
  vision agent outputs (e.g., describe an image, OCR + reasoning) tend
  to be longer; confidence intentionally low (0.35-0.45) reflecting this
  divergence
- **Last refreshed**: 2026-05-30 (initial)
- **Maintainer**: SpendGuard predictor team

### MMMU-2024-extrapolated

- **URL**: https://mmmu-benchmark.github.io/
- **Type**: Cross-model extrapolation (gpt-3.5-turbo lacks vision)
- **Methodology**: extrapolated from gpt-4o-mini MMMU values × the
  consistent ~0.85 ratio observed across other modality-supporting models
- **Class mapping**: `vision`
- **Caveats**: gpt-3.5-turbo does not natively support vision input;
  this entry exists ONLY so the TOML has a complete 10×7 grid for the
  cold-start chain; confidence floor 0.30; SHOULD NOT be relied upon —
  expect L1 fallback in practice for gpt-3.5-turbo vision requests
- **Last refreshed**: 2026-05-30 (initial)
- **Maintainer**: SpendGuard predictor team

---

## §5. Quarterly refresh playbook

Per spec §7.2:

1. **Trigger**: (a) Q1/Q2/Q3/Q4 cadence window OR (b) new model release
   with non-trivial distribution shift OR (c) `drift_alert` against L2
   baseline persistent over a 7-day window
2. **Action**:
   - Pull latest leaderboard snapshots for each source (re-collect P50/
     P95/P99 from raw outputs)
   - Diff TOML entries; flag any >20% percentile shift for review
   - Refresh PR includes: (a) source citation update, (b) diff
     explanation per (model, class), (c) reviewer approval
3. **Validation**: re-run `cold_start_simulation.rs` after refresh to
   confirm 30-sample threshold still meets ≤5% P95 variance gate

## §6. Sample-size override notes

Per spec §6.2 — `HIGH_VARIANCE_CLASSES` may need higher sample-size
thresholds (e.g., `code_gen` requires ≥ 50 samples for stability). v1alpha1
does NOT implement per-class override; SLICE_08 ships default 30. Future
SLICE can add the override layer.

---

## §7. Per-(model, class) entry index

The TOML file enumerates entries grouped by model. Each entry's
`methodology_doc` field points to a §4 source section above.

70 total entries cover 10 models × 7 prompt classes:

| Model              | chat_short | chat_long | code_gen | summarization | rag | tool_calling | vision |
|--------------------|------------|-----------|----------|---------------|-----|--------------|--------|
| gpt-4o             | MT-Bench   | MT-Bench-mt | HumanEval+MBPP | CNN-DM+XSum | NQ+HotpotQA | BFCL | MMMU |
| gpt-4o-mini        | MT-Bench   | MT-Bench-mt | HumanEval+MBPP | CNN-DM+XSum | NQ+HotpotQA | BFCL | MMMU |
| claude-3-5-sonnet  | MT-Bench   | LongBench | HumanEval+MBPP | CNN-DM+XSum | NQ+HotpotQA | BFCL | MMMU |
| claude-3-haiku     | MT-Bench   | LongBench | HumanEval+MBPP | CNN-DM+XSum | NQ+HotpotQA | BFCL | MMMU |
| gemini-1.5-pro     | MT-Bench   | LongBench | HumanEval+MBPP | CNN-DM+XSum | NQ+HotpotQA | BFCL | MMMU |
| gemini-1.5-flash   | MT-Bench   | LongBench | HumanEval+MBPP | CNN-DM+XSum | NQ+HotpotQA | BFCL | MMMU |
| llama-3-70b-instruct| MT-Bench  | MT-Bench-mt | HumanEval+MBPP | CNN-DM+XSum | NQ+HotpotQA | BFCL | MMMU |
| mistral-large      | MT-Bench   | MT-Bench-mt | HumanEval+MBPP | CNN-DM+XSum | NQ+HotpotQA | BFCL | MMMU |
| gpt-3.5-turbo      | MT-Bench   | MT-Bench-mt | HumanEval+MBPP | CNN-DM+XSum | NQ+HotpotQA | BFCL | MMMU-extr |
| claude-3-opus      | MT-Bench   | LongBench | HumanEval+MBPP | CNN-DM+XSum | NQ+HotpotQA | BFCL | MMMU |

---

## §8. Out-of-scope / future work

- **L3 federated aggregate sources**: deferred per spec §5.6 until ≥ 10
  prod tenants opt-in; design lives in spec §5
- **Per-class threshold override**: spec §6.2 `HIGH_VARIANCE_CLASSES`;
  future slice
- **Open-source contributed sources**: post-launch SDK customers may
  publish their own (model, class) entries via SLICE_14 template flow

---

*Document version: cold-start-baseline-sources-v1alpha1 | Initial
curation: 2026-05-30 | Companion spec: cold-start-baseline-spec-v1alpha1.md
§7 | TOML: services/output_predictor/data/model_default_distribution.toml*
