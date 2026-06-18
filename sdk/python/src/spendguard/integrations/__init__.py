"""Per-framework adapters for SpendGuard.

Each integration is gated behind an optional dependency:

  # pydantic-ai auto-install is temporarily fail-closed due to
  # CVE-2026-25580; install a vetted non-vulnerable upstream release
  # when available.
  pip install spendguard-sdk[langchain]            # spendguard.integrations.langchain
  pip install spendguard-sdk[langgraph]            # spendguard.integrations.langgraph
  pip install spendguard-sdk[openai-agents]        # spendguard.integrations.openai_agents
  pip install spendguard-sdk[litellm]              # spendguard.integrations.litellm
  pip install spendguard-sdk[litellm-guardrail]    # spendguard.integrations.litellm_guardrail

Importing a submodule whose extras are not installed raises a clean
ImportError pointing at the install hint (no deep ModuleNotFoundError).

Top-level convenience re-export (PEP 562 lazy ``__getattr__``):

    ``from spendguard.integrations import SpendGuardGuardrail``

The lazy hook keeps ``import spendguard.integrations`` lightweight (no
forced ``litellm`` import) while still surfacing the new D11 entry
point on the integrations namespace per
``docs/internal/slices/COV_D11_S1_guardrail_class.md`` test-plan step 3.
"""

from __future__ import annotations

from typing import TYPE_CHECKING

if TYPE_CHECKING:  # pragma: no cover - typing-only branch
    from .litellm_guardrail import SpendGuardGuardrail  # noqa: F401


# Symbols that ``from spendguard.integrations import <name>`` resolves
# via lazy submodule import. Map of attr name -> submodule path inside
# this package. Keep the dict tiny — each entry forces a submodule
# import on first access, which transitively pulls the upstream
# framework. SLICE 1 ships only ``SpendGuardGuardrail``; further
# integrations stay reachable via their fully-qualified module path.
_LAZY_EXPORTS = {
    "SpendGuardGuardrail": "spendguard.integrations.litellm_guardrail",
}


def __getattr__(name: str) -> object:
    """PEP 562 lazy attribute resolver for lightweight re-exports."""
    target_module = _LAZY_EXPORTS.get(name)
    if target_module is None:
        raise AttributeError(
            f"module {__name__!r} has no attribute {name!r}"
        )
    import importlib

    module = importlib.import_module(target_module)
    return getattr(module, name)


def __dir__() -> list[str]:
    """Expose lazy names to ``dir()`` / IDE introspection."""
    return sorted({*globals().keys(), *_LAZY_EXPORTS.keys()})


__all__ = ["SpendGuardGuardrail"]
