"""Per-framework adapters for SpendGuard.

Each integration is gated behind an optional dependency:

  # pydantic-ai auto-install is temporarily fail-closed due to
  # CVE-2026-25580; install a vetted non-vulnerable upstream release
  # when available.
  pip install spendguard-sdk[langchain]      # spendguard.integrations.langchain
  pip install spendguard-sdk[langgraph]      # spendguard.integrations.langgraph
  pip install spendguard-sdk[openai-agents]  # spendguard.integrations.openai_agents
  pip install spendguard-sdk[litellm]        # spendguard.integrations.litellm

Importing a submodule whose extras are not installed raises a clean
ImportError pointing at the install hint (no deep ModuleNotFoundError).
"""
