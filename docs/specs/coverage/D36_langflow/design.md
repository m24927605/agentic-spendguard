# D36 — Langflow Custom Python Component — design.md

**Status:** Spec — Tier 3, build plan §2.3.
**Parent strategy:** [`docs/strategy/framework-coverage-build-plan-2026-06.md`](../../../strategy/framework-coverage-build-plan-2026-06.md).
**Owner:** Backend Architect.
**Closest analog:** [`sdk/python/src/spendguard/integrations/langchain.py`](../../../../sdk/python/src/spendguard/integrations/langchain.py) — Langflow components run LangChain `BaseChatModel` instances under the hood.

## 1. Problem

Langflow (DataStax, MIT, ~80k stars) is a visual no-code builder for LangChain Python flows. Operators drag-drop nodes; each LLM node holds a per-node `BaseChatModel` config (`ChatOpenAI`, `ChatAnthropic`, etc.). v1.8 (Mar 2026) added global model-provider config but per-node remains primary. Self-hosted Langflow against OpenAI / Anthropic has **no budget primitive**, no signed audit, no pre-call dollar gate.

D36 ships a Langflow **custom Python component** `SpendGuardChatModelWrapper` that accepts any LangChain `BaseChatModel` (drag-drop a `ChatOpenAI` node into the `inner` input) and wraps it with the **existing** `SpendGuardChatModel` from `sdk/python/src/spendguard/integrations/langchain.py`. The component renders as a canvas card; every downstream node sees a budget-gated model.

## 2. Goals

1. PyPI package `spendguard-langflow-component` (separate from `spendguard-sdk` because Langflow components must be vendored into `LANGFLOW_COMPONENTS_PATH`). Depends on `spendguard-sdk[langchain]>=0.5.1`.
2. Single Langflow `Component` subclass exposing one canvas card.
3. Inputs: `inner` (`HandleInput` typed `LanguageModel`), `sidecar_uds_path`, `tenant_id`, `budget_id`, `window_instance_id`, `unit_token_kind`, `model_family`, `claim_estimator_chars_per_token` (default 4).
4. Output: `LanguageModel` handle (same shape as `ChatOpenAI` output) so downstream nodes accept it identically.
5. Build method instantiates `SpendGuardClient`, runs `connect()` + `handshake()`, wraps `inner` in `SpendGuardChatModel`, returns the wrapped instance.
6. Demo `make demo-up DEMO_MODE=langflow_real` boots Langflow + sidecar + a 3-node flow; verifies ALLOW + DENY + audit row.
7. Docs page `docs/site/docs/integrations/langflow.md` with install + canvas screenshot.

## 3. Non-goals

- Wrapping non-`BaseChatModel` nodes (Embeddings, Tools, Memory). Budget gate fires at LLM call only.
- Token-by-token mid-stream cap — end-of-stream commit only (parity with `langchain.py` POC scope).
- Bundling a Langflow image. Ship the component; operators install it into their existing Langflow.
- Langflow Cloud (DataStax-hosted) marketplace push automation. PyPI is the install surface; Cloud push is follow-up.
- Per-flow budget IDs read from flow metadata — v1 reads from canvas component inputs only.
- Replacing Langflow's per-node `BaseChatModel` config UX. Operators still drop their `ChatOpenAI`, then connect to our wrapper.

## 4. Architecture

```
Langflow canvas
  ChatOpenAI node ──(LanguageModel handle)──┐
                                            ▼
   SpendGuardChatModelWrapper.build_model()
     1. SpendGuardClient(socket, tenant_id) → connect() + handshake()
     2. SpendGuardChatModel(inner=self.inner, client=..., budget/window/unit/...)
     3. install_autobind(wrapped, flow_id=self.graph.flow_id)
     4. return wrapped
                                            │
                                            ▼ (LanguageModel handle)
   downstream Langflow node → await wrapped.ainvoke(messages)
                                            ▼
   existing langchain.py: run_context → request_decision → inner._agenerate → emit_llm_call_post
                                            ▼
                              SpendGuard sidecar (UDS + mTLS)
```

