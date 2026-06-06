# D24 — Implementation

**Reads:** [`design.md`](design.md), [`acceptance.md`](acceptance.md), [`review-standards.md`](review-standards.md).
**Touches:** Python SDK + demo orchestration + public docs. No Rust changes. No proto changes. No DB schema changes.

## 1. Module layout

```
sdk/python/src/spendguard/integrations/
├── openai_agents.py        # existing — RunContext / run_context REUSED, NOT modified
└── autogen.py              # NEW — D24 (~350 LOC)

sdk/python/pyproject.toml   # +`autogen` extra → `autogen-core>=0.4`

sdk/python/tests/integrations/
├── test_autogen.py         # NEW — unit tests with mock ChatCompletionClient (~300 LOC)
└── test_autogen_real.py    # NEW — integration tests parametrized over autogen-agentchat + ag2 (~200 LOC)

deploy/demo/autogen/        # NEW
└── bootstrap.py            # env-driven SpendGuardChatCompletionClient factory

deploy/demo/
├── Makefile                # +DEMO_MODE=agent_real_autogen / agent_real_ag2 branches
├── verify_step_autogen.sql # NEW — SQL gate (covers both modes; LINEAGE column asserted)
└── demo/run_demo.py        # +run_agent_real_autogen_mode(), +run_agent_real_ag2_mode()

docs/site/docs/integrations/
└── autogen-ag2.md          # NEW — public docs page covering both lineages
```

## 2. Slice breakdown

### Slice 1 — Module skeleton + extras + lineage probe (S)

**Files:** `sdk/python/src/spendguard/integrations/autogen.py` (new), `sdk/python/pyproject.toml` (edit).

```python
# sdk/python/src/spendguard/integrations/autogen.py
"""AutoGen 0.4+ / AG2 ChatCompletionClient wrap adapter.

Both AutoGen 0.4+ (Microsoft, maintenance mode) and AG2 (community
fork, ~48k stars Apache-2.0) share `autogen_core.models.ChatCompletionClient`
as the LLM abstraction. SpendGuard subclasses it and wraps `create()` /
`create_stream()` with reserve / call / commit.

Install with:

    pip install 'spendguard-sdk[autogen]'                  # base
    pip install autogen-agentchat>=0.4 autogen-ext[openai] # AutoGen lineage
    # OR
    pip install ag2>=0.7                                   # AG2 lineage

Integration shape:

    from autogen_ext.models.openai import OpenAIChatCompletionClient
    from autogen_agentchat.agents import AssistantAgent  # OR ag2.agents

    from spendguard import SpendGuardClient
    from spendguard.integrations.autogen import SpendGuardChatCompletionClient
    from spendguard.integrations.openai_agents import RunContext, run_context

    client = SpendGuardClient(socket_path=..., tenant_id=...)
    await client.connect(); await client.handshake()

    guarded = SpendGuardChatCompletionClient(
        inner=OpenAIChatCompletionClient(model="gpt-4o-mini"),
        client=client, budget_id=..., window_instance_id=...,
        unit=..., pricing=...,
        claim_estimator=lambda messages: [common_pb2.BudgetClaim(...)],
    )
    agent = AssistantAgent(name="x", model_client=guarded)

    async with run_context(RunContext(run_id="...")):
        result = await agent.on_messages([...], cancellation_token)
"""

from __future__ import annotations

import hashlib
from collections.abc import AsyncIterator, Callable
from typing import Any

from ..client import DecisionOutcome, SpendGuardClient
from ..ids import derive_idempotency_key, derive_uuid_from_signature
from .openai_agents import current_run_context  # REUSED

try:
    from autogen_core.models import (
        ChatCompletionClient as _ChatCompletionClient,
        CreateResult,
        LLMMessage,
        RequestUsage,
    )
except ImportError as exc:  # pragma: no cover
    raise ImportError(
        "spendguard.integrations.autogen requires the [autogen] extra. "
        "Install with: pip install 'spendguard-sdk[autogen]' AND one of "
        "`autogen-agentchat>=0.4` (Microsoft) or `ag2>=0.7` (community)."
    ) from exc

try:
    from spendguard._proto.spendguard.common.v1 import common_pb2
except ImportError as exc:  # pragma: no cover
    raise ImportError("spendguard proto stubs missing. Run `make proto`.") from exc


# Lineage probe — telemetry only, never branches business logic.
def _probe_lineage() -> str:
    has_autogen_agentchat = False
    has_ag2 = False
    try:
        import autogen_agentchat  # noqa: F401
        has_autogen_agentchat = True
    except ImportError:
        pass
    try:
        import ag2  # noqa: F401
        has_ag2 = True
    except ImportError:
        pass
    if has_autogen_agentchat and has_ag2:
        return "both"
    if has_ag2:
        return "ag2"
    if has_autogen_agentchat:
        return "autogen"
    return "core-only"  # autogen-core only — degenerate, still works


LINEAGE: str = _probe_lineage()


ClaimEstimator = Callable[[list[Any]], list[Any]]
"""Project BudgetClaim list from the messages payload."""
```

