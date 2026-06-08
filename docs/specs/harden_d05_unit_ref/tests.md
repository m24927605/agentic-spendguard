# HARDEN_D05_UR — tests

> Companion to [`design.md`](./design.md) + [`implementation.md`](./implementation.md). Specifies the test inventory required to land HARDEN_D05_UR.

## 1. TS SDK substrate (load-bearing)

### 1.1 Unit tests — `sdk/typescript/tests/locked-surface.test.ts`

**U-LS-01** `UnitRef.unitId` is an OPTIONAL string field at the type level
- Construct `UnitRef` with and without `unitId`; both compile
- Compile-time assertion via `AssertMutuallyAssignable`

**U-LS-02** `UnitRef` 2-field shape (no unitId) is still assignable
- Backward-compat check: `const u: UnitRef = { unit: "X", denomination: 0 }`

### 1.2 Wire-shape tests — `sdk/typescript/tests/unit-id-wire.test.ts` (NEW)

**U-WS-01** `unitId` provided → threads to wire `BudgetClaim.unit.unitId` verbatim
- Mock sidecar; reserve() with UUID; assert mock recorded UUID

**U-WS-02** `unitId` omitted → wire `BudgetClaim.unit.unitId === ""` (backward compat)
- Mock sidecar; reserve() without; assert mock recorded ""

**U-WS-03** `unitId` propagates to ALL claims in multi-claim reserve()
- 3-claim reserve, each with different unitId; all 3 land verbatim

**U-WS-04** `unitId` propagates from `req.projectedUnit` too
- Same threading via the projectedUnit field path (per client.ts:1246)

**U-WS-05** `unitId` propagates from `req.unit` in commitEstimated
- commitEstimated wire shape carries unitId (per client.ts:1356 and 1458)

**U-WS-06** Empty-string `unitId` (operator explicitly passes "") → still empty
- `unitId: ""` is identity-preserving on the wire (no coercion to undefined)

### 1.3 Regression test — `sdk/typescript/tests/handshake-reserve-commit.test.ts`

**U-RG-01** All existing tests still pass after substrate change
- 263+ baseline tests (D05/7 baseline) pass without modification

## 2. Per-adapter test pattern (mechanical)

### 2.1 TS adapter (D04 + D06 + D08 + D29) — each:

**TA-01** Options interface accepts `unitId`
- Type-level test: `{ client, tenantId, unitId: "uuid" }` constructs

**TA-02** Reserve call wire shape carries `unitId`
- Mock SpendGuardClient; instrument adapter's reserve path; assert wire claim's unitId matches options

**TA-03** Backward compat: options without `unitId` still work (no throw at construction)

### 2.2 Python adapter (D19 + D20 + D21 + D22 + D23 + D24 + D26 + D27 + D28) — each:

**TP-01** `SpendGuard{Adk,Strands,DSPy,Agno,BeeAI,AutoGen,Letta,LlamaIndex,Atomic}Options` accepts `unit_id` (or canonical equivalent name)
- `SpendGuardXyzOptions(client=..., tenant_id=..., unit_id="uuid")` constructs

**TP-02** Reserve call wire shape carries `unit_id`
- Mock SpendGuardClient; spy on `client.reserve` arg; assert claim's unit.unit_id matches

**TP-03** Backward compat: omitting `unit_id` continues to construct

### 2.3 .NET adapter (D07) — same pattern in xUnit:

**TN-01** `SpendGuardOptions.UnitId` field present + nullable
**TN-02** `SidecarClient.RequestDecision` plumbs UnitId through
**TN-03** Backward compat: null UnitId proceeds

## 3. Cross-adapter integration test (NEW)

**XA-01** Single `sdk/typescript/tests/cross-adapter-unitid-smoke.test.ts` — NEW
- Constructs `SpendGuardClient` directly with a real `unitId` UUID
- Runs `handshake()` + `reserve()` against the existing mock sidecar
- Asserts the wire shape carries the UUID through the full pipeline
- This protects against future regression of the substrate

## 4. Demo verification (load-bearing — restores marathon-softened gates)

### 4.1 Per affected demo overlay

For each of the 14 affected demos, the `verify_step_agent_real_<name>.sql` MUST pass without softening:

