"""decision_context_json enrichment for Langflow-driven calls.

Per design.md §6, the demo's verify SQL distinguishes Langflow calls
from raw LangChain SDK calls via ``decision_context->>'integration' =
'langchain'`` AND ``decision_context->>'source' = 'langflow'``. The
existing ``spendguard.integrations.langchain._agenerate`` doesn't pass
``decision_context_json`` to ``request_decision`` -- and per
``implementation.md`` §3 we MUST NOT mutate it.

The fix: monkey-patch the ``SpendGuardClient.request_decision`` method
on the per-build client instance to inject the langflow tags. Because
we own the client (constructed inside ``build_model``), this stays
isolated to the wrapper's lifecycle and never leaks across builds
(INV-4) or to other tenants.

For streaming calls the ``stream=true`` tag is added through the
streaming-aware ``decision_context_json``; for DENY paths the demo
driver records ``stub_hits`` separately on its post-step assert (the
counting-stub hit count) -- the SQL only needs it for the negative
INV-1 check.
"""

from __future__ import annotations

import functools
from typing import Any


def install_decision_context(
    client: Any,
    *,
    extra: dict[str, Any] | None = None,
) -> Any:
    """Wrap ``client.request_decision`` so every call carries Langflow tags.

    Args:
        client: a connected ``SpendGuardClient`` instance constructed
            by ``build_model``.
        extra: optional extra key/value pairs to fold into the
            ``decision_context_json`` for every call (e.g. ``stream``).
            Defaults to ``None``.

    Returns:
        The same ``client`` with ``request_decision`` patched.

    Notes:
        Caller-supplied ``decision_context_json`` kwargs win on key
        collision (per ``SpendGuardClient.request_decision`` semantics
        in ``sdk/python/src/spendguard/client.py`` line 480).
    """
    original_req = client.request_decision
    base_ctx = {"integration": "langchain", "source": "langflow"}
    if extra:
        for k, v in extra.items():
            base_ctx[k] = v

    @functools.wraps(original_req)
    async def _request_decision_tagged(*args: Any, **kwargs: Any) -> Any:
        caller_ctx = kwargs.get("decision_context_json") or {}
        merged = dict(base_ctx)
        # Caller-supplied wins on collision (mirrors SDK semantics).
        merged.update(caller_ctx)
        kwargs["decision_context_json"] = merged
        return await original_req(*args, **kwargs)

    # ``SpendGuardClient`` is not a Pydantic BaseModel — assignment
    # lands directly. We use ``object.__setattr__`` defensively in
    # case the SDK switches to slots in a future release.
    object.__setattr__(client, "request_decision", _request_decision_tagged)
    return client


__all__ = ["install_decision_context"]