`pyproject.toml`:

```toml
[project.optional-dependencies]
autogen = [
  "autogen-core>=0.4",
]
```

### Slice 2 — `SpendGuardChatCompletionClient.create()` (M)

```python
def _signature(messages: list[Any], tools: Any, extra_create_args: dict[str, Any]) -> str:
    text = repr(messages) + "|" + repr(tools) + "|" + repr(sorted(extra_create_args.items()))
    return hashlib.blake2b(text.encode("utf-8"), digest_size=16).hexdigest()


class SpendGuardChatCompletionClient(_ChatCompletionClient):  # type: ignore[misc, valid-type]
    """AutoGen/AG2 ChatCompletionClient wrapper.

    Subclasses `autogen_core.models.ChatCompletionClient` and overrides
    `create` to insert PRE/POST sidecar hooks around the inner client.
    `create_stream` brackets the inner stream (POC scope — parity with
    SpendGuardAgentsModel.stream_response).
    """

    def __init__(
        self,
        *,
        inner: _ChatCompletionClient,
        client: SpendGuardClient,
        budget_id: str,
        window_instance_id: str,
        unit: Any,
        pricing: Any,
        claim_estimator: ClaimEstimator,
    ) -> None:
        # ChatCompletionClient is ABC with no shared state — skip super().__init__().
        self._inner = inner
        self._client = client
        self._budget_id = budget_id
        self._window_instance_id = window_instance_id
        self._unit = unit
        self._pricing = pricing
        self._claim_estimator = claim_estimator

    async def create(
        self,
        messages: list[Any],
        *,
        tools: Any = (),
        json_output: Any = None,
        extra_create_args: dict[str, Any] | None = None,
        cancellation_token: Any = None,
    ) -> Any:
        ctx = current_run_context()
        extra = dict(extra_create_args or {})
        signature = _signature(messages, tools, extra)
        llm_call_id = str(derive_uuid_from_signature(signature, scope="llm_call_id"))
        decision_id = str(derive_uuid_from_signature(signature, scope="decision_id"))
        step_id = f"{ctx.run_id}:autogen-call:{signature[:16]}"
        idempotency_key = derive_idempotency_key(
            tenant_id=self._client.tenant_id,
            session_id=self._client.session_id,
            run_id=ctx.run_id,
            step_id=step_id,
            llm_call_id=llm_call_id,
            trigger="LLM_CALL_PRE",
        )

        outcome: DecisionOutcome = await self._client.request_decision(
            trigger="LLM_CALL_PRE",
            run_id=ctx.run_id,
            step_id=step_id,
            llm_call_id=llm_call_id,
            tool_call_id="",
            decision_id=decision_id,
            route="llm.call",
            projected_claims=self._claim_estimator(messages),
            idempotency_key=idempotency_key,
        )

        try:
            result = await self._inner.create(
                messages,
                tools=tools,
                json_output=json_output,
                extra_create_args=extra,
                cancellation_token=cancellation_token,
            )
        except BaseException as exc:  # CancelledError + everything else
            if outcome.reservation_ids:
                outcome_kind = "CANCELLED" if type(exc).__name__ == "CancelledError" else "FAILURE"
                await self._client.emit_llm_call_post(
                    run_id=ctx.run_id, step_id=step_id, llm_call_id=llm_call_id,
                    decision_id=outcome.decision_id,
                    reservation_id=outcome.reservation_ids[0],
                    provider_reported_amount_atomic="",
                    estimated_amount_atomic="0",
                    unit=self._unit, pricing=self._pricing,
                    provider_event_id="", outcome=outcome_kind,
                )
            raise

        total_tokens = self._extract_total_tokens(result)
        if outcome.reservation_ids:
            await self._client.emit_llm_call_post(
                run_id=ctx.run_id, step_id=step_id, llm_call_id=llm_call_id,
                decision_id=outcome.decision_id,
                reservation_id=outcome.reservation_ids[0],
                provider_reported_amount_atomic="",
                estimated_amount_atomic=str(total_tokens),
                unit=self._unit, pricing=self._pricing,
                provider_event_id="", outcome="SUCCESS",
            )
        return result

    @staticmethod
    def _extract_total_tokens(result: Any) -> int:
        usage = getattr(result, "usage", None)
        if usage is None:
            return 0
        prompt = getattr(usage, "prompt_tokens", 0) or 0
        completion = getattr(usage, "completion_tokens", 0) or 0
        return int(prompt) + int(completion)
```

