"""Internal helper that wires ``estimator_for_model`` into each
integration's ``ClaimEstimator`` signature.

The five integrations (``litellm`` / ``langchain`` / ``pydantic_ai`` /
``openai_agents`` / ``agt``) each define a ``ClaimEstimator`` type
alias with a different call signature — they accept different
framework-specific payload shapes (e.g. LangChain's
``Sequence[BaseMessage]``, Pydantic-AI's
``(Sequence[ModelMessage], ModelSettings | None)``, AGT's
``Mapping[str, Any]``).

To keep the default-estimator wiring uniform across all five, this
module exposes one factory per integration that:

1. Looks up the right ``EstimatorFns`` for ``model`` via
   ``estimators.estimator_for_model``.
2. Extracts the framework-specific "messages" payload from the
   integration's call context.
3. Calls ``count_input_tokens`` + ``count_output_tokens_max``.
4. Wraps the totals into a single ``common_pb2.BudgetClaim`` matching
   the resolved ``BudgetBinding`` (budget_id / window / unit).

The returned callable is the per-integration ``ClaimEstimator`` and
slots in where the user would otherwise supply one.

Per spec §8.5 backward compat: when a caller passes
``claim_estimator=<their own>`` (anything other than ``None``), the
default factory is NOT invoked — the caller's estimator wins.

Strategy A reservation: per
``tokenizer-service-spec-v1alpha1.md`` §3.7 the reservation amount =
``input_tokens + output_tokens`` (tokens), or for cost units the
projector multiplies by ``price_per_token``. The SDK estimator emits
``amount_atomic = input_tokens + output_tokens`` and the sidecar
applies the price-unit conversion. This mirrors the
``crates/spendguard-tokenizer`` Strategy A formula.
"""

from __future__ import annotations

from collections.abc import Callable, Mapping, Sequence
from typing import Any

from spendguard._proto.spendguard.common.v1 import common_pb2

from ..estimators import EncoderKind, estimator_for_model


def _build_claim(
    *,
    budget_id: str,
    window_instance_id: str,
    unit: Any,
    amount_atomic: int,
    direction: int = common_pb2.BudgetClaim.DEBIT,
) -> Any:
    """Construct a single ``BudgetClaim`` matching the resolver binding."""
    return common_pb2.BudgetClaim(
        budget_id=budget_id,
        unit=unit,
        amount_atomic=str(amount_atomic),
        direction=direction,
        window_instance_id=window_instance_id,
    )


def _resolve_max_tokens(model_settings: Any) -> int | None:
    """Best-effort extraction of ``max_tokens`` from a framework's settings.

    Different frameworks expose different field names:
      * OpenAI API:  ``max_tokens`` / ``max_completion_tokens``
      * Anthropic:   ``max_tokens``  (required by API but optional in some SDK wrappers)
      * Gemini:      ``max_output_tokens`` / ``generation_config.max_output_tokens``
      * LangChain:   ``model_kwargs["max_tokens"]`` (sometimes nested)
      * LiteLLM:     ``data["max_tokens"]``

    Returns ``None`` if no value is set; the estimator's
    ``count_output_tokens_max`` then uses the family's default context
    window.
    """
    if model_settings is None:
        return None

    # Dict-like settings (LiteLLM data, LangChain kwargs, OpenAI raw)
    if isinstance(model_settings, Mapping):
        for key in ("max_tokens", "max_completion_tokens", "max_output_tokens"):
            val = model_settings.get(key)
            if isinstance(val, int) and val > 0:
                return val
        # Nested generation_config (Gemini SDK style)
        gen_cfg = model_settings.get("generation_config")
        if isinstance(gen_cfg, Mapping):
            val = gen_cfg.get("max_output_tokens")
            if isinstance(val, int) and val > 0:
                return val
        return None

    # Attribute-style settings (Pydantic-AI ModelSettings, openai-agents
    # ModelSettings — both are pydantic models with `max_tokens` field).
    for attr in ("max_tokens", "max_completion_tokens", "max_output_tokens"):
        val = getattr(model_settings, attr, None)
        if isinstance(val, int) and val > 0:
            return val
    return None


