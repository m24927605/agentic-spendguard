# HARDEN_D05_UR — design

> **Scope:** Broaden the D05 TS SDK substrate's `UnitRef` public surface to thread `unit_id` (UUID) through to the sidecar ledger, unblocking DENY + STREAM full-assertion across ~14 adapter demos.
> **Affected adapters:** D04 LangChain TS, D06 Vercel AI SDK + Mastra, D07 Microsoft Agent Framework (Python branch), D08 OpenAI Agents TS, D19 Google ADK, D20 AWS Strands, D21 DSPy, D22 Agno, D23 BeeAI, D24 AutoGen, D26 Letta, D27 LlamaIndex, D28 Atomic Agents, D29 Inngest AgentKit.

## 1. The substrate gap

### 1.1 Symptom seen during marathon
All real-mode demos for the ~14 adapters above proceed cleanly through:
1. Sidecar UDS handshake
2. Adapter PRE hook fires
3. SDK builds the wire `DecisionRequest`

…and then the ledger rejects the request with:

```
claim[0].unit.unit_id empty
```

raised at `services/ledger/src/handlers/reserve_set.rs:258` (or its sibling validators in `provider_report.rs:144` and elsewhere).

### 1.2 Root cause (LOCKED at this audit)
`sdk/typescript/src/client.ts:1612-1637` (function `mapUnitRef`) hardcodes the wire-level `unit_id` to the empty string:

```ts
function mapUnitRef(unit: UnitRef): { unitId: string; kind: 0; ... } {
  return {
    unitId: "",         // ← THE GAP
    kind: 0,
    currency: "",
    unitName: unit.unit,
    ...
  };
}
```

The public-surface `UnitRef` (lines 144-147) is the compact 2-field shape:

```ts
export interface UnitRef {
  unit: string;
  denomination: number;
}
```

There is **no public path** for an adapter author to thread the canonical-truth ledger `unit_id` UUID into the wire payload. The comment at lines 1622-1627 acknowledges this and points at SLICE 6 for the broadening — but SLICE 6 chose subpath splits over UnitRef broadening. The gap was left for a post-marathon HARDEN slice.

### 1.3 Why the ledger requires `unit_id`
- Ledger reserve flow at `reserve_set.rs:258` validates every claim has a non-empty `unit_id` (UUID format)
- Ledger reconciles `(tenant_id, budget_id, window_instance_id, unit_id)` to the canonical row in `ledger_accounts`
- `unit_id` is not a free-form string — it's the UUID PK of the unit row that pricing + budget binding hang off

### 1.4 Why we can't just derive `unit_id` server-side
- Ledger doesn't know which unit (USD_MICROS vs OUTPUT_TOKENS vs custom-credit-program) the caller meant — `unit_name` is free-form
- Different tenants can have same unit_name pointing at different unit rows (tenant-scoped namespacing)
- Auditability demands the caller declares which unit_id, so the audit chain captures intent
- Same reason `tenant_id` + `budget_id` + `window_instance_id` are all caller-declared

## 2. The fix shape (LOCKED)

### 2.1 Broaden public `UnitRef`

```ts
export interface UnitRef {
  /** Free-form unit slug — e.g. "USD_MICROS", "OUTPUT_TOKENS", "ACU". Required. */
  unit: string;
  /** Denomination exponent — e.g. -6 for micros. Required. */
  denomination: number;
  /** Optional UUID of the canonical ledger row this unit binds to.
   *  When provided, the SDK passes it verbatim on the wire; when omitted, the
   *  SDK sends "" and the ledger rejects with INVALID_REQUEST. Adapter authors
   *  SHOULD provide unitId when they need the ledger to resolve a budget claim;
   *  recipe-style adapters (where no ledger reserve happens) MAY omit. */
  unitId?: string;
}
```

