# D24 — AutoGen / AG2 `ChatCompletionClient` Wrap Adapter

**Status:** Spec — Tier 3, build plan `framework-coverage-build-plan-2026-06.md` §2.3.
**Owner:** AI Engineer. **Depends on:** D12 (LiteLLM SDK shim — transitive coverage for LiteLLM-routed clients).
**Sibling reference:** [`D19_google_adk`](../D19_google_adk/) (lifecycle callback adapter).
**Closest analog:** `spendguard.integrations.openai_agents` — `Model` subclass wrap with PRE/POST gating.

## 1. Problem

AutoGen 0.4+ (Microsoft, maintenance mode as of 2026-02) and AG2 (community fork led by ex-AutoGen maintainers, ~48k stars, Apache-2.0) **share the same `autogen_core.models.ChatCompletionClient` abstract base class**. Every `AssistantAgent`, `MagenticOneGroupChat`, and `Swarm` invokes the client through two methods:

- `async create(messages, *, tools, json_output, extra_create_args, cancellation_token) -> CreateResult`
- `async create_stream(messages, *, tools, ...) -> AsyncIterator[str | CreateResult]`

Both lineages ship vendor implementations (`OpenAIChatCompletionClient`, `AnthropicChatCompletionClient`, `AzureAIChatCompletionClient`) that wrap the provider SDK. Direct framework users instantiate one and hand it to `AssistantAgent(model_client=...)`.

There is **no callback or middleware hook** on `ChatCompletionClient`. SpendGuard cannot retrofit `add_middleware()` upstream. The only safe, version-stable surface is to **subclass `ChatCompletionClient`** and wrap an inner client, mirroring `SpendGuardAgentsModel` (D-shipped OpenAI Agents adapter).

D12 (LiteLLM SDK shim) provides transitive coverage when the inner client is `autogen_ext.models.litellm.LiteLLMChatCompletionClient`, but only ~20% of AutoGen/AG2 production deployments route via LiteLLM (per Trend Researcher 2026-06). For the OpenAI/Anthropic/Azure-direct majority, D24 is the only enforcement path.

## 2. Goals

1. Public class `SpendGuardChatCompletionClient(ChatCompletionClient)` in `spendguard.integrations.autogen` covering BOTH AutoGen 0.4+ and AG2 (single import path; runtime detection of which lineage is loaded).
2. Wraps `create()` and `create_stream()` with reserve-before / commit-after / release-on-exception identical in semantics to `SpendGuardAgentsModel`.
3. Extras: `spendguard-sdk[autogen]` resolves to `autogen-core>=0.4` (the shared base) and is compatible with either `autogen-agentchat>=0.4` OR `ag2>=0.7` installed alongside.
4. Demo modes: `agent_real_autogen` and `agent_real_ag2` — same workload, distinct `pip install` set, both produce identical audit ledger rows.
5. Tests parametrized over both lineages (`pytest.mark.parametrize("lineage", ["autogen", "ag2"])`) — load-bearing acceptance gate.
6. Reuses shared `RunContext` + `run_context()` from `spendguard.integrations.openai_agents` so a polyglot agent stack (OpenAI Agents → AutoGen → Pydantic-AI in one run) shares a single trace.
7. Docs page `docs/site/docs/integrations/autogen-ag2.md` distinguishing the two lineages with a single integration recipe.

## 3. Non-goals

