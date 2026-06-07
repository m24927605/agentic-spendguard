"""SpendGuard Langflow custom component — drag-drop budget gate.

Wraps any LangChain ``BaseChatModel`` (drag-dropped onto the canvas as
a ``ChatOpenAI`` / ``ChatAnthropic`` / etc. node) with the existing
SpendGuard LangChain integration. All reservation/commit logic lives in
``spendguard.integrations.langchain.SpendGuardChatModel`` -- this file
is adapter glue + Langflow component metadata only.

Per design.md (D36) §5 Key decisions:

- **Reuse, don't reimplement.** Wrapper imports the SDK's
  ``SpendGuardChatModel`` so SDK bug fixes propagate. The wrapper
  package never edits ``sdk/python/src/spendguard/integrations/langchain.py``.
- **Composition via ``inner`` HandleInput, not subclassing.** Operator
  drops a ``ChatOpenAI``, connects its ``LanguageModel`` output into
  our ``inner`` input. Langflow's type system enforces compatibility.
- **Run-context auto-bind.** Langflow flow runs call ``ainvoke()``
  without ``run_context(...)``. Auto-bind enters one using
  ``self.graph.flow_id`` (or a stable ``uuid4()`` fallback) when no
  caller has bound one. Caller-bound contexts ALWAYS win (INV-3).
- **Fail-closed default.** DEGRADE -> the SDK raises ``DecisionSkipped``
  which Langflow surfaces as an error node. ``SPENDGUARD_LANGFLOW_FAIL_OPEN=1``
  mirrors the per-integration escape-hatch convention.
"""

from __future__ import annotations

import os
from typing import Any

from ._version import __version__

# Langflow 1.8+ is the floor; the integration target uses the new
# ``Component`` base class (NOT the deprecated ``CustomComponent``).
# We surface a clear ImportError with install hint when missing.
try:
    from langflow.custom import Component  # type: ignore
    from langflow.inputs import (  # type: ignore
        HandleInput,
        IntInput,
        MessageTextInput,
        SecretStrInput,
    )
    from langflow.io import Output  # type: ignore
except ImportError as exc:  # pragma: no cover - install hint
    raise ImportError(
        "spendguard_langflow.component requires Langflow >=1.8.0. "
        "Install with: pip install 'langflow>=1.8.0,<2.0.0'"
    ) from exc


# Sentinel used by unit tests + the demo's docs to confirm version pin
# is in sync with pyproject.toml + metadata YAML (review-standards §3.2).
LANGFLOW_COMPONENT_VERSION = __version__


class SpendGuardChatModelWrapper(Component):
    """Drag-drop budget gate for any LangChain ``BaseChatModel``.

    Inputs are declared via Langflow's typed input system. The
    ``inner`` input is a ``HandleInput`` typed ``LanguageModel`` so any
    model node (``ChatOpenAI``, ``ChatAnthropic``, ``ChatVertexAI``,
    etc.) connects without type friction.

    Output is a single ``LanguageModel`` handle -- downstream Langflow
    nodes accept the wrapped model identically to a raw ``ChatOpenAI``.
    """

    display_name = "SpendGuard Budget Gate"
    description = (
        "Gates any LangChain chat model through a SpendGuard sidecar. "
        "Drop a model node (ChatOpenAI, ChatAnthropic, ...) into the "
        "'Inner Model' input. Downstream nodes see a budget-gated model "
        "that pre-reserves spend and commits real usage end-of-call."
    )
    icon = "shield"
    name = "SpendGuardChatModelWrapper"
    documentation = "https://spendguard.dev/docs/integrations/langflow"

    inputs = [
        HandleInput(
            name="inner",
            display_name="Inner Model",
            input_types=["LanguageModel"],
            required=True,
            info=(
                "The LangChain BaseChatModel this gate wraps. Connect a "
                "ChatOpenAI / ChatAnthropic / ChatVertexAI / etc. node "
                "here. The wrapper preserves the model's bind_tools() + "
                "with_structured_output() surface."
            ),
        ),
        MessageTextInput(
            name="sidecar_uds_path",
            display_name="SpendGuard Sidecar UDS Path",
            value="/run/spendguard/sidecar.sock",
            required=True,
            info=(
                "Filesystem path of the SpendGuard sidecar Unix domain "
                "socket. Falls back to env SPENDGUARD_SIDECAR_UDS if "
                "this canvas input is left blank."
            ),
        ),
        SecretStrInput(
            name="tenant_id",
            display_name="Tenant ID",
            required=True,
            info="Operator-issued tenant UUID. Pulls from Langflow globals.",
        ),
        MessageTextInput(
            name="budget_id",
            display_name="Budget ID",
            required=True,
            info="UUID of the SpendGuard budget this gate debits.",
        ),
        MessageTextInput(
            name="window_instance_id",
            display_name="Window Instance ID",
            required=True,
            info="UUID of the current rolling window the budget is bound to.",
        ),
        MessageTextInput(
            name="unit_token_kind",
            display_name="Unit Token Kind",
            value="output_token",
            advanced=True,
            info="Token kind for the BudgetClaim (output_token / total_token).",
        ),
        MessageTextInput(
            name="model_family",
            display_name="Model Family",
            value="gpt-4",
            advanced=True,
            info="Model family label baked into the UnitRef id (e.g. gpt-4, claude-3).",
        ),
        IntInput(
            name="claim_estimator_chars_per_token",
            display_name="Estimator chars/token",
            value=4,
            advanced=True,
            info=(
                "Heuristic divisor for the default chars/N estimator. "
                "4 matches the OpenAI rule-of-thumb; raise for verbose "
                "system prompts where chars overcount tokens."
            ),
        ),
    ]

    outputs = [
        Output(
            name="model",
            display_name="Gated Model",
            method="build_model",
            types=["LanguageModel"],
        ),
    ]

    # build_model is wired in Slice 2 — declared here so importing the
    # class doesn't crash Langflow's component loader.
    def build_model(self) -> Any:
        """Construct + return a wrapped ``SpendGuardChatModel``.

        Implemented by Slice 2 via :func:`_build_model_sync`. Declared
        here as a NotImplementedError so the skeleton import surface
        matches review-standards §1.7 (no silent ``None`` returns).
        """
        from ._build import build_model_sync

        return build_model_sync(self)


__all__ = [
    "LANGFLOW_COMPONENT_VERSION",
    "SpendGuardChatModelWrapper",
]