# ─────────────────────────────────────────────────────────────────────
# Integration-specific default estimator factories.
#
# Each takes (budget_id, window_instance_id, unit, model) and returns
# a Callable matching the integration's ClaimEstimator signature.
# ─────────────────────────────────────────────────────────────────────


def langchain_default_claim_estimator(
    *,
    budget_id: str,
    window_instance_id: str,
    unit: Any,
    model: str,
) -> Callable[[Sequence[Any]], list[Any]]:
    """LangChain ``ClaimEstimator = Callable[[Sequence[BaseMessage]], list[Any]]``.

    Takes the message list directly (no model_settings parameter
    in the LangChain hook — max_tokens is read off the bound model
    when available via the closed-over ``model`` string lookup).
    """
    fns = estimator_for_model(model)

    def estimator(messages: Sequence[Any]) -> list[Any]:
        input_tokens = fns.count_input_tokens(list(messages), model)
        # LangChain ChatModel doesn't expose max_tokens through this
        # callable surface (it's bound on the inner model). Use the
        # family default context window as a conservative cap; users
        # who need exact max_tokens behavior should supply their own
        # claim_estimator.
        output_tokens = fns.count_output_tokens_max(None, model)
        amount = input_tokens + output_tokens
        return [
            _build_claim(
                budget_id=budget_id,
                window_instance_id=window_instance_id,
                unit=unit,
                amount_atomic=amount,
            )
        ]

    return estimator


def pydantic_ai_default_claim_estimator(
    *,
    budget_id: str,
    window_instance_id: str,
    unit: Any,
    model: str,
) -> Callable[[Sequence[Any], Any], list[Any]]:
    """Pydantic-AI ``(Sequence[ModelMessage], ModelSettings | None) → list[BudgetClaim]``."""
    fns = estimator_for_model(model)

    def estimator(messages: Sequence[Any], model_settings: Any) -> list[Any]:
        input_tokens = fns.count_input_tokens(list(messages), model)
        max_tokens = _resolve_max_tokens(model_settings)
        output_tokens = fns.count_output_tokens_max(max_tokens, model)
        amount = input_tokens + output_tokens
        return [
            _build_claim(
                budget_id=budget_id,
                window_instance_id=window_instance_id,
                unit=unit,
                amount_atomic=amount,
            )
        ]

    return estimator


def openai_agents_default_claim_estimator(
    *,
    budget_id: str,
    window_instance_id: str,
    unit: Any,
    model: str,
) -> Callable[[Any], list[Any]]:
    """OpenAI Agents ``ClaimEstimator = Callable[[input_payload], list[BudgetClaim]]``.

    The input is the ``Runner.run(input=...)`` payload — either a
    string or a list of Items. The Agents Model surface doesn't pass
    model_settings to the estimator (it's on the wrapper-stored
    ``model_settings`` arg) so we use family default for max_tokens.
    Users that need exact max_tokens behavior should supply their own
    claim_estimator.
    """
    fns = estimator_for_model(model)

    def estimator(input_payload: Any) -> list[Any]:
        # input may be str or list of Items
        if isinstance(input_payload, str):
            messages = [{"role": "user", "content": input_payload}]
        elif isinstance(input_payload, list):
            messages = input_payload
        else:
            messages = [input_payload]
        input_tokens = fns.count_input_tokens(messages, model)
        output_tokens = fns.count_output_tokens_max(None, model)
        amount = input_tokens + output_tokens
        return [
            _build_claim(
                budget_id=budget_id,
                window_instance_id=window_instance_id,
                unit=unit,
                amount_atomic=amount,
            )
        ]

    return estimator