### 2.2 `mapUnitRef` thread-through

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
    unitId: unit.unitId ?? "",  // ← THE FIX
    kind: 0,
    currency: "",
    unitName: unit.unit,
    tokenKind: "",
    modelFamily: "",
    creditProgram: "",
  };
}
```

### 2.3 Public surface — adapter-side options

Each adapter that owns a SpendGuardClient-construction site MUST pass `unitId` through one of:
- Explicit option (e.g. `SpendGuardCallbackHandlerOptions.unitId`)
- Env-var fallback (e.g. `SPENDGUARD_UNIT_ID`)
- Demo overlay sets the env var

This is **additive** to the existing options surface — no breaking change.

### 2.4 Python SDK side
The Python SDK does **NOT** have the same gap. `sdk/python/src/spendguard/client.py` exposes `unit_id` via `BudgetClaim` construction and the demo overlays already pass it. Python adapters built off the marathon (D11, D14-D16, D19, D20, D21, D22, D23, D24, D26, D27, D28) already thread it.

The TS-only nature of the gap explains why the .NET D07 demo (which uses TS-equivalent shape) also degraded but Python D07 demo PRE-succeeded with reservation row landing.

### 2.5 What changes per-adapter

Each of the 14 affected adapters gets:
1. `unitId?: string` added to the options surface (typed)
2. Plumb `unitId` to the underlying SpendGuardClient `BudgetClaim.unit.unitId` field
3. Demo overlay's env var → unitId
4. JSDoc note: "Required for ledger reserve; optional for recipe-style integration"

No new tests are required *for the option plumbing itself* (it's mechanical), but each adapter MUST add at least one unit test asserting the unitId reaches the wire shape, and the demo verify SQL gates the full DENY+STREAM cycle now.

## 3. Anti-scope

- ❌ Server-side derivation of unit_id from unit_name (rejected per §1.4)
- ❌ New `unit_id` resolution RPC (sidecar already does this at reserve time)
- ❌ Multi-tenant unit_id namespacing changes (already tenant-scoped per ledger schema)
- ❌ Breaking the existing 2-field `UnitRef` shape — `unitId` is OPTIONAL
- ❌ Python SDK changes — already correct (see §2.4)
- ❌ Cross-language fixture additions — no new helper functions
- ❌ Proto changes — wire shape already supports `unit_id`

## 4. Affected files (estimate)

| File | Change | LoC est. |
|------|--------|----------|
| `sdk/typescript/src/client.ts` | UnitRef interface + mapUnitRef thread-through | ~10 |
| `sdk/typescript/tests/handshake-reserve-commit.test.ts` or new | unitId on wire shape | ~30 |
| `sdk/typescript-langchain/src/options.ts` | optional unitId field | ~5 |
| `sdk/typescript-langchain/src/handler.ts` | plumb unitId → reserve | ~5 |
| `sdk/typescript-langchain/tests/...` | unit test | ~20 |
| × 8 (D04+D06+D08+D29 TS adapters + 4 Python adapters' env vars) | mechanical mirror | ~200 |
| `deploy/demo/*/docker-compose.yaml` × 14 | add `SPENDGUARD_UNIT_ID=...` env | ~14 lines |
| `deploy/demo/*/verify_step_*.sql` × ~10 | tighten gates that were softened | ~50 |
| `sdk/python/CHANGELOG.md` + adapter CHANGELOGs | no-op disclosure | ~30 |
| **Total** | | **~350 LoC** |

## 5. Demo gate rectification

During marathon, each of the affected demos had its verify SQL softened (often with `|| echo "outbox-closure skipped...D05 UnitRef gap"`) to allow the partial-pass demo to ship. HARDEN_D05_UR must restore the hard gates:
- `reserve >= 1` already PASS
- `commit_estimated >= 1` was softened — restore HARD
- `denied_decision >= 1` already PASS for DENY paths
- Outbox closure was softened — restore HARD
- INV-2 strict-order already PASS

## 6. Forward-compat note

When v0.2 adds typed `tokenKind`, `modelFamily`, `creditProgram` (currently empty strings in the wire), the broadened `UnitRef` interface accepts them as optional fields. No re-broadening needed.

## 7. Out-of-scope (track as separate followups, not HARDEN_D05_UR)

- The `defaultCallSignature` cross-language fixture deferral (tracked as separate work since D05/9)
- The proto `LLM_CALL_OUTCOME` event-kind bump (cross-component slice deferred since D05/5)
- The `x-spendguard-reason-code` trailer extension on the sidecar side (deferred since D05/5)