### Slice 3 — `create_stream()` + pass-through methods (S)

```python
    def create_stream(
        self, messages: list[Any], *, tools: Any = (), json_output: Any = None,
        extra_create_args: dict[str, Any] | None = None, cancellation_token: Any = None,
    ) -> AsyncIterator[Any]:
        # POC: stream from inner directly; PRE/POST fires at the create() boundary
        # when AssistantAgent eventually issues a non-streaming finalization call.
        # Tracked as follow-on (parity with openai_agents.stream_response).
        return self._inner.create_stream(
            messages, tools=tools, json_output=json_output,
            extra_create_args=extra_create_args, cancellation_token=cancellation_token,
        )

    # Pass-through introspection — required by AssistantAgent / token-budget caps.
    def actual_usage(self) -> Any: return self._inner.actual_usage()
    def total_usage(self) -> Any: return self._inner.total_usage()
    def count_tokens(self, messages: list[Any], *, tools: Any = ()) -> int:
        return self._inner.count_tokens(messages, tools=tools)
    def remaining_tokens(self, messages: list[Any], *, tools: Any = ()) -> int:
        return self._inner.remaining_tokens(messages, tools=tools)
    @property
    def capabilities(self) -> Any: return self._inner.capabilities
    @property
    def model_info(self) -> Any: return self._inner.model_info


__all__ = ["LINEAGE", "SpendGuardChatCompletionClient", "ClaimEstimator"]
```

### Slice 4 — Tests parametrized over both lineages

See `tests.md` §2.

### Slice 5 — Demo modes + docs

`Makefile` additions:

```make
else ifeq ($(DEMO_MODE),agent_real_autogen)
	@echo "[demo] DEMO_MODE=agent_real_autogen → AutoGen 0.4+ AssistantAgent wrapped via SpendGuardChatCompletionClient"
	$(MAKE) up_minimal
	docker compose run --rm \
	    --env SPENDGUARD_DEMO_MODE=agent_real_autogen \
	    demo python /workspace/run_demo.py
	$(MAKE) verify_autogen

else ifeq ($(DEMO_MODE),agent_real_ag2)
	@echo "[demo] DEMO_MODE=agent_real_ag2 → AG2 AssistantAgent wrapped via SpendGuardChatCompletionClient"
	$(MAKE) up_minimal
	docker compose run --rm \
	    --env SPENDGUARD_DEMO_MODE=agent_real_ag2 \
	    demo python /workspace/run_demo.py
	$(MAKE) verify_autogen
```

`verify_step_autogen.sql` asserts at least one `audit_outbox` row with `route='llm.call'`, `trigger='LLM_CALL_PRE'`, and a paired `LLM_CALL_POST` with non-zero `estimated_amount_atomic` within 60s of the PRE.

## 3. Public docs page

`docs/site/docs/integrations/autogen-ag2.md` — one page covering both lineages, with a decision table:

| If you use | Install | Integration import |
|------------|---------|--------------------|
| AutoGen 0.4+ (Microsoft) | `pip install 'spendguard-sdk[autogen]' autogen-agentchat autogen-ext[openai]` | `from spendguard.integrations.autogen import SpendGuardChatCompletionClient` |
| AG2 (community fork) | `pip install 'spendguard-sdk[autogen]' ag2` | (same import) |
| Routing via LiteLLM | D12 shim covers transitively | `spendguard.integrations.litellm_shim.install(...)` |
