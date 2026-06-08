"""Options + per-call state types for the DSPy SpendGuard callback.

Three dataclasses live here:

* ``BudgetBinding`` — what the operator's ``budget_resolver`` returns
  on each ``on_lm_start``. Carries the budget / unit / pricing tuple
  the reserve will book against. Mirrors AWS Strands' constructor-time
  binding except here the binding is per-call (resolved from the
  ``dspy.LM`` model string), which lets one callback gate multiple
  budgets keyed off model.
* ``RunContext`` — optional cross-framework run_id correlation. DSPy
  callbacks don't carry a stable run-scope identifier; this dataclass
  lets the operator bridge a parent LangChain / Strands / pydantic-ai
  ``run_id`` into the DSPy gate.
* ``_CallState`` — internal stash entry, keyed by DSPy's UUID
  ``call_id``. Holds the reservation companion identifiers between
  ``on_lm_start`` and the matching ``on_lm_end``.
* ``SpendGuardDSPyOptions`` — POCO wrapping the same knobs as the
  callback constructor's kwargs. Optional; for users who prefer to
  bundle config via a settings file. Parity with Strands' options
  pattern.
"""

from __future__ import annotations

import contextvars
import time
from dataclasses import dataclass, field
from typing import Any

from ._errors import SpendGuardConfigError


# ─────────────────────────────────────────────────────────────────────
# Public POCOs
# ─────────────────────────────────────────────────────────────────────


@dataclass(frozen=True, slots=True)
class BudgetBinding:
    """Per-call budget resolution returned by ``budget_resolver``.

    DSPy gates at the LM-call boundary so each call may target a
    different model and therefore a different budget — the resolver
    receives the LM's ``model`` string (e.g. ``"openai/gpt-4o-mini"``)
    and returns the budget_id / unit / pricing tuple to debit.

    Attributes:
        budget_id: Budget the reservation debits.
        window_instance_id: Time-window scope on the budget.
        unit: ``common_pb2.UnitRef`` describing the unit binding.
        pricing: ``common_pb2.PricingFreeze`` for ledger lookup.
    """

    budget_id: str
    window_instance_id: str
    unit: Any  # common_pb2.UnitRef
    pricing: Any  # common_pb2.PricingFreeze


@dataclass(frozen=True, slots=True)
class RunContext:
    """Optional cross-framework run-scope correlation.

    DSPy itself does NOT carry a stable run-scoped identifier on
    ``on_lm_start`` / ``on_lm_end`` (the ``call_id`` is per-LM-call).
    Operators who want to share a ``run_id`` with adjacent integrations
    (LangChain ``RunContext``, Strands ``StrandsRunContext``, etc.)
    can supply a ``run_context_factory`` that returns one of these.

    Attributes:
        run_id: Caller-minted run identifier. When provided via the
            factory, used as the ``RequestDecision.ids.run_id``. When
            omitted, the callback mints a fresh UUIDv7 per LM call.
    """

    run_id: str


@dataclass(frozen=True, slots=True)
class SpendGuardDSPyOptions:
    """Per-callback configuration POCO for the DSPy integration.

    Optional — the ``SpendGuardDSPyCallback`` constructor accepts the
    same fields directly. Use this dataclass when wiring the adapter
    via a config file or ``hydra`` / ``pydantic-settings``.

    Attributes:
        tenant_id: SpendGuard tenant scope. REQUIRED — validated.
        sidecar_socket_path: Path to the sidecar UDS. Default matches
            the project-wide convention.
        fail_closed: When ``True`` (the default) a sidecar DEGRADE or
            unavailability raises ``SpendGuardDegradeBlocked`` /
            ``SidecarUnavailable``. When ``False`` (dev only — also
            triggered by ``SPENDGUARD_DSPY_FAIL_OPEN=1``) the call
            proceeds without a reservation; commit will not fire.
        callback_first: Documentation knob — verified at construction
            and surfaced in the docstring so operators don't accidentally
            order the callback after a user observer that would bypass
            reserve.

    Raises:
        SpendGuardConfigError: any required string field empty.
    """

    tenant_id: str
    sidecar_socket_path: str = "/var/run/spendguard/sidecar.sock"
    fail_closed: bool = True
    callback_first: bool = True
    unit_id: str | None = None
    """Canonical-truth UUID of the ledger unit row.

    When provided, the callback threads it through to
    ``BudgetClaim.unit.unit_id`` on the wire so the sidecar ledger can
    resolve the budget claim. Most operators source this from the
    ``SPENDGUARD_UNIT_ID`` env var at adapter construction time.

    Omitting leaves the wire field empty and the ledger rejects the
    reserve with ``INVALID_REQUEST: claim[N].unit.unit_id empty`` —
    recipe-style integrations (no ledger reserve) MAY omit. NB: this
    is the ledger UUID, distinct from the free-form ``unit`` slug —
    they are NOT interchangeable.

    Additive optional field shipped under HARDEN_D05_UR (the Python
    SDK proto ``UnitRef.unit_id`` field already exists; this option
    threads it through the adapter's reserve path).
    """

    def __post_init__(self) -> None:
        if not self.tenant_id or not self.tenant_id.strip():
            raise SpendGuardConfigError(
                "SpendGuardDSPyOptions.tenant_id must be a non-empty string."
            )
        if not self.sidecar_socket_path or not self.sidecar_socket_path.strip():
            raise SpendGuardConfigError(
                "SpendGuardDSPyOptions.sidecar_socket_path must be a "
                "non-empty string."
            )


# ─────────────────────────────────────────────────────────────────────
# Internal stash entry (test surface — used by U06 / U13 / U14 etc.)
# ─────────────────────────────────────────────────────────────────────


@dataclass(slots=True)
class _CallState:
    """Per-LM-call stash entry, keyed by DSPy's UUID ``call_id``.

    Holds the reservation companion identifiers between the
    ``on_lm_start`` reserve and the matching ``on_lm_end`` commit/
    release. ``started_at`` carries the monotonic timestamp so the
    TTL sweep can drop stale entries when an ``on_lm_end`` is missed
    (e.g. provider crash before DSPy fires the end hook).

    ``shim_token`` is the contextvars token returned by
    ``_SHIM_IN_FLIGHT.set(True)`` so the matching ``on_lm_end`` can
    restore the previous value rather than dropping to ``False``
    (would clobber a sibling D12 frame's outer state).
    """

    decision_id: str
    reservation_id: str | None
    llm_call_id: str
    step_id: str
    run_id: str
    unit: Any  # common_pb2.UnitRef
    pricing: Any  # common_pb2.PricingFreeze
    inputs_signature: str
    estimator_amount_atomic: str
    model_str: str = ""
    started_at: float = field(default_factory=time.monotonic)
    shim_token: contextvars.Token[bool] | None = None


__all__ = [
    "BudgetBinding",
    "RunContext",
    "SpendGuardDSPyOptions",
    "_CallState",
]
