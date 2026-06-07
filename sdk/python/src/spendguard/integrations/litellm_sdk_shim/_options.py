"""Public option object for the LiteLLM SDK shim.

D12 ships a simpler operator-facing surface than D11's
``SpendGuardDirectAcompletion`` constructor: instead of asking the
operator to wire ``budget_resolver`` / ``claim_estimator`` /
``claim_reconciler`` factory callables, the shim asks for the four
fields a single-tenant single-budget operator actually has:

* ``client``    — already-constructed (and connected) ``SpendGuardClient``.
* ``tenant_id`` — same value the client was constructed with; surfaced
  here so the shim can ergonomically log + thread it without poking
  through ``client._tenant_id``.
* ``budget_id`` — optional; falls back to ``SPENDGUARD_BUDGET_ID``.
* ``fail_open`` — optional; matches ``SPENDGUARD_LITELLM_FAIL_OPEN=1``
  ergonomics from the D11 / direct-acompletion path.

The shim builds the ``BudgetBinding`` + default estimator / reconciler
internally. Operators who need full operator-controlled resolvers can
either pre-build a ``SpendGuardDirectAcompletion`` and call it directly
or drop down to the D11 ``SpendGuardLiteLLMCallback`` path.
"""

from __future__ import annotations

from dataclasses import dataclass
from typing import TYPE_CHECKING

if TYPE_CHECKING:
    from ...client import SpendGuardClient


@dataclass(frozen=True, slots=True)
class SpendGuardShimOptions:
    """Operator-facing config for ``install_shim``.

    Frozen so the install state machine can hash a stable signature for
    idempotent re-install (DESIGN §5).
    """

    client: SpendGuardClient
    tenant_id: str
    budget_id: str | None = None
    fail_open: bool = False

    def __post_init__(self) -> None:
        if not self.tenant_id:
            raise ValueError(
                "SpendGuardShimOptions.tenant_id is required (non-empty)."
            )
        # `client` is a runtime object; type-check at construction is
        # weak (mock clients in tests don't subclass), so we just
        # confirm truthiness here. Heavier checks happen in _core when
        # the client is actually invoked.
        if self.client is None:
            raise ValueError(
                "SpendGuardShimOptions.client is required (non-None)."
            )


__all__ = ["SpendGuardShimOptions"]
