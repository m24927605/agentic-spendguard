"""SpendGuard Langflow custom component package.

Drop-in budget gate for Langflow (https://github.com/langflow-ai/langflow).
The component wraps any LangChain ``BaseChatModel`` connected as the
``inner`` input and routes every call through the SpendGuard sidecar's
RequestDecision -> CommitEstimated lifecycle.

Public surface::

    from spendguard_langflow import SpendGuardChatModelWrapper

Anything else is a private helper; import paths under
``spendguard_langflow._*`` are not part of the public API.

The component delegates all reservation/commit logic to
``spendguard.integrations.langchain.SpendGuardChatModel`` so SDK fixes
propagate without re-vendoring. This package adds:

1. Langflow Component metadata (canvas inputs + output handle).
2. ``run_context`` auto-bind glue so canvas-driven invocations don't
   need explicit ``async with run_context(...)`` wrapping.
3. ``decision_context`` enrichment tagging every call as
   ``integration=langchain, source=langflow`` so the audit chain can
   distinguish Langflow-driven calls from raw LangChain SDK callers.
4. ``spendguard-langflow-install`` CLI to drop the component file into
   ``$LANGFLOW_COMPONENTS_PATH`` for vendored installs.
"""

from __future__ import annotations

from ._version import __version__

try:
    from .component import SpendGuardChatModelWrapper
except ImportError:
    # langflow not installed yet — `from spendguard_langflow import
    # __version__` still works for tooling. The component import raises
    # on first attribute access.
    SpendGuardChatModelWrapper = None  # type: ignore[assignment]


__all__ = ["SpendGuardChatModelWrapper", "__version__"]
