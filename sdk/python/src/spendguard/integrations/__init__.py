"""Per-framework adapters for SpendGuard.

Each integration is gated behind an optional dependency:

  pip install spendguard-sdk[pydantic-ai]    # spendguard.integrations.pydantic_ai
  pip install spendguard-sdk[langchain]      # spendguard.integrations.langchain
  pip install spendguard-sdk[langgraph]      # spendguard.integrations.langgraph
  pip install spendguard-sdk[openai-agents]  # spendguard.integrations.openai_agents

Importing a submodule whose extras are not installed raises a clean
ImportError pointing at the install hint (no deep ModuleNotFoundError).
"""