| Demo | Gate restored | Threshold |
|------|---------------|-----------|
| agent_real_langchain_ts | commit_estimated count, outbox closure | ≥1 each |
| agent_real_vercel_ai_mastra | same | ≥1 each |
| agent_real_openai_agents_ts | same | ≥1 each |
| agent_real_inngest_agent_kit | same | ≥1 each |
| agent_real_adk | same | ≥1 each |
| agent_real_strands | same | ≥1 each |
| agent_real_dspy | same | ≥1 each |
| agent_real_agno | same | ≥1 each |
| agent_real_beeai | same | ≥1 each |
| agent_real_autogen | same | ≥1 each |
| agent_real_smolagents | same | ≥1 each |
| agent_real_letta | same | ≥1 each |
| agent_real_llamaindex | same | ≥1 each |
| agent_real_atomic_agents | same | ≥1 each |
| maf_python_real | same | ≥1 each |
| (also D33 anythingllm_real if it touches reserve — verify) | gate as required | ≥1 each |

### 4.2 Demo run cadence

`make demo-up DEMO_MODE=agent_real_<name>` MUST emit the success line for **each** of the 14+ demos:
```
[demo] agent_real_<name> ALL 3 steps PASS (ALLOW + DENY + STREAM)
```

Each demo's runner asserts the counting-stub call_count delta matches expected (2 hits for ALLOW + STREAM, 0 for DENY).

### 4.3 Hard regression check

After HARDEN_D05_UR, the Makefile's `demo-verify-agent-real-<name>` targets MUST NOT have `|| echo "skipped"` softening. Any softening discovered during R1 review is a Blocker per review-standards §3.

## 5. CI matrix entry (forward-compat)

A new CI job `.github/workflows/d05-unit-ref-regression.yml` (if not already covered by an existing matrix) MUST run:
- `pnpm run test` for sdk/typescript, sdk/typescript-langchain, sdk/typescript-vercel-ai, sdk/typescript-openai-agents, sdk/typescript-inngest-agent-kit
- `pytest tests/integrations/{adk,strands,dspy,agno,beeai,autogen,smolagents,letta,llamaindex,atomic_agents,agent_framework}/`
- `dotnet test sdk/dotnet-agent-framework/Spendguard.AgentFramework.Tests/`

(May fold into existing per-package workflows if cleaner.)

## 6. Test count target

| Layer | Tests | Notes |
|-------|-------|-------|
| TS substrate unit | ≥8 | 2 LS + 6 WS + 1 cross-adapter smoke + regression |
| TS adapters (4 × 3 = 12) | ≥12 | TA-01..03 per adapter |
| Python adapters (~9 × 3 = 27) | ≥27 | TP-01..03 per adapter |
| .NET adapter (1 × 3 = 3) | ≥3 | TN-01..03 |
| Cross-adapter smoke | ≥1 | XA-01 |
| Demo verify SQL | 14+ | One per affected demo, all HARD |
| **Total** | **≥65** | |

## 7. Negative cases (must verify)

**N-01** Sending `unitId: ""` explicitly → sidecar rejects with the same `INVALID_REQUEST: claim[N].unit.unit_id empty` error as before. The behavior change is NOT that the SDK rejects empty — it's that the SDK now ALLOWS the caller to set non-empty.

**N-02** Sending `unitId: "not-a-uuid"` → sidecar rejects (UUID validation). The SDK does NOT pre-validate UUID format — that's the ledger's contract.

**N-03** Sending `unitId` for a UUID that doesn't exist in `ledger_units` → sidecar rejects with `INVALID_REQUEST: unit not found` or similar. Again, SDK is a pure pass-through.

These negative cases ensure we're not changing semantics, only adding a new field.

## 8. Coverage delta

The TS substrate change adds ~10 LoC; tests cover all branches:
- `unit.unitId === undefined` → "" (already-covered + new explicit assertion)
- `unit.unitId === ""` → "" (new explicit assertion)
- `unit.unitId === "valid-uuid"` → "valid-uuid" (new primary assertion)

Branch coverage on `mapUnitRef`: 100%. Test count: per branch ≥1.
