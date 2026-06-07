"""Build-time wiring for :class:`SpendGuardChatModelWrapper`.

Implements the synchronous ``build_model()`` Langflow contract. Calls
``connect()`` + ``handshake()`` on a fresh :class:`SpendGuardClient`,
constructs the SDK's :class:`SpendGuardChatModel`, installs the
run-context auto-bind + decision-context tagging glue, and returns the
wrapped model.

Per design.md §5 the build is per-invocation; multi-flow / multi-tenant
isolation is guaranteed by NOT caching the client at module level.
"""

from __future__ import annotations

import asyncio
import os
from typing import Any

from ._decision_context import install_decision_context
from ._run_context import install_autobind


_DEFAULT_ESTIMATOR_FLOOR = 50
"""Minimum projected tokens per estimator call.

Matches the docstring example in
``sdk/python/src/spendguard/integrations/langchain.py:118-135`` so the
default wrapper estimator mirrors the SDK's published heuristic.
"""


def _resolve_uds(canvas_input: str | None) -> str:
    """Pick the sidecar UDS path -- canvas first, env fallback.

    Raises:
        ValueError: if both canvas input AND ``SPENDGUARD_SIDECAR_UDS``
            env are empty. The message names BOTH (review-standards §2.4).
    """
    if canvas_input:
        return canvas_input
    env_value = os.environ.get("SPENDGUARD_SIDECAR_UDS")
    if env_value:
        return env_value
    raise ValueError(
        "SpendGuard sidecar UDS not configured. Set the canvas input "
        "'SpendGuard Sidecar UDS Path' or env SPENDGUARD_SIDECAR_UDS."
    )


def _build_estimator(
    *,
    budget_id: str,
    window_instance_id: str,
    unit_ref: Any,
    chars_per_token: int,
) -> Any:
    """Build the default chars/N estimator closure.

    Mirrors the example in ``langchain.py:118-135``. Uses
    ``max(50, chars // chars_per_token)`` so a single short message
    still produces a non-degenerate reservation.
    """
    from spendguard._proto.spendguard.common.v1 import common_pb2

    def estimator(messages: Any) -> list[Any]:
        chars = sum(len(getattr(m, "content", "")) for m in messages)
        projected = max(_DEFAULT_ESTIMATOR_FLOOR, chars // chars_per_token)
        return [
            common_pb2.BudgetClaim(
                budget_id=budget_id,
                unit=unit_ref,
                amount_atomic=str(projected),
                direction=common_pb2.BudgetClaim.DEBIT,
                window_instance_id=window_instance_id,
            )
        ]

    return estimator


async def _build_async(component: Any) -> Any:
    """Build the wrapped model. Called from ``build_model_sync``.

    Args:
        component: the :class:`SpendGuardChatModelWrapper` instance.
            Attribute access pulls inputs the same way Langflow's
            component runtime does.
    """
    from spendguard import SpendGuardClient
    from spendguard._proto.spendguard.common.v1 import common_pb2
    from spendguard.integrations.langchain import SpendGuardChatModel

    uds = _resolve_uds(getattr(component, "sidecar_uds_path", None))
    tenant_id = getattr(component, "tenant_id", None)
    if not tenant_id:
        raise ValueError(
            "SpendGuard tenant_id not configured. Set the canvas input "
            "'Tenant ID' or pull from a Langflow global variable."
        )
    budget_id = getattr(component, "budget_id", None)
    window_instance_id = getattr(component, "window_instance_id", None)
    if not budget_id or not window_instance_id:
        raise ValueError(
            "SpendGuard budget_id + window_instance_id must both be set "
            "on the canvas component before build."
        )

    unit_token_kind = getattr(component, "unit_token_kind", None) or "output_token"
    model_family = getattr(component, "model_family", None) or "gpt-4"

    # Per review-standards §2.6: default floor 50 + chars_per_token at
    # least 1 (zero would divide-by-zero on a long prompt).
    raw_cpt = getattr(component, "claim_estimator_chars_per_token", None) or 4
    chars_per_token = max(1, int(raw_cpt))

    unit_ref = common_pb2.UnitRef(
        unit_id=f"{model_family}.{unit_token_kind}",
        token_kind=unit_token_kind,
        model_family=model_family,
    )

    client = SpendGuardClient(socket_path=uds, tenant_id=tenant_id)
    await client.connect()
    await client.handshake()

    # decision_context_json tagging — every request_decision call gets
    # integration=langchain + source=langflow folded in so the audit
    # chain distinguishes Langflow runs.
    install_decision_context(client)

    estimator = _build_estimator(
        budget_id=budget_id,
        window_instance_id=window_instance_id,
        unit_ref=unit_ref,
        chars_per_token=chars_per_token,
    )

    inner = getattr(component, "inner", None)
    if inner is None:
        raise ValueError(
            "SpendGuard wrapper 'Inner Model' input is empty. Drop a "
            "ChatOpenAI / ChatAnthropic node onto the canvas and "
            "connect its LanguageModel output to this gate's Inner "
            "Model input."
        )

    wrapped = SpendGuardChatModel(
        inner=inner,
        client=client,
        budget_id=budget_id,
        window_instance_id=window_instance_id,
        unit=unit_ref,
        pricing=common_pb2.PricingFreeze(),
        claim_estimator=estimator,
    )

    flow_id = getattr(getattr(component, "graph", None), "flow_id", None)
    install_autobind(wrapped, flow_id=flow_id)
    return wrapped


def build_model_sync(component: Any) -> Any:
    """Synchronous Langflow ``build_model`` entrypoint.

    Langflow 1.8's ``Component.build_model`` contract is sync. We bridge
    to the async client lifecycle via ``asyncio.run`` -- and guard
    against being called inside a running loop (which would deadlock).
    """
    try:
        running = asyncio.get_running_loop()
    except RuntimeError:
        running = None

    if running is not None and running.is_running():
        # Defensive: Langflow's build phase is sync but a future
        # version (or third-party harness) might invoke from an async
        # context. Better to fail fast with a clear hint than deadlock.
        raise RuntimeError(
            "SpendGuardChatModelWrapper.build_model() must be called "
            "outside a running event loop. Langflow's build phase is "
            "sync; if you see this error, file a bug on the SpendGuard "
            "Langflow component with your Langflow version."
        )

    return asyncio.run(_build_async(component))


__all__ = ["build_model_sync"]
