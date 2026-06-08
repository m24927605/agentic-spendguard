# HARDEN_D05_UR — implementation

> Companion to [`design.md`](./design.md). Specifies the LOCKED implementation surface, function signatures, and per-file change shape.

## 1. TS SDK substrate (load-bearing)

### 1.1 `sdk/typescript/src/client.ts`

#### UnitRef interface (~line 144)

**Before:**
```ts
export interface UnitRef {
  unit: string;
  denomination: number;
}
```

**After (LOCKED):**
```ts
export interface UnitRef {
  /** Free-form unit slug — e.g. "USD_MICROS", "OUTPUT_TOKENS", "ACU". Required. */
  unit: string;

  /** Denomination exponent — e.g. -6 for micros. Required. */
  denomination: number;

  /** Canonical-truth UUID of the ledger unit row.
   *
   * When provided, the SDK threads it verbatim onto `BudgetClaim.unit.unit_id`
   * on the wire. When omitted, the SDK sends "" and the ledger reject with
   * `INVALID_REQUEST: claim[N].unit.unit_id empty`.
   *
   * Adapters that issue ledger-backed `client.reserve()` MUST provide
   * `unitId`. Recipe-style integrations (where no ledger reserve happens) MAY
   * omit. The most common operator path is to set this from the
   * `SPENDGUARD_UNIT_ID` env var at adapter construction time.
   *
   * NB: this is the ledger UUID, distinct from the free-form `unit` slug —
   * they are NOT interchangeable. Multiple unit slugs can resolve to the same
   * unit_id when migration aliasing is configured.
   */
  unitId?: string;
}
```

#### mapUnitRef function (~line 1612)

**Before:**
```ts
function mapUnitRef(unit: UnitRef): { unitId: string; ... } {
  return {
    unitId: "",
    ...
  };
}
```

**After (LOCKED):**
```ts
function mapUnitRef(unit: UnitRef): {
  unitId: string;
  kind: 0;
  currency: string;
  unitName: string;
  tokenKind: string;
  modelFamily: string;
  creditProgram: string;
} {
  return {
    unitId: unit.unitId ?? "",
    kind: 0,
    currency: "",
    unitName: unit.unit,
    tokenKind: "",
    modelFamily: "",
    creditProgram: "",
  };
}
```

**The comment block at lines 1622-1627 must be REWRITTEN** to reflect the new behavior. Old comment said "leave the canonical-truth unit_id empty — ledger resolves canonical truth server-side"; new comment must say "thread unitId through when provided; empty triggers ledger INVALID_REQUEST."

#### Locked-surface test extension

`sdk/typescript/tests/locked-surface.test.ts` MUST add:
```ts
it("UnitRef has optional unitId field", () => {
  const u: UnitRef = { unit: "USD_MICROS", denomination: -6, unitId: "550e8400-e29b-41d4-a716-446655440000" };
  expect(u.unitId).toBe("550e8400-e29b-41d4-a716-446655440000");
  const u2: UnitRef = { unit: "USD_MICROS", denomination: -6 };
  expect(u2.unitId).toBeUndefined();
});
```

#### Wire-shape test extension

`sdk/typescript/tests/handshake-reserve-commit.test.ts` (or new file `unitId-wire.test.ts`) MUST add:
```ts
it("unitId on UnitRef threads to wire BudgetClaim.unit.unit_id verbatim", async () => {
  // Mock sidecar captures the wire RequestDecisionRequest
  // ... reserve() with { unit: "USD_MICROS", denomination: -6, unitId: "uuid-here" }
  // ... assert mock.lastRequest.claims[0].unit.unitId === "uuid-here"
});

it("missing unitId defaults to empty string on wire (sidecar rejects)", async () => {
  // ... reserve() with { unit: "USD_MICROS", denomination: -6 }
  // ... assert mock.lastRequest.claims[0].unit.unitId === ""
});
```

## 2. Per-adapter contract (mechanical)

### 2.1 TS adapter pattern (4 adapters: D04 + D06 + D08 + D29)

Each adapter that exposes a public options surface (e.g. `SpendGuardCallbackHandlerOptions`, `SpendGuardMiddlewareOptions`, `SpendGuardAgentsOptions`, `WrapWithSpendGuardOptions`) MUST:

1. Add optional `unitId?: string` field
2. Plumb to the underlying `BudgetClaim.unit.unitId` when building the reserve request
3. Document the env-var fallback pattern in JSDoc (e.g., `// often sourced from SPENDGUARD_UNIT_ID env var`)
4. Add ≥1 unit test asserting the unitId reaches `mockSidecar.reserveCalls[0].claims[0].unit.unitId`

