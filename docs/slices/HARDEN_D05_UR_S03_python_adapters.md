# HARDEN_D05_UR_S03 — Python adapter contract sweep

> **Pass**: HARDEN_D05_UR
> **Slice**: 3 of 4 (M — mechanical sweep across ~9 Python adapters)

## Scope

Add optional `unit_id: str | None = None` to each Python adapter's options dataclass, plumb to the underlying SpendGuardClient `BudgetClaim.unit.unit_id` field. Strictly additive.

The Python adapters affected:
1. **D07 Microsoft Agent Framework** (Python branch) — `spendguard.integrations.agent_framework`
2. **D19 Google ADK** — `spendguard.integrations.adk`
3. **D20 AWS Strands** — `spendguard.integrations.strands`
4. **D21 DSPy** — `spendguard.integrations.dspy`
5. **D22 Agno** — `spendguard.integrations.agno`
6. **D23 BeeAI** — `spendguard.integrations.beeai`
7. **D24 AutoGen** — `spendguard.integrations.autogen`
8. **D26 Letta** — `spendguard.integrations.letta`
9. **D27 LlamaIndex** — `spendguard.integrations.llamaindex`
10. **D28 Atomic Agents** — `spendguard.integrations.atomic_agents`

Also (verify): D11 LiteLLM Proxy + D11 SDK shim already pass `unit_id` through. Confirm no regression there.

## Per-adapter file pattern

For each adapter:
- `_options.py` — add `unit_id: str | None = None` to options dataclass
- `_hook.py` / `_wrapper.py` / `_middleware.py` — plumb to claim.unit.unit_id
- `tests/integrations/<name>/test_*.py` — TP-01 / TP-02 / TP-03 per [`tests.md`](../specs/harden_d05_unit_ref/tests.md) §2.2

## Test plan

≥27 new tests (3 per adapter × 9 adapters). Some may consolidate into the existing test file as new test functions.

## Anti-scope

- ❌ TS adapter sweep (SLICE 2 — done)
- ❌ TS SDK substrate (SLICE 1 — done)
- ❌ Demo overlay changes (SLICE 4)

## Acceptance gates

1. Full SDK pytest pass (≥1140 baseline + 27 new = ≥1167)
2. No regression in any existing test
3. ruff clean on touched files
4. No new dep added

## Reviewer

Claude Code CLI per LOCKED standards.

## Backlinks

- Spec set: [`implementation.md`](../specs/harden_d05_unit_ref/implementation.md) §2.2; [`tests.md`](../specs/harden_d05_unit_ref/tests.md) §2.2
