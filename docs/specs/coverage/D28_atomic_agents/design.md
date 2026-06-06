# D28 — Atomic Agents Instructor Client Wrap Adapter

**Status:** Spec — Tier 3, build plan `framework-coverage-build-plan-2026-06.md` §2.3.
**Owner:** AI Engineer.
**Closest analog:** `spendguard.integrations.openai_agents` — Model/client-wrap with PRE/POST gating. **Sibling reference:** [`D26_letta`](../D26_letta/).

## 1. Problem

Atomic Agents (BrainBlend AI, ~6k stars, MIT) is Pydantic-first, built on **Instructor** (Jason Liu). `BaseAgent` is constructed via `BaseAgentConfig(client=<instructor_client>, model=..., input_schema=..., output_schema=...)`. At run time, `agent.run(...)` calls `self.client.chat.completions.create_with_completion(response_model=output_schema, ...)` (or `.create(...)` on older paths). Instructor patches the underlying provider SDK; `instructor.from_openai(openai.OpenAI(...))` returns an `Instructor` whose `.chat.completions.create*` understands `response_model=` and internally performs **retry-on-validation-error** loops — each retry is a fresh provider HTTP call.

No first-class LLM-call middleware exists. Two gate-points:

| Candidate | Coverage | Verdict |
|-----------|----------|---------|
| Wrap raw provider SDK before `instructor.from_openai(...)` | First call only; validation retries bypass the wrap (Instructor calls its patched method, not the raw transport) | **Rejected** |
| Wrap the **Instructor object** — patch `.chat.completions.create` / `.create_with_completion` | Every call, retries included | **Adopted** |

## 2. Goals

1. Public helper `wrap_instructor_client(client, *, spendguard_client, ...) -> Instructor`-compatible proxy in `spendguard.integrations.atomic_agents`, covering both sync (`Instructor`) and async (`AsyncInstructor`).
2. Wraps `.chat.completions.create` AND `.chat.completions.create_with_completion`. Every call — including Instructor's validation retries — fires reserve / call / commit.
3. Extras: `spendguard-sdk[atomic-agents]` resolves to `atomic-agents>=1.0,<2.0` and `instructor>=1.5,<2.0`.
4. Demo mode `agent_real_atomic_agents` exercising `BaseAgent` with a Pydantic `output_schema`, gated through SpendGuard; deny path asserts zero provider HTTP.
5. Public docs page explaining the Instructor-wrap rationale and contrasting with the rejected raw-SDK wrap.
6. Reuses shared `RunContext` + `run_context()` from `spendguard.integrations.openai_agents` for polyglot trace sharing.

## 3. Non-goals

- Patching `BaseAgent` directly — surface churns per release.
- Wrapping Instructor's `Mode` selection logic — Instructor's concern.
- Wrapping `client.messages.create` (Anthropic-native Instructor surface) — Atomic Agents documents `chat.completions`.
- An Atomic Agents `Hook`-system PR upstream — none exists; not critical path.
- Instructor streaming (`Partial[...]` / `Iterable[...]`) — POC scope deferred.

## 4. Architecture

```
BaseAgent.run(...)
  → self.client.chat.completions.create_with_completion(
        model=..., messages=..., response_model=output_schema, ...)
    → SpendGuardInstructorProxy.create_with_completion(...)
        ├─ ctx = current_run_context()           [reused from openai_agents]
        ├─ signature = blake2b(messages | response_model.__name__ | model | tools)
        ├─ sidecar.RequestDecision(LLM_CALL_PRE, projected_claims)
        │     ALLOW = continue · DENY = raise (fail-closed before HTTP)
        ├─ inner.chat.completions.create_with_completion(...)   [provider HTTP via Instructor]
        │     (Instructor's internal retries re-enter this proxy → each gets its own reservation)
        └─ sidecar.emit_llm_call_post(SUCCESS|FAILURE|CANCELLED,
                                      estimated=raw_completion.usage.total_tokens)
```

