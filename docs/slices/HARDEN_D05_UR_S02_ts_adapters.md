# HARDEN_D05_UR_S02 — TS adapter contract sweep

> **Pass**: HARDEN_D05_UR
> **Slice**: 2 of 4 (M — mechanical sweep across 4 TS adapter packages)

## Scope

Add optional `unitId?: string` to the options interface of each TS adapter, plumb through to the underlying `client.reserve` `BudgetClaim.unit.unitId`, JSDoc + tests + CHANGELOG. **Strictly additive.**

The 4 TS adapters:
1. **D04 LangChain TS** (`sdk/typescript-langchain/`) — `SpendGuardCallbackHandlerOptions`
2. **D06 Vercel AI SDK** (`sdk/typescript-vercel-ai/`) — `SpendGuardMiddlewareOptions`
3. **D08 OpenAI Agents TS** (`sdk/typescript-openai-agents/`) — `SpendGuardAgentsOptions`
4. **D29 Inngest AgentKit** (`sdk/typescript-inngest-agent-kit/`) — `WrapWithSpendGuardOptions`

Plus D07 .NET (`sdk/dotnet-agent-framework/`) — `SpendGuardOptions.UnitId: Guid?` per [`implementation.md`](../specs/harden_d05_unit_ref/implementation.md) §2.3.

## Per-package file pattern

For each TS package:
- `src/options.ts` — add `unitId?: string` to options interface with JSDoc
- `src/handler.ts` / `src/middleware.ts` / `src/withSpendGuard.ts` — plumb to claim.unit.unitId
- `tests/*.test.ts` — TA-01 / TA-02 / TA-03 per [`tests.md`](../specs/harden_d05_unit_ref/tests.md) §2.1
- `CHANGELOG.md` — Unreleased entry citing HARDEN_D05_UR

For .NET:
- `Spendguard.AgentFramework/Options/SpendGuardOptions.cs` — `UnitId: Guid?`
- `Spendguard.AgentFramework/Sidecar/SidecarClient.cs` — plumb to RequestDecision
- `Spendguard.AgentFramework.Tests/UnitIdTests.cs` (NEW)

## Test plan

≥12 new tests across 4 TS adapters (3 each: TA-01, TA-02, TA-03).
≥3 new tests for .NET (TN-01, TN-02, TN-03).

## Anti-scope

- ❌ Python adapter sweep (SLICE 3)
- ❌ Demo overlay changes (SLICE 4)
- ❌ Verify SQL changes (SLICE 4)
- ❌ Memory updates (SLICE 4)

## Acceptance gates

For each affected package:
1. typecheck + lint + build + test all green
2. ≥3 new tests pass
3. No baseline test regression
4. Bundle delta per package < 50 bytes
5. CHANGELOG entry shipped

## Reviewer

Claude Code CLI per LOCKED standards. Single R1 review per package OR one bundled R1 over all 4+1 — reviewer choice.

## Backlinks

- Spec set: [`implementation.md`](../specs/harden_d05_unit_ref/implementation.md) §2.1, §2.3; [`tests.md`](../specs/harden_d05_unit_ref/tests.md) §2.1, §2.3
- Slice 1: [`HARDEN_D05_UR_S01_substrate.md`](HARDEN_D05_UR_S01_substrate.md) — load-bearing prerequisite
