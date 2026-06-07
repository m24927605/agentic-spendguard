"""DSPy integration — gates ``dspy.LM`` calls via ``BaseCallback``.

DSPy (``dspy-ai`` >= 2.6) ships ``dspy.LM`` which routes through LiteLLM
by default. Two paths bypass the D12 LiteLLM shim:

  1. Custom ``dspy.LM`` subclasses overriding ``__call__`` to hit
     provider SDKs directly.
  2. Users who have NOT installed D12 but call ``dspy.Predict`` /
     ``dspy.ChainOfThought`` directly.

D21 closes both paths via ``dspy.utils.callback.BaseCallback``.
``on_lm_start(call_id, instance, inputs)`` fires before EVERY
``dspy.LM`` call regardless of routing; ``on_lm_end`` fires after.

Integration shape::

    import dspy
    from spendguard import SpendGuardClient
    from spendguard.integrations.dspy import (
        SpendGuardDSPyCallback, BudgetBinding,
    )
    from spendguard._proto.spendguard.common.v1 import common_pb2

    client = SpendGuardClient(socket_path=..., tenant_id=...)
    await client.connect()
    await client.handshake()

    def resolve(model_str: str) -> BudgetBinding:
        return BudgetBinding(
            budget_id="...", window_instance_id="...",
            unit=common_pb2.UnitRef(unit_id="usd_micros"),
            pricing=common_pb2.PricingFreeze(pricing_version="2026-q2"),
        )

    def reconcile(outputs):
        first = outputs[0] if outputs else None
        usage = getattr(first, "usage", {}) or {}
        return [common_pb2.BudgetClaim(
            budget_id="...", unit=unit,
            amount_atomic=str(usage.get("total_tokens", 100)),
            direction=common_pb2.BudgetClaim.DEBIT,
            window_instance_id="...")]

    cb = SpendGuardDSPyCallback(
        client=client, budget_resolver=resolve,
        claim_reconciler=reconcile,
    )
    dspy.configure(
        lm=dspy.LM("openai/gpt-4o-mini"),
        callbacks=[cb],  # MUST be FIRST in the list
    )

    # Every LM call inside dspy.Predict / dspy.ChainOfThought now
    # reserves before / commits after.
    qa = dspy.ChainOfThought("question -> answer")
    result = qa(question="What is 2+2?")

POC scope:
  - End-of-call commit only; intra-call streaming is not gated.
  - ``on_tool_*`` / ``on_module_*`` callbacks are not bound; tool spend
    rolls into the parent LM reservation. Reserved for D21.1.
  - DSPy 2.6 callbacks are sync. We dispatch via ``asyncio.run`` outside
    a running loop and raise ``SyncInAsyncContext`` when one is.
  - DENY raises ``DecisionDenied`` directly so DSPy's runtime surfaces
    it to the caller before the LM dispatch.
  - DEGRADE fails closed by default; ``SPENDGUARD_DSPY_FAIL_OPEN=1``
    permits the call to continue (dev only; no commit row will be
    produced).
"""

from __future__ import annotations

# Import-time guard: surface a helpful install hint when the user
# imports this module without the extras installed. Fires once at
# module load; the wrapper itself is import-resilient so the unit
# suite can load ``_wrapper`` directly via package-path bypass.
try:
    from dspy.utils.callback import BaseCallback  # noqa: F401
except ImportError as exc:  # pragma: no cover
    raise ImportError(
        "spendguard.integrations.dspy requires the [dspy] extra. "
        "Install with: pip install 'spendguard-sdk[dspy]'"
    ) from exc

from ..._litellm_shim import _IN_FLIGHT as _SHIM_IN_FLIGHT
from ._errors import (
    ApprovalRequired,
    DecisionDenied,
    DecisionSkipped,
    DecisionStopped,
    HandshakeError,
    SidecarUnavailable,
    SpendGuardConfigError,
    SpendGuardDegradeBlocked,
    SpendGuardError,
)
from ._options import (
    BudgetBinding,
    RunContext,
    SpendGuardDSPyOptions,
    _CallState,
)
from ._wrapper import (
    BudgetResolver,
    ClaimEstimator,
    ClaimReconciler,
    SpendGuardDSPyCallback,
    _PENDING,
    _PENDING_TTL_SECONDS,
)

__all__ = [
    # Primary callback class
    "SpendGuardDSPyCallback",
    # Type aliases for advanced configuration
    "BudgetResolver",
    "ClaimEstimator",
    "ClaimReconciler",
    # POCOs
    "BudgetBinding",
    "RunContext",
    "SpendGuardDSPyOptions",
    # Module-level state (test surface)
    "_PENDING",
    "_PENDING_TTL_SECONDS",
    "_SHIM_IN_FLIGHT",
    # Error re-exports (catch-from-one-place)
    "ApprovalRequired",
    "DecisionDenied",
    "DecisionSkipped",
    "DecisionStopped",
    "HandshakeError",
    "SidecarUnavailable",
    "SpendGuardConfigError",
    "SpendGuardDegradeBlocked",
    "SpendGuardError",
]
