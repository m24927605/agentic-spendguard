"""Internal: shared ``_IN_FLIGHT`` contextvar bridging D21 (DSPy adapter)
and D12 (LiteLLM SDK shim).

When BOTH ``spendguard.integrations.dspy`` (D21) AND
``spendguard.integrations.litellm_shim`` (D12) are installed in the same
interpreter, a single ``dspy.LM(...)`` call would otherwise trigger two
reservations: once from D21's ``on_lm_start`` (the BaseCallback hook
fires before the LM's HTTP) and again from D12's ``acompletion`` wrapper
(LiteLLM's own dispatcher fires when DSPy routes through it).

The shared ``_IN_FLIGHT: ContextVar[bool]`` lives here, in a module that
both D21 and D12 import, so:

  1. D21's ``on_lm_start`` calls ``_IN_FLIGHT.set(True)`` BEFORE
     dispatching its ``RequestDecision`` RPC.
  2. D12's wrapper checks ``_IN_FLIGHT.get()`` at entry; when ``True``,
     it short-circuits to the original ``acompletion`` (D21 owns the
     reservation).
  3. D21's ``on_lm_end`` resets the contextvar via the captured token.

The module is intentionally tiny so D21-only installs do not pay for
the LiteLLM extras transitively, and D12-only installs do not depend
on DSPy. The contextvar exists at SDK-root so both adapters can pull
from the same object identity (the design.md §5 "shared contextvar
contract" review-standards §1.4 + G13 verification).

This file ships with D21. D12's next release will import ``_IN_FLIGHT``
from here as its canonical source — until then the contextvar simply
has no consumer when D12 is absent, which is a safe no-op.
"""

from __future__ import annotations

import contextvars


# Per-task boolean flag. ``ContextVar`` (not threading.local) so the
# value scopes to the asyncio task / contextvars context, not the OS
# thread — critical for FastAPI / Starlette / DSPy-async workloads
# where a single thread juggles many tasks.
_IN_FLIGHT: contextvars.ContextVar[bool] = contextvars.ContextVar(
    "spendguard_shim_in_flight",
    default=False,
)


__all__ = ["_IN_FLIGHT"]