def litellm_default_claim_estimator(
    *,
    budget_id: str,
    window_instance_id: str,
    unit: Any,
    model: str,
) -> Callable[[Any], list[Any]]:
    """LiteLLM ``ClaimEstimator = Callable[[ResolverContext], list[BudgetClaim]]``.

    The ResolverContext exposes ``.data`` (the LiteLLM request dict),
    ``.user_api_key_dict``, ``.call_type``. We pull ``messages`` and
    ``max_tokens`` from ``.data`` — both are present in the standard
    LiteLLM proxy request payload.

    Note: the ``model`` closure-captured is the model passed at
    integration setup. If the LiteLLM request specifies a different
    model (model alias / fallback), ``data["model"]`` overrides the
    closure so the estimator dispatches to the right encoder.
    """
    # No upfront fns; we re-dispatch per call because ``data["model"]``
    # may differ from the setup-time `model` (LiteLLM aliases).

    def estimator(rctx: Any) -> list[Any]:
        # ResolverContext.data is the LiteLLM hook's `data: dict`
        data = getattr(rctx, "data", None) or {}
        effective_model = data.get("model") or model
        fns = estimator_for_model(effective_model)
        messages = data.get("messages") or []
        input_tokens = fns.count_input_tokens(list(messages), effective_model)
        max_tokens = data.get("max_tokens")
        output_tokens = fns.count_output_tokens_max(max_tokens, effective_model)
        amount = input_tokens + output_tokens
        return [
            _build_claim(
                budget_id=budget_id,
                window_instance_id=window_instance_id,
                unit=unit,
                amount_atomic=amount,
            )
        ]

    return estimator


def agt_default_claim_estimator(
    *,
    budget_id: str,
    window_instance_id: str,
    unit: Any,
    model: str,
) -> Callable[[Mapping[str, Any]], list[Any]]:
    """AGT ``ClaimEstimator = Callable[[Mapping[str, Any]], list[BudgetClaim]]``.

    The AGT payload is an arbitrary action/args mapping rather than
    a message list. For tool-only actions there's no LLM call to
    tokenise; we use a conservative chars-per-4 of the serialised
    payload as an upper bound (matches the Tier 3 fallback intent —
    "we don't know exactly, but we know roughly").

    For agent actions that include LLM call context (``"prompt"`` /
    ``"messages"`` keys), we tokenise via the dispatched estimator;
    otherwise we fall back to the payload serialisation length.
    """
    fns = estimator_for_model(model)

    def estimator(payload: Mapping[str, Any]) -> list[Any]:
        # Prefer explicit message list if present
        if "messages" in payload and isinstance(payload["messages"], list):
            messages = payload["messages"]
        elif "prompt" in payload:
            messages = [{"role": "user", "content": str(payload["prompt"])}]
        else:
            # Tool-only action: serialise the full payload as a
            # conservative input estimate.
            messages = [{"role": "user", "content": repr(dict(payload))}]
        input_tokens = fns.count_input_tokens(messages, model)
        output_tokens = fns.count_output_tokens_max(
            payload.get("max_tokens"), model
        )
        amount = input_tokens + output_tokens
        return [
            _build_claim(
                budget_id=budget_id,
                window_instance_id=window_instance_id,
                unit=unit,
                amount_atomic=amount,
            )
        ]

    return estimator


# Sentinel used by integrations to detect "user didn't pass a value vs
# user passed None on purpose". Per spec §8.5 backward compat both
# should default to the integration-built default estimator.
#
# Callers can still force the OLD behaviour ("require explicit
# estimator") by importing this sentinel and passing it explicitly —
# the integration MAY treat it as "no default applied", though no
# current integration uses this escape hatch.
_NO_DEFAULT = object()


__all__ = [
    "_NO_DEFAULT",
    "agt_default_claim_estimator",
    "langchain_default_claim_estimator",
    "litellm_default_claim_estimator",
    "openai_agents_default_claim_estimator",
    "pydantic_ai_default_claim_estimator",
]