### 2.2 Python adapter pattern (8+ adapters: D19 + D20 + D21 + D22 + D23 + D24 + D26 + D27 + D28)

Python SDK substrate already correct. Each adapter MUST:
1. Add optional `unit_id: str | None = None` field to options dataclass
2. Plumb to the underlying SpendGuardClient `BudgetClaim.unit.unit_id` field
3. Document `SPENDGUARD_UNIT_ID` env-var fallback
4. Add ≥1 pytest asserting wire shape

### 2.3 .NET adapter pattern (1 adapter: D07 .NET)

`sdk/dotnet-agent-framework/Spendguard.AgentFramework/Options/SpendGuardOptions.cs` adds `UnitId: Guid?` field. `SidecarClient.RequestDecision` plumbs through.

## 3. Demo overlay sweep

### 3.1 Each affected demo overlay
For each of `deploy/demo/agent_real_{langchain_ts,vercel_ai_mastra,openai_agents_ts,inngest_agent_kit,adk,strands,dspy,agno,beeai,autogen,smolagents,letta,llamaindex,atomic_agents,maf_python}/docker-compose.yaml`:

Add env var to the adapter-runner service:
```yaml
environment:
  - SPENDGUARD_UNIT_ID=00000000-0000-4000-8000-000000000001
```

(Use the same UUID across all demos to match the `seed_workspace.sql` ledger row, OR each demo's existing tenant_id-keyed unit row.)

### 3.2 Each affected verify SQL
Files: `deploy/demo/verify_step_agent_real_{*}.sql`

Remove softening lines like:
```sql
-- BEFORE (softened during marathon):
-- SELECT count(*) FROM commit_estimated_events ... → coalesce(0, ...) → tolerate 0
```

Restore HARD gate:
```sql
-- AFTER (restored):
DO $$
DECLARE c integer;
BEGIN
  SELECT count(*) INTO c FROM canonical_events WHERE event_type = 'llm_call_post' AND ...;
  IF c < 1 THEN RAISE EXCEPTION 'INV-5 violated: zero commit_estimated events'; END IF;
END $$;
```

### 3.3 Makefile
Remove the `|| echo "outbox-closure skipped...D05 UnitRef gap"` softening from `demo-verify-agent-real-{*}` targets — outbox closure must now hard-fail.

## 4. Test seed data

Each demo overlay's seed SQL (if it has one — D11/6 pattern) seeds a tenant + budget + unit row. The unit row must use the same UUID that the env var `SPENDGUARD_UNIT_ID` carries. Default LOCKED value:

```sql
INSERT INTO ledger_units (unit_id, unit_name, denomination, ...)
VALUES ('00000000-0000-4000-8000-000000000001', 'USD_MICROS', -6, ...);
```

(Demos that already have their own canonical seed unit_id retain it.)

## 5. CHANGELOG entries

Each affected SDK package adds an Unreleased entry:

```markdown
## [Unreleased]

### Added
- `UnitRef.unitId` — optional canonical-truth UUID of the ledger unit row.
  Adapters that issue ledger-backed reserve calls now thread this through to
  `BudgetClaim.unit.unit_id` on the wire (closes the HARDEN_D05_UR substrate
  gap that previously blocked DENY+STREAM full assertion across ~14 adapter
  demos). Backward-compat: omitting `unitId` matches prior behavior (sends
  empty string; sidecar will INVALID_REQUEST as before).
```

## 6. Compile-time check

After the substrate change, all 14 adapter test suites + demo build must compile clean. The TS substrate change is additive (optional field), so existing call sites continue to compile.

## 7. Backward compat invariant

Calling `reserve({...})` with a `BudgetClaim` whose `unit` lacks `unitId` MUST still produce the same wire shape as before (empty string `unitId`). The behavior change is *only* visible when the new field is set. This means existing adapter unit tests that don't set `unitId` MUST continue to pass without modification.

## 8. Cross-language fixture invariant

The 20 cross-language fixtures at `sdk/fixtures/cross-language/v1.json` are NOT affected by this change (they test `deriveIdempotencyKey` / `computePromptHash` / `deriveUuidFromSignature`, not `UnitRef`). No fixture regeneration required.

## 9. Performance

Adding one optional field assignment to `mapUnitRef` is a constant-time op. Bundle impact on `sdk/typescript/dist/index.js` minified: < 100 bytes (a single `?? ""` operator). No tree-shaking impact.
