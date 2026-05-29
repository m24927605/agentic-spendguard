# Slice 12 — SDK integrations `default_estimator()`

> **Branch**: `slice/SLICE_12_sdk_default_estimators`
> **Status**: draft
> **Spec ancestor(s)**: `tokenizer-service-spec-v1alpha1.md` §3; `run-cost-projector-spec-v1alpha1.md` §5 (Signal 3 SDK)
> **Depends on prior slices**: SLICE_04 (Anthropic / Gemini Tier 2), SLICE_06 (output_predictor for SDK fallback path)
> **Blocks subsequent slices**: SLICE_15 (E2E benchmark across SDK paths)
> **Estimated PR size**: medium (Python SDK estimators + integrations update + decorator + tests; ~1000 LOC)

---

## §0. TL;DR

Python SDK gains `spendguard.estimators.*` module with `openai_default()`, `anthropic_default()`, `gemini_default()` functions using tiktoken / vendored BPE. Five integrations (litellm / langchain / pydantic_ai / openai_agents / agt) default `claim_estimator` parameter based on model family. `with_run_plan()` decorator for Signal 3. Backwards compat: explicit `claim_estimator` still wins.

---

## §1. Architectural context

per `tokenizer-service-spec-v1alpha1.md` (same dispatch table used in SDK); `run-cost-projector-spec-v1alpha1.md` §5 (Signal 3 wire). Serves Q2 (Tier 2 accessible from SDK without proxy).

---

## §2. Scope (must-do)

- New module `sdk/python/src/spendguard/estimators/` with `openai.py`, `anthropic.py`, `gemini.py`
- Each estimator uses corresponding Python tokenizer lib (tiktoken, anthropic-py BPE, google-py BPE)
- Implementations align with `tokenizer-service-spec-v1alpha1.md` §3.1 dispatch table semantically
- Default `claim_estimator` parameter in all 5 integrations selects estimator based on model family
- `with_run_plan(planned_calls, planned_tools)` decorator implementation
- Decorator wires `planned_steps_hint` into `request_decision` adapter UDS call
- `pyproject.toml` adds `tiktoken` as dependency; vendored Anthropic / Gemini tokenizers as data files
- Backwards compat: caller-supplied `claim_estimator` still wins when provided
- Integration test for each SDK with default estimator (no caller-supplied)

---

## §3. Out of scope

| 項目 | 推給 |
|---|---|
| Rust SDK (separate ecosystem) | post-launch |
| Go SDK (separate ecosystem) | post-launch |
| Vendored Cohere tokenizer in Python | follow-up |
| SDK gRPC fallback to tokenizer service | post-launch |

---

## §4. File-level change list

### 4.1 New files

- `sdk/python/src/spendguard/estimators/__init__.py`
- `sdk/python/src/spendguard/estimators/openai.py`
- `sdk/python/src/spendguard/estimators/anthropic.py`
- `sdk/python/src/spendguard/estimators/gemini.py`
- `sdk/python/src/spendguard/estimators/dispatch.py` (Python mirror of Rust dispatch table)
- `sdk/python/src/spendguard/run_plan.py` (`with_run_plan` decorator)
- `sdk/python/data/anthropic_bpe/` (data files for Anthropic BPE)
- `sdk/python/data/gemini_bpe/`

### 4.2 Modified files

- `sdk/python/src/spendguard/integrations/litellm.py`
- `sdk/python/src/spendguard/integrations/langchain.py`
- `sdk/python/src/spendguard/integrations/pydantic_ai.py`
- `sdk/python/src/spendguard/integrations/openai_agents.py`
- `sdk/python/src/spendguard/integrations/agt.py`
- `sdk/python/pyproject.toml` — add tiktoken dep; package data files
- `sdk/python/tests/integrations/test_*.py` — new tests for default estimator path

---

## §5. Schema / proto changes

No proto changes. SDK wire surface uses existing adapter.proto.

---

## §6. Audit-chain impact

- SDK-side computed `BudgetClaim` flows through to sidecar; audit row has `tokenizer_tier='T2'` and correct `tokenizer_version_id`
- Signal 3 hint via `planned_steps_hint` reaches projector

---

## §7. Failure mode coverage

| 場景 | 行為 |
|---|---|
| tiktoken not installed | pip install hint + raise clear error |
| Anthropic BPE asset missing | raise + suggest reinstall |
| Unknown model | fall to chars/4 heuristic + emit warning (SDK has no Tier 3 metric path; uses warnings module) |
| Caller-supplied claim_estimator vs default | caller wins (backwards compat) |
| `with_run_plan` decorator on non-async function | clear error |

---

## §8. Acceptance criteria

### 8.1 Unit tests

- Each estimator matches Rust tokenizer service output for 50 golden inputs
- Default estimator selection: model family correctly routes to right estimator
- `with_run_plan` decorator: `planned_steps_hint` correctly attached to request_decision

### 8.2 Integration tests

- LangChain ChatOpenAI with no `claim_estimator`: works with default; audit row populated
- Pydantic-AI with default estimator: same
- LiteLLM proxy with default: same
- OpenAI Agents SDK with `with_run_plan(planned_calls=5)`: projector sees hint

### 8.3 Property tests

- 100 varied model strings: estimator dispatches correctly (no AttributeError on unknown)

### 8.4 Demo-mode regression

- `make demo-up DEMO_MODE=agent_real_langgraph` works without caller-supplied claim_estimator
- All 8+ demos pass

### 8.5 Backwards compat

- Existing user code with explicit `claim_estimator = my_estimator` still uses my_estimator (no override)

---

## §9. Slice-specific adversarial review checklist

1. tiktoken version pin in pyproject.toml? Lockfile reproducibility.
2. Vendored Anthropic BPE: how synced with Rust vendored asset (SLICE_04)? Process documented.
3. Default selection priority: model_family inferred from model string? Show dispatch.
4. `with_run_plan` decorator: works with both sync + async functions?
5. `with_run_plan` decorator: nested decoration (call inside another run-planned function)?
6. Backwards compat: explicit None vs missing claim_estimator param treated the same?
7. SDK warnings: not metrics (sidecar metrics) — but emit Python warnings?
8. Package data files: properly included in wheel via pyproject.toml configuration?
9. Cold-start Python: no L4/L3 (those are server-side); SDK only does L2 via tokenizer family. Tested.
10. Cross-version compat with prior SDK 0.4.0 users?

---

## §10. Out-of-scope deferrals

| 項目 | 推給 |
|---|---|
| Rust / Go SDK | post-launch |
| Local cache in SDK for repeated tokenize | not yet needed |
| `with_run_plan` for TypeScript SDK | post-launch |

---

## §11. Risk / rollback plan

- Risk: default estimator divergence from server-side (e.g., SDK tokenizes differently than tokenizer service)
- Mitigation: 50 golden sample parity tests
- Rollback: revert SDK upgrade; users specify `claim_estimator` manually

---

## §12. AIT execution notes

- Recommended `--agent Backend Architect` (Python-specific dispatch optional)
- `--review-budget standard`
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
- [ ] universal §1.11 (backwards compat) verified for SDK users
- [ ] PyPI release script updated
- [ ] PR references `tokenizer-service-spec-v1alpha1.md` + `run-cost-projector-spec-v1alpha1.md` §5

---

*Slice version: SLICE_12_sdk_default_estimators v1alpha1 (draft) | Spec ancestors: tokenizer-service-spec + run-cost-projector §5 | Depends: SLICE_04, SLICE_06 | Branch: `slice/SLICE_12_sdk_default_estimators`*
