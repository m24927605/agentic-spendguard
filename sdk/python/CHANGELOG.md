# Changelog

## 0.1.0a1 (Phase 4 O1) — 2026-05-09

Initial SDK release. Restructured from `spendguard-pydantic-ai` to the
multi-framework `spendguard-sdk` with optional extras.

### Added

- Top-level package `spendguard` with framework-agnostic core
  (`SpendGuardClient`, `DecisionStopped`, etc.)
- `spendguard.integrations.pydantic_ai` (was `spendguard_pydantic_ai`
  top-level) — gated behind `pip install 'spendguard-sdk[pydantic-ai]'`
- Slots reserved for `spendguard.integrations.langchain`,
  `spendguard.integrations.langgraph`,
  `spendguard.integrations.openai_agents` (Phase 4 O5).

### Changed

- Package name: `spendguard-pydantic-ai` → `spendguard-sdk`
- Python module: `spendguard_pydantic_ai` → `spendguard`
- Pydantic-AI wrapper moved from top-level to
  `spendguard.integrations.pydantic_ai`
- Internal contextvar renamed `spendguard_pydantic_ai_run_context` →
  `spendguard_run_context`

### Migration from `spendguard-pydantic-ai`

```python
# Before
from spendguard_pydantic_ai import SpendGuardClient, SpendGuardModel, RunContext

# After
from spendguard import SpendGuardClient
from spendguard.integrations.pydantic_ai import SpendGuardModel, RunContext
```

```bash
# Before
pip install spendguard-pydantic-ai

# After
pip install 'spendguard-sdk[pydantic-ai]'
```
