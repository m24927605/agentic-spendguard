"""Options dataclasses for the LlamaIndex SpendGuard handler.

Thin POCO bundles. The handler's primary configuration is passed through
keyword args on the constructor (``client``, ``budget_id``,
``window_instance_id``, ``unit``, ``pricing``); this dataclass exists for
users who want to keep the non-proto, POCO-shaped knobs in one place
(mirrors the ADK / Strands / AutoGen integrations' ``Options`` pattern).

LlamaIndex's per-event ``event_id`` is the cross-call correlation key,
so a ``RunContext`` is NOT required — the handler keys its stash off
``event_id`` directly. ``LlamaIndexRunContext`` is provided for callers
bridging a LlamaIndex query to a cross-framework ``run_id`` (e.g. a
parent LangChain run wrapping a RAG query engine).
"""

from __future__ import annotations

from dataclasses import dataclass

from ._errors import SpendGuardConfigError


@dataclass(frozen=True, slots=True)
class SpendGuardLlamaIndexOptions:
    """Per-handler configuration POCO for the LlamaIndex integration.

    Attributes:
        tenant_id: SpendGuard tenant scope. REQUIRED — validated non-empty.
        budget_id: Budget the reservation debits. REQUIRED — validated non-empty.
        window_instance_id: Time-window scope on the budget.
            REQUIRED — validated non-empty.
        sidecar_socket_path: Path to the sidecar UDS. Default matches the
            project-wide convention.

    Raises:
        SpendGuardConfigError: any required string field empty / whitespace.
    """

    tenant_id: str
    budget_id: str
    window_instance_id: str
    sidecar_socket_path: str = "/var/run/spendguard/sidecar.sock"

    def __post_init__(self) -> None:
        """Validate required fields are non-empty."""
        if not self.tenant_id or not self.tenant_id.strip():
            raise SpendGuardConfigError(
                "SpendGuardLlamaIndexOptions.tenant_id must be a non-empty string."
            )
        if not self.budget_id or not self.budget_id.strip():
            raise SpendGuardConfigError(
                "SpendGuardLlamaIndexOptions.budget_id must be a non-empty string."
            )
        if not self.window_instance_id or not self.window_instance_id.strip():
            raise SpendGuardConfigError(
                "SpendGuardLlamaIndexOptions.window_instance_id must be a "
                "non-empty string."
            )
        if not self.sidecar_socket_path or not self.sidecar_socket_path.strip():
            raise SpendGuardConfigError(
                "SpendGuardLlamaIndexOptions.sidecar_socket_path must be a "
                "non-empty string."
            )


@dataclass(frozen=True, slots=True)
class LlamaIndexRunContext:
    """Optional run-scope correlation context.

    LlamaIndex's callback manager carries a per-event ``event_id`` and an
    optional ``parent_id`` end-to-end, so a ``RunContext`` is *not*
    required (unlike LangChain / Pydantic-AI where the user must bind
    one before invocation).

    Provide this when bridging a LlamaIndex query to a cross-framework
    ``run_id`` (e.g. a parent LangChain run wrapping the query engine).
    Set on a per-handler basis via the ``run_id_fn`` constructor arg —
    the handler invokes ``run_id_fn(payload)`` first; if it returns a
    non-empty string, that wins over ``trace_id`` and ``parent_id``.

    Attributes:
        run_id: Caller-minted run identifier. When set, the handler
            uses it as the ``RequestDecision.ids.run_id`` instead of
            deriving from the LlamaIndex ``trace_id`` / ``parent_id``.
    """

    run_id: str


@dataclass(slots=True)
class _PendingCall:
    """Stash entry, keyed by LlamaIndex ``event_id``.

    Holds the reservation companion identifiers between the
    ``on_event_start`` reserve and the matching ``on_event_end``
    commit. Mutable so future POC scope (e.g. attaching per-call
    tracing metadata) doesn't break the dataclass API.
    """

    reservation_id: str
    decision_id: str
    step_id: str
    llm_call_id: str
    run_id: str
    signature: str


__all__ = [
    "LlamaIndexRunContext",
    "SpendGuardLlamaIndexOptions",
    "_PendingCall",
]