The wrapper is a **thin Langflow adapter**: zero new reservation/commit logic. All gating lives in the existing SDK. D36 = packaging + component metadata + canvas UX + run-context auto-binding glue.

## 5. Key decisions

- **Reuse, don't reimplement.** Wrapper imports `SpendGuardChatModel` from `spendguard.integrations.langchain`. SDK bug fixes propagate.
- **Composition via `inner` HandleInput, not subclassing.** Operator drops a `ChatOpenAI`, connects its `LanguageModel` output into our `inner` input. Langflow's type system enforces compatibility.
- **Run-context auto-binding.** Langflow nodes call `ainvoke()` without `run_context(...)`. The build method **monkey-patches the returned wrapper's `_agenerate`** to enter `run_context` if no caller has bound one, using `self.graph.flow_id` (or a `uuid7` fallback). Caller-bound contexts always win (parity with existing langchain.py contract).
- **Separate PyPI package.** Langflow's loader walks `LANGFLOW_COMPONENTS_PATH` for files. Distributing as a vendor-drop tree avoids touching the SDK's import surface; SDK upgrades don't force re-vendoring. An `install_into_langflow` CLI copies the file into the target tree.
- **Credentials as canvas inputs, not env.** UDS path + tenant + budget IDs are canvas inputs (operator types them or pulls from Langflow global variables). `SPENDGUARD_SIDECAR_UDS` env-var fallback supported for headless deploys.
- **`HandleInput(input_types=["LanguageModel"])`.** The Langflow public protocol for accepting any model — same shape Langflow's own `Agent` component uses.
- **No streaming-specific code.** `SpendGuardChatModel._astream` (inherited from `BaseChatModel`) already routes through `_agenerate` for the SpendGuard pre/post path.
- **Fail-closed default.** Sidecar DEGRADE → `DecisionSkipped` raised by SDK; Langflow surfaces an error node. `SPENDGUARD_LANGFLOW_FAIL_OPEN=1` mirrors the per-integration escape-hatch convention.

## 6. Slice plan

| Slice | Title | Size |
|-------|-------|------|
| `COV_D36_S1_component_skeleton` | Package layout + `SpendGuardChatModelWrapper` class skeleton, no wiring | S |
| `COV_D36_S2_wrap_logic` | `build_model()` wires `SpendGuardClient` + `SpendGuardChatModel` + run-context auto-binding | M |
| `COV_D36_S3_metadata_yaml` | Langflow component metadata + PyPI `pyproject.toml` + install CLI | S |
| `COV_D36_S4_demo_mode` | `DEMO_MODE=langflow_real` + compose overlay + flow seed JSON + verify SQL | M |
| `COV_D36_S5_docs_publish` | `docs/site/docs/integrations/langflow.md` + canvas screenshot + PyPI publish workflow | S |

5 slices, ~750 LOC (~350 impl + 300 test + 100 yaml/docs/compose).

## 7. Open questions (locked at spec write)

1. **Langflow SDK floor:** `langflow>=1.8.0,<2.0.0`. v1.7 lacked stable `HandleInput` + `LanguageModel` handle.
2. **`inner` mutability:** Langflow re-invokes `build_model()` between flow runs; a per-run `SpendGuardClient` is acceptable (no connection pooling in v1).
3. **Global model provider config (v1.8+):** v1 D36 covers per-node wrapping only. Global-config interception deferred, tracked as a GH issue at merge.
4. **Run-context ID source:** prefer `self.graph.flow_id`. Fallback `uuid7()`. Caller-bound `run_context` always wins.
5. **Component vendoring vs pip-install:** pip-install drops the `.py` into site-packages; `spendguard-langflow-install --target` copies it to `$LANGFLOW_COMPONENTS_PATH`. Documented in docs page §Install.
