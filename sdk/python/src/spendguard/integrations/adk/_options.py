"""Options dataclass for the Google ADK SpendGuard callback.

A thin, optional config bundle. The ADK callback's primary configuration
is passed through positional / keyword args on the callback constructor
(``client``, ``budget_id``, ``window_instance_id``, ``unit``,
``pricing``); this dataclass exists for users who want to keep the
non-proto, POCO-shaped knobs in one place (mirrors the MAF integration's
``SpendGuardAgentFrameworkOptions`` pattern).

Unlike the MAF options object, this is **not required** — the
``SpendGuardAdkCallback`` constructor accepts the same fields directly
for ergonomic parity with the LangChain / OpenAI Agents priors. Use
this dataclass when wiring the adapter via a config file or
``hydra`` / ``pydantic-settings``.
"""

from __future__ import annotations

from dataclasses import dataclass

from ._errors import SpendGuardConfigError


@dataclass(frozen=True, slots=True)
class SpendGuardAdkOptions:
    """Per-callback configuration POCO for the ADK integration.

    Attributes:
        tenant_id: SpendGuard tenant scope. REQUIRED — validated non-empty.
        budget_id: Budget the reservation debits. REQUIRED — validated non-empty.
        window_instance_id: Time-window scope on the budget.
            REQUIRED — validated non-empty.
        sidecar_socket_path: Path to the sidecar UDS. Default matches the
            project-wide convention.
        state_namespace: Prefix for the ``CallbackContext.state`` keys
            used by this adapter. Defaults to ``"spendguard"``; override
            only if running multiple SpendGuard callbacks in the same
            ADK runtime (rare).

    Raises:
        SpendGuardConfigError: any required string field empty / whitespace.
    """

    tenant_id: str
    budget_id: str
    window_instance_id: str
    sidecar_socket_path: str = "/var/run/spendguard/sidecar.sock"
    state_namespace: str = "spendguard"

    def __post_init__(self) -> None:
        """Validate required fields are non-empty."""
        if not self.tenant_id or not self.tenant_id.strip():
            raise SpendGuardConfigError(
                "SpendGuardAdkOptions.tenant_id must be a non-empty string."
            )
        if not self.budget_id or not self.budget_id.strip():
            raise SpendGuardConfigError(
                "SpendGuardAdkOptions.budget_id must be a non-empty string."
            )
        if not self.window_instance_id or not self.window_instance_id.strip():
            raise SpendGuardConfigError(
                "SpendGuardAdkOptions.window_instance_id must be a "
                "non-empty string."
            )
        if not self.sidecar_socket_path or not self.sidecar_socket_path.strip():
            raise SpendGuardConfigError(
                "SpendGuardAdkOptions.sidecar_socket_path must be a "
                "non-empty string."
            )
        if not self.state_namespace or not self.state_namespace.strip():
            raise SpendGuardConfigError(
                "SpendGuardAdkOptions.state_namespace must be a non-empty "
                "string."
            )


__all__ = [
    "SpendGuardAdkOptions",
]
