# Changelog

## 0.3.0 — 2026-05-20

First non-pre-release. Bumps Dev Status from 3-Alpha → 4-Beta.
LiteLLM integration end-to-end + signed audit chain enrichment.

### Added

- **`spendguard.integrations.litellm`** — full LiteLLM proxy + direct
  async integration (Phase 5):
  - `SpendGuardLiteLLMCallback` + `_LoopBoundCallback` — proxy
    CustomLogger that gates every `/v1/chat/completions` call (pre-call
    reserve, post-call commit, streaming reconciler, failure release).
  - `SpendGuardDirectAcompletion` — async wrapper for
    `litellm.acompletion()` direct callers (no proxy needed). Sync
    `litellm.completion()` NOT supported (see ADR-005).
  - Fail-closed defaults; `SPENDGUARD_LITELLM_FAIL_OPEN=1` dev override.
- `spendguard.integrations.openai_agents` — OpenAI Agents SDK gating
  (model-wrap + composite paths).
- `spendguard.integrations.langchain` / `.langgraph` — ChatModel
  gating with streaming reconciler.
- `spendguard.integrations.agt` — Microsoft AGT composite evaluator
  (PolicyEngine + SpendGuard as policy plugin).
- 5 new demo modes: `agent_real_openai_agents`, `agent_real_agt`,
  `litellm_real`, `litellm_deny`, `litellm_direct`.
- `examples/litellm-proxy-composite/` — runnable example with
  docker-compose + production-shape callback template.

### Changed

- `DecisionDenied.status_code = 403` — LiteLLM proxy now maps
  SpendGuard policy denials to HTTP 403 (was: 500).
- `SidecarUnavailable.status_code = 503` — transient infra failures
  map to HTTP 503 Service Unavailable.
- `litellm` extra now pulls `litellm[proxy]` transitively
  (fastapi/uvicorn/gunicorn) so `python -m litellm.proxy.proxy_cli`
  works out of the box.

### Audit chain (GH #77 closed)

The sidecar now extracts a 12-key allowlist from
`request_decision.runtime_metadata` and emits the values into
`canonical_events.payload_json.data.spendguard.*` for both ALLOW +
DENY CloudEvent payloads. Allowlist: integration / litellm_call_id /
model / pricing_version / price_snapshot_hash_hex / fx_rate_version /
unit_conversion_version / prompt_hash / call_type / stream / mode /
team_id. PII smuggling guard: only scalar kinds accepted; Struct/List
dropped with WARN.

### Documentation

- `docs/specs/litellm-integration/PROXY_RECIPE.md` — operator recipe.
- `docs/site/docs/integrations/litellm.md` — public docs page.
- `docs/specs/litellm-integration/100-percent-design.md` — architect's
  design lock for the 100%-completion epics.

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