`create_with_completion` returns `tuple[ParsedModel, ChatCompletion]`. The raw `ChatCompletion.usage.total_tokens` is the canonical cost source. For `.create()` (parsed-only) we read usage from the `_raw_response` attribute Instructor stores on the parsed model (validated against `instructor==1.5.2`).

## 5. Key decisions

- **Wrap the Instructor object, not the raw provider SDK.** Covers validation retries.
- **Composition + `__getattr__` delegation, no subclass.** `Instructor`/`AsyncInstructor` use heavy `__init_subclass__` machinery and accept private kwargs.
- **Override `create` AND `create_with_completion`.** Atomic Agents 1.0+ uses the latter; older code uses the former.
- **Sync + async dual wrapping via inner-type detection.** Runtime check on `instructor.Instructor` vs `instructor.AsyncInstructor`; one factory dispatches.
- **Each Instructor retry gets its own reservation.** Empirically against `instructor==1.5.2`, each retry mutates `messages` (validation error injected) → signature naturally diverges → fresh `llm_call_id`. Tested behavior, not a knob.
- **Reuse `RunContext` from `openai_agents`.** Polyglot stacks share one trace.
- **No default `claim_estimator`.** Instructor's polyglot routing makes any single default wrong.
- **Proxy is NOT an `Instructor` subclass.** Atomic Agents duck-types on `client.chat.completions.create`; the proxy satisfies that without inheritance.

## 6. Slice plan

| Slice | Title | Size |
|-------|-------|------|
| `COV_D28_S1_module_skeleton` | Module skeleton + `[atomic-agents]` extra + ImportError contract + `wrap_instructor_client` factory + sync/async dispatch | S |
| `COV_D28_S2_create_with_completion` | `create_with_completion()` + `create()` PRE/POST sync + async, usage extraction from raw `ChatCompletion` | M |
| `COV_D28_S3_tests` | Unit + integration tests (real Atomic Agents `BaseAgent` + Instructor + `pytest-httpx`) + deny-path zero-HTTP + Instructor-retry reservation-per-attempt | M |
| `COV_D28_S4_demo_and_docs` | `agent_real_atomic_agents` demo mode + Makefile + verify SQL + integration docs page | M |

4 slices, S/M only, ~1100 LOC total (~350 impl + 500 test + 250 docs/yaml/demo).

## 7. Interfaces

```python
import instructor
from openai import OpenAI
from atomic_agents.agents.base_agent import BaseAgent, BaseAgentConfig
from pydantic import BaseModel

from spendguard import SpendGuardClient
from spendguard.integrations.atomic_agents import wrap_instructor_client
from spendguard.integrations.openai_agents import RunContext, run_context

class Answer(BaseModel):
    final: str

raw_instructor = instructor.from_openai(OpenAI())
guarded = wrap_instructor_client(
    raw_instructor,
    spendguard_client=client,
    budget_id=..., window_instance_id=...,
    unit=..., pricing=...,
    claim_estimator=lambda kwargs: [common_pb2.BudgetClaim(...)],
)

agent = BaseAgent(BaseAgentConfig(
    client=guarded, model="gpt-4o-mini",
    system_prompt_generator=..., input_schema=..., output_schema=Answer,
))

async with run_context(RunContext(run_id="...")):
    result = agent.run({"query": "What's 2+2?"})
```

Full operator sample in `implementation.md` §2.

## 8. Open questions (locked)

1. **Instructor 1.x churn:** `create_with_completion` stable since 1.3.x. Pin `instructor>=1.5,<2.0`.
2. **Atomic Agents 1.x churn:** `BaseAgentConfig(client=...)` stable since 1.0.0. Pin `atomic-agents>=1.0,<2.0`.
3. **Streaming:** out-of-scope POC, follow-on parallel to `openai_agents.stream_response`.
4. **Anthropic-native messages surface:** out-of-scope; Atomic Agents documents `chat.completions`.
5. **Validation-retry reservations:** each retry gets its own reservation via natural signature divergence. Tested, not configured.