- Per-chunk gating inside `create_stream()`. Stream gating brackets the whole stream at the model boundary (parity with OpenAI Agents stream POC); intra-stream tool calls inherit the parent reservation.
- Wrapping `count_tokens()` / `total_usage()` / `remaining_tokens()` introspection methods — pass-through to inner.
- AG2-specific extensions (e.g. AG2's `register_for_llm` decorator) — those are AG2-only and orthogonal to the LLM gate.
- Microsoft AGT integration (D7 / already shipped via `spendguard.integrations.agt`). AGT is a separate framework, not AutoGen.

## 4. Architecture

```
AssistantAgent.on_messages(...)
  → SpendGuardChatCompletionClient.create(messages, tools, ...)
      ├─ ctx = current_run_context()           [reused from openai_agents]
      ├─ signature = blake2b(messages | tools | extra_create_args)
      ├─ llm_call_id / decision_id derived from signature
      ├─ sidecar.RequestDecision(LLM_CALL_PRE, projected_claims)
      │     ALLOW = continue · DENY/DEGRADE = raise (fail-closed)
      ├─ inner.create(messages, tools, ...)    [provider HTTP]
      └─ sidecar.emit_llm_call_post(SUCCESS|FAILURE|CANCELLED,
                                    estimated=usage.completion_tokens)
```

`CreateResult.usage` exposes `RequestUsage(prompt_tokens, completion_tokens)` in both lineages (verified against `autogen-core 0.4.0` and `ag2 0.7.0` — identical dataclass). Total-token extraction mirrors `_extract_total_tokens` from `openai_agents.py:294-308`.

## 5. Key decisions

- **Single module covers both lineages.** Both AutoGen 0.4+ and AG2 re-export `ChatCompletionClient` from `autogen_core.models` (AG2 vendored the namespace unchanged). One subclass works against either. Runtime split detected by import probe at module load — recorded in `_LINEAGE` constant for telemetry, never branches business logic.
- **Composition over inheritance for the inner client.** Constructor takes `inner: ChatCompletionClient`. We never instantiate the provider SDK ourselves — matches OpenAI Agents adapter pattern.
- **No `__init__` super call.** `ChatCompletionClient` is an ABC with no shared state; identical to D-shipped OpenAI Agents POC pattern.
- **Reuse `RunContext` / `run_context()` from `openai_agents`** instead of duplicating. Polyglot agent stacks share one trace.
- **`create_stream()` returns inner stream directly** (POC scope per OpenAI Agents parity). Bracketed PRE/POST fires at the `create()` boundary; tool calls inherit. Per-chunk gating tracked as follow-on.
- **`CancelledError` → `outcome=CANCELLED`** in POST (matches D12 shim). Other exceptions → `outcome=FAILURE` + re-raise.
- **No default `claim_estimator`.** Operator MUST pass one. Future SLICE_12-style default dispatch is out of scope (the inner client's `model` attribute is not standardized across vendor implementations in AutoGen 0.4 — `OpenAIChatCompletionClient.model` exists, `AnthropicChatCompletionClient` uses `_model_name`).
- **`spendguard-sdk[autogen]` extra resolves `autogen-core` only.** Operator picks `autogen-agentchat` OR `ag2` themselves — we don't pin the lineage.

## 6. Slice plan

| Slice | Title | Size |
|-------|-------|------|
| `COV_D24_S1_module_skeleton` | Module skeleton + `[autogen]` extra + lineage probe + ImportError contract | S |
| `COV_D24_S2_wrap_create` | `SpendGuardChatCompletionClient.create()` PRE/POST with full signature passthrough | M |
| `COV_D24_S3_wrap_stream_and_passthrough` | `create_stream()` bracket + `count_tokens` / `total_usage` / `actual_usage` / `remaining_tokens` / `capabilities` / `model_info` pass-through | S |
| `COV_D24_S4_tests_parametrized` | Unit + integration tests parametrized over AutoGen + AG2 (~20 tests) | M |
| `COV_D24_S5_demos_and_docs` | `agent_real_autogen` + `agent_real_ag2` demo modes + Makefile + verify SQL + integration docs page | M |

5 slices, S/M only, ~1100 LOC total (~350 impl + 500 test + 250 docs/yaml/demo).

## 7. Interfaces

```python
from spendguard.integrations.autogen import (
    SpendGuardChatCompletionClient,
    LINEAGE,  # "autogen" | "ag2" | "both"
)
# Reuses RunContext / run_context from openai_agents
from spendguard.integrations.openai_agents import RunContext, run_context

guarded = SpendGuardChatCompletionClient(
    inner=OpenAIChatCompletionClient(model="gpt-4o-mini"),
    client=spendguard_client,
    budget_id=...,
    window_instance_id=...,
    unit=...,
    pricing=...,
    claim_estimator=lambda messages: [common_pb2.BudgetClaim(...)],
)
agent = AssistantAgent(name="x", model_client=guarded)  # AutoGen or AG2
```

Full operator code sample in `implementation.md` §2.

## 8. Open questions (locked)

1. **AG2 lineage divergence risk:** locked — both lineages re-export `autogen_core.models.ChatCompletionClient` unchanged through at least AG2 0.7.x. If AG2 forks the ABC in a future release, D24 will pin a max version and ship D24.1.
2. **`extra_create_args` signature variance:** locked — pass-through opaque dict. Hash via `repr()` for signature derivation.
3. **`tools` parameter inclusion in signature:** locked — included. Different tool sets = different reservations.
