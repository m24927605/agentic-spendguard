# COV_D39_01_ts_pkg — D39 AG-UI spend-event family: `@spendguard/ag-ui` TS package

> **Deliverable**: D39 AG-UI spend-event family (display-only)
> **Slice**: 1 of 3 (M)
> **Spec set**: [`docs/specs/coverage/D39_ag_ui/`](../../specs/coverage/D39_ag_ui/)
> **LOCKED design.md trumps this slice doc** (coverage build-plan §1.2 P0; D05/7 slice-author bug pattern). Schema/API text below is copied verbatim from the spec set — if any copy here disagrees with `design.md`, `design.md` wins and the disagreement is a slice-author bug.

## Scope

Build the entire `@spendguard/ag-ui` npm package (version `0.1.0`, Apache-2.0, in-tree at `sdk/typescript-ag-ui/`, same top-level layout as `sdk/typescript-langchain/`): the five event-name constants, the envelope + five builder-input types, the five pure builders, the field validators, the §7 canonical serializer, the SSE encode helper, and the validation error class — plus the full TS test suite (TP-01..TP-31), the AG-UI pinned compat test, and the **minting of the frozen cross-language fixture corpus** `sdk/fixtures/cross-language/ag_ui_v1.json` via the reference generator `generate_ag_ui.mjs`. Builders are pure (no clock, no RNG, no env, no I/O, no `node:` imports anywhere in `src/`); the package has **zero runtime dependencies** and never imports `@ag-ui/core` or `@spendguard/sdk` at runtime (design.md §4, §10, §11.3-§11.4).

This is the vocabulary + reference-implementation slice: the corpus minted here is FROZEN at merge and becomes the byte-level contract that slice 2 (Python) must match without co-evolution (design.md §12 justification). Size class: **M**.

## Files touched

All NEW, per implementation.md §1.1 (verbatim layout):

```
sdk/typescript-ag-ui/
├── package.json
├── tsconfig.json
├── tsconfig.tests.json
├── tsup.config.ts
├── biome.json
├── vitest.config.ts
├── README.md                  # carries the §1.1 display-only notice verbatim
├── CHANGELOG.md
├── LICENSE_NOTICES.md
├── scripts/
│   └── size-budget.sh         # copy of sdk/typescript-langchain/scripts pattern
├── src/
│   ├── index.ts               # public barrel — design.md §8.1, nothing else
│   ├── names.ts               # SPENDGUARD_AG_UI_EVENT_NAMES + name type
│   ├── events.ts              # SpendGuardAgUiEvent + BuildContext + 5 Input interfaces
│   ├── builders.ts            # the 5 build* functions + shared payload assembly
│   ├── validate.ts            # field validators (non-empty, decimal, RFC3339, arrays)
│   ├── canonical.ts           # canonicalEventJson (design.md §7 rule)
│   ├── sse.ts                 # encodeSse + AgUiEmit type
│   ├── errors.ts              # AgUiEventValidationError
│   └── version.ts             # VERSION constant (generated, matches package.json)
└── tests/
    ├── builders.test.ts       # TP-01..TP-13 (purity, payload shape, mapping)
    ├── validate.test.ts       # TP-14..TP-19 (omission, rejection, taxonomy)
    ├── canonical.test.ts      # TP-20..TP-24 (canonical rule conformance)
    ├── sse.test.ts            # TP-25..TP-26
    ├── crossLanguage.test.ts  # TP-27 (fixture corpus byte-equality)
    ├── agUiCompat.test.ts     # TP-28..TP-29 (pinned @ag-ui/core assignability + parse)
    ├── bundle.test.ts         # TP-30..TP-31 (no node:/dep imports; size budget)
    └── _support/
        └── vectors.ts         # shared deterministic input vectors
```

Fixture corpus + generator (also slice 1):

```
sdk/fixtures/cross-language/
├── ag_ui_v1.json              # frozen corpus — NEVER edit in place (D05 discipline)
└── generate_ag_ui.mjs         # reference generator; runs against dist/ builders
```

The TS README and the package description carry the design.md §1.1 display-only notice verbatim (review-standards §1.2; the formal A6.1 docs gate re-runs in slice 3's acceptance subset, but the README content lands here).

> NOTE-TO-ORCHESTRATOR: implementation.md §1.1 does not list `pnpm-workspace.yaml`, but the repo has one and house convention (COV_D04_S1) registers each new TS package there; acceptance A1.1 also references "the workspace's package-manager equivalent used by `sdk/typescript-langchain/`". This slice assumes workspace registration of `sdk/typescript-ag-ui` is in scope as a one-line addition. Flagging rather than silently expanding the implementation.md file list.

## LOCKED surface — quoted verbatim

### design.md §5.1 — Envelope

Every event is an AG-UI `CUSTOM` event-shaped **plain object**:

```json
{
  "type": "CUSTOM",
  "name": "spendguard.<family>.<event>",
  "value": { "...payload per §5.3-§5.7..." },
  "timestamp": 1765843200000
}
```

- `type` — literal `"CUSTOM"` (AG-UI `EventType.CUSTOM`).
- `name` — the AG-UI CUSTOM extension discriminator; one of the five names in
  §5.2. This is the namespace claim: `spendguard.` prefix, lowercase,
  dot-separated.
- `value` — the payload object. All payload keys are ASCII snake_case (matches
  ASP Draft-01 / the audit-chain wire vocabulary).
- `timestamp` — OPTIONAL integer epoch **milliseconds**, the AG-UI base-event
  envelope convention. Builders never read clocks: present iff the caller
  supplied it. `[VERIFY-AT-IMPL: confirm against the pinned @ag-ui/core that
  the BaseEvent optional timestamp field is named `timestamp` and is epoch
  milliseconds; adjust the envelope key here only via design.md revision.]`
- No other envelope keys. In particular AG-UI's optional `rawEvent` is never
  emitted.

### design.md §5.2 — Event names

| Constant | AG-UI CUSTOM `name` | Emitted when |
|---|---|---|
| `budgetSnapshot` | `spendguard.budget.snapshot` | Host app wants to render current budget state (run start, periodic refresh) |
| `reservationCreated` | `spendguard.reservation.created` | Sidecar `reserve` returned ALLOW / ALLOW_WITH_CAPS (SpendGuard `CONTINUE` / `DEGRADE`) |
| `reservationCommitted` | `spendguard.reservation.committed` | `commitEstimated` acked |
| `reservationReleased` | `spendguard.reservation.released` | `release` acked (abort / timeout / cancel path) |
| `decisionDenied` | `spendguard.decision.denied` | Sidecar denied the call pre-dispatch (any deny-class outcome) |

These five strings are the public vocabulary. Renames, additions, or removals
after this spec merges require a re-spec (review-standards §2 P0).

### design.md §5.3 — `spendguard.budget.snapshot` payload

| Key | Type | Req | Source / ASP mapping |
|---|---|---|---|
| `schema_version` | string, literal `"1"` | ✓ | Injected by builder. Soft-schema churn guard for consumers. |
| `budget_id` | string, non-empty | ✓ | ASP `budget_id` (verbatim). |
| `window_instance_id` | string, non-empty | ✓ | SpendGuard wire / ASP `BudgetClaim.window_instance_id` (verbatim). |
| `unit` | string, non-empty | ✓ | ASP `unit` slug (verbatim), e.g. `"usd_micros"`, `"output_token"`. |
| `unit_id` | string | omit-if-empty | Ledger unit-row UUID (HARDEN_D05_UR). See §6 unitId invariant. |
| `remaining_atomic` | decimal string | ✓ | Available capacity. Mirrors substrate `QueryBudgetResult.availableAtomic`; `_atomic` suffix per ASP naming convention. |
| `reserved_atomic` | decimal string | ✓ | Held reservations. Mirrors `QueryBudgetResult.reservedAtomic`. |
| `spent_atomic` | decimal string | ✓ | Committed spend. Mirrors `QueryBudgetResult.committedAtomic`. |
| `as_of` | RFC 3339 string | ✓ | Snapshot time. Mirrors `QueryBudgetResult.asOfSeconds`. |

ASP note: Draft-01 has no budget-snapshot audit event (upstream `query_budget`
is an RPC verb, not an audit suffix), so only the field-name *conventions*
(`budget_id`, `unit`, `*_atomic`) are inherited; the snapshot shape itself is a
D39 display-vocabulary original.

NB: the TS substrate's `client.queryBudget()` is still a §9.4 placeholder
(throws `QUERY_BUDGET_NOT_YET_WIRED`; no `QueryBudget` RPC exists on
`adapter.proto`). Snapshot **inputs therefore come from whatever budget source
the host app has** (seeded config, operator dashboard API, future queryBudget).
The builder does not care; the demo's honesty story is in §9.

### design.md §5.4 — `spendguard.reservation.created` payload

| Key | Type | Req | Source / ASP mapping |
|---|---|---|---|
| `schema_version` | string `"1"` | ✓ | Injected by builder. |
| `decision_id` | string, non-empty | ✓ | ASP common field (verbatim). From `DecisionOutcome.decisionId`. |
| `reservation_id` | string, non-empty | ✓ | ASP `reservation_id` (verbatim). From `DecisionOutcome.reservationIds[0]`. |
| `budget_id` | string, non-empty | ✓ | ASP (verbatim). From the reserve's `BudgetClaim.scopeId` binding. |
| `window_instance_id` | string, non-empty | ✓ | ASP `BudgetClaim.window_instance_id` (verbatim). |
| `unit` | string, non-empty | ✓ | ASP `unit` (verbatim). |
| `unit_id` | string | omit-if-empty | §6 unitId invariant. |
| `amount_atomic_reserved` | decimal string | ✓ | ASP `audit.reserve` required field (verbatim). The reserved claim amount. |
| `decision` | `"ALLOW"` \| `"ALLOW_WITH_CAPS"` | ✓ | ASP decision enum (verbatim). Mapping from SpendGuard wire: `CONTINUE` → `ALLOW`; `DEGRADE` → `ALLOW_WITH_CAPS` (per ASP Draft-01 §2: DEGRADE is the ALLOW_WITH_CAPS pattern). |
| `ttl_expires_at` | RFC 3339 string | ✓ | ASP `audit.reserve` `ttl_expires_at` (verbatim). From `DecisionOutcome.ttlExpiresAtSeconds`. |
| `reason_codes` | string[] | optional, omit-if-absent/empty | ASP common field (verbatim). From `DecisionOutcome.reasonCodes`. |
| `matched_rule_ids` | string[] | optional, omit-if-absent/empty | SpendGuard audit vocabulary (`DecisionOutcome.matchedRuleIds`). |
| `run_id` | string | omit-if-empty | Substrate-derived run correlation ID. |
| `llm_call_id` | string | omit-if-empty | Substrate-derived per-call ID. |
| `event_time` | RFC 3339 string | ✓ | ASP common `event_time` (verbatim) — caller-supplied; builders never read clocks. |

`caps` (ASP `audit.reserve` when ALLOW_WITH_CAPS) is deliberately NOT in
v0.1.0: the upstream cap-type vocabulary is deferred to `budget_reservation`
v0.2 and SpendGuard's DEGRADE patch is a JSON mutation, not a caps list.
When `decision = "ALLOW_WITH_CAPS"` the display story is the decision value +
`reason_codes`; a `caps` field addition is a design.md revision.

### design.md §5.5 — `spendguard.reservation.committed` payload

| Key | Type | Req | Source / ASP mapping |
|---|---|---|---|
| `schema_version` | string `"1"` | ✓ | Injected by builder. |
| `decision_id` | string, non-empty | ✓ | ASP common (verbatim). |
| `reservation_id` | string, non-empty | ✓ | ASP `audit.commit` required field (verbatim). |
| `budget_id` | string, non-empty | ✓ | ASP (verbatim). |
| `window_instance_id` | string, non-empty | ✓ | ASP (verbatim). |
| `unit` | string, non-empty | ✓ | ASP (verbatim). |
| `unit_id` | string | omit-if-empty | §6 unitId invariant. |
| `amount_atomic_estimated` | decimal string | ✓ | **SpendGuard extension, documented delta.** SpendGuard's only commit lane today is `CommitEstimated` (ASP Draft-01 §8 known delta — the observed-amount lane is backlog). Display events tell the truth: this amount is the estimated reconciliation, named distinctly from ASP's `amount_atomic_observed` so no consumer mistakes it for provider-reported usage. |
| `amount_atomic_observed` | decimal string | optional, omit-if-absent | ASP `audit.commit` field (verbatim), **reserved**: emitted only once SpendGuard ships the observed-amount commit lane. v0.1.0 builders accept and emit it when supplied (forward-compat) but the demo never sets it. |
| `outcome` | `"SUCCESS"` \| `"PROVIDER_ERROR"` \| `"CLIENT_TIMEOUT"` \| `"RUN_ABORTED"` | ✓ | SpendGuard `CommitEstimatedRequest.outcome` enum (verbatim, all four values). |
| `run_id` | string | omit-if-empty | Substrate-derived. |
| `llm_call_id` | string | omit-if-empty | Substrate-derived. |
| `event_time` | RFC 3339 string | ✓ | ASP common (verbatim). |

### design.md §5.6 — `spendguard.reservation.released` payload

| Key | Type | Req | Source / ASP mapping |
|---|---|---|---|
| `schema_version` | string `"1"` | ✓ | Injected by builder. |
| `reservation_id` | string, non-empty | ✓ | ASP `audit.release` required field (verbatim). |
| `decision_id` | string | omit-if-empty | ASP common. Optional here because the adapter-wire `ReleaseRequest` does not carry it. |
| `reason_codes` | string[], ≥ 1 entry | ✓ | ASP `audit.release`: "Reason for release goes in `reason_codes`" (verbatim). Use the Draft-01 §4 examples: `provider_error`, `client_timeout`, `run_cancelled`, … |
| `ledger_transaction_id` | string | omit-if-empty | SpendGuard `ReleaseOutcome.ledgerTransactionId`. |
| `run_id` | string | omit-if-empty | Substrate-derived. |
| `llm_call_id` | string | omit-if-empty | Substrate-derived. |
| `event_time` | RFC 3339 string | ✓ | ASP common (verbatim). |

### design.md §5.7 — `spendguard.decision.denied` payload

| Key | Type | Req | Source / ASP mapping |
|---|---|---|---|
| `schema_version` | string `"1"` | ✓ | Injected by builder. |
| `decision_id` | string, non-empty | ✓ | ASP common (verbatim). From `DecisionDenied.decisionId`. |
| `decision` | string, literal `"DENY"` | ✓ | ASP decision enum (verbatim). Injected by builder — every deny-class SpendGuard outcome is ASP `DENY`. |
| `denied_kind` | `"DENY"` \| `"STOP"` \| `"STOP_RUN_PROJECTION"` \| `"SKIP"` \| `"APPROVAL_REQUIRED"` | ✓ | SpendGuard sidecar decision-outcome taxonomy (finer-grained than ASP's single `DENY`; SpendGuard extension, documented). Mapping from SDK error classes: `DecisionDenied` → `DENY`; `DecisionStopped` → `STOP` (or `STOP_RUN_PROJECTION` when distinguishable); `DecisionSkipped` → `SKIP`; `ApprovalRequired` → `APPROVAL_REQUIRED`. `[VERIFY-AT-IMPL: whether the TS/Python `DecisionStopped` error exposes the wire decision enum value to distinguish STOP vs STOP_RUN_PROJECTION; if not, callers emit `STOP` and the projection nuance stays visible in `reason_codes` (`RUN_BUDGET_PROJECTION_EXCEEDED`, `RUN_STEPS_EXCEEDED`, `RUN_CEILING`, `RUN_DRIFT_DETECTED`).]` |
| `reason_codes` | string[], **≥ 1 entry** | ✓ | ASP common (verbatim). From `DecisionDenied.reasonCodes` — contract-DSL rule reason codes plus engine `RUN_*` codes. When `denied_kind = "APPROVAL_REQUIRED"`, the array MUST include the literal `"approval_required"` (ASP Draft-01 §2: approval-required is DENY + that reason code); the builder validates and throws if missing — it does NOT silently append. |
| `matched_rule_ids` | string[] | optional, omit-if-absent/empty | From `DecisionDenied.matchedRuleIds`. |
| `budget_id` | string | omit-if-empty | ASP (verbatim). Optional — a deny can fire before budget binding. |
| `window_instance_id` | string | omit-if-empty | ASP (verbatim). |
| `unit` | string | omit-if-empty | ASP (verbatim). |
| `unit_id` | string | omit-if-empty | §6 unitId invariant. |
| `run_id` | string | omit-if-empty | Substrate-derived. |
| `llm_call_id` | string | omit-if-empty | Substrate-derived. |
| `event_time` | RFC 3339 string | ✓ | ASP common (verbatim). |

### design.md §6 — unitId invariant (HARDEN_D05_UR)

Any payload that references a unit MUST carry `unit_id` when the caller has it.
**Never emit an empty `unit_id`**: if the input `unitId` is `undefined`/`None`
or the empty string, the builder **omits the key entirely** — documented here
and asserted by tests (tests.md TP-14/TA-14). The same omit-if-empty rule
applies uniformly to every optional string field (`run_id`, `llm_call_id`,
`decision_id` on released, `budget_id` on denied, …): empty string and absent
are the same thing and serialize identically — this collapse is load-bearing
for cross-language byte-equivalence.

### design.md §7 — Canonical JSON rule (LOCKED)

1. **Encoding**: UTF-8, no BOM, no trailing newline.
2. **Whitespace**: none. Separators are `,` and `:` exactly
   (Python: `json.dumps(obj, ensure_ascii=False, sort_keys=True, separators=(",", ":"))`;
   TS: recursive key-sorted rebuild + `JSON.stringify`).
3. **Key order**: object keys sorted lexicographically by Unicode code point,
   recursively at every nesting level.
4. **Keys**: ASCII-only (`[\x21-\x7e]`), enforced by the serializer (throw on
   violation). This makes Python's code-point sort and JS's UTF-16 code-unit
   sort provably identical for every legal key.
5. **Values**: strings, booleans, integers, arrays, objects only.
   - `null` is forbidden — omit the key instead.
   - Floats / non-finite numbers are forbidden (throw). Integers must satisfy
     `|n| ≤ 2^53 − 1` and `-0` is forbidden. (In practice the only number in a
     D39 event is the envelope `timestamp`; every amount is a decimal string.)
   - Strings must be valid Unicode; unpaired surrogates are rejected (throw).
     Both `json.dumps(ensure_ascii=False)` and `JSON.stringify` then agree on
     the escape set: `"` `\` and the C0 controls (shorthand `\b \t \n \f \r`,
     `\u00XX` otherwise); all other characters pass through as raw UTF-8.
6. **Array order**: preserved as given (arrays are caller-ordered, e.g.
   `reason_codes`).

`canonicalEventJson(event)` / `canonical_event_json(event)` apply this rule to
the **whole envelope** (`type`, `name`, `value`, `timestamp?`). The SSE helper
is defined on top of it:

```
encodeSse(e) === "data: " + canonicalEventJson(e) + "\n\n"
```

**Fixture corpus**: `sdk/fixtures/cross-language/ag_ui_v1.json`, minted in
slice 1 by the TS reference generator, frozen, then consumed byte-for-byte by
both the TS and Python suites (D05 corpus discipline: never edit in place; new
vectors → `ag_ui_v2.json`). ≥ 20 vectors per tests.md §4.

### design.md §8.1 — Public API surface, TypeScript (verbatim signatures)

```ts
// ── Names ───────────────────────────────────────────────────────────────
export const SPENDGUARD_AG_UI_EVENT_NAMES = {
  budgetSnapshot: "spendguard.budget.snapshot",
  reservationCreated: "spendguard.reservation.created",
  reservationCommitted: "spendguard.reservation.committed",
  reservationReleased: "spendguard.reservation.released",
  decisionDenied: "spendguard.decision.denied",
} as const;

export type SpendGuardAgUiEventName =
  (typeof SPENDGUARD_AG_UI_EVENT_NAMES)[keyof typeof SPENDGUARD_AG_UI_EVENT_NAMES];

// ── Envelope ────────────────────────────────────────────────────────────
export interface SpendGuardAgUiEvent {
  readonly type: "CUSTOM";
  readonly name: SpendGuardAgUiEventName;
  readonly value: Readonly<Record<string, unknown>>;
  readonly timestamp?: number; // integer epoch ms; present iff caller supplied
}

export interface BuildContext {
  /** AG-UI envelope timestamp (integer epoch milliseconds). Builders never
   *  read clocks: omitted from the event when not provided. */
  timestampMs?: number;
}

// ── Builder inputs (camelCase per TS house style; builders map to the
//    snake_case payload keys locked in design.md §5) ─────────────────────
export interface BudgetSnapshotInput {
  budgetId: string;
  windowInstanceId: string;
  unit: string;
  unitId?: string;
  remainingAtomic: string;
  reservedAtomic: string;
  spentAtomic: string;
  asOf: string; // RFC 3339
}

export interface ReservationCreatedInput {
  decisionId: string;
  reservationId: string;
  budgetId: string;
  windowInstanceId: string;
  unit: string;
  unitId?: string;
  amountAtomicReserved: string;
  decision: "ALLOW" | "ALLOW_WITH_CAPS";
  ttlExpiresAt: string; // RFC 3339
  reasonCodes?: readonly string[];
  matchedRuleIds?: readonly string[];
  runId?: string;
  llmCallId?: string;
  eventTime: string; // RFC 3339
}

export interface ReservationCommittedInput {
  decisionId: string;
  reservationId: string;
  budgetId: string;
  windowInstanceId: string;
  unit: string;
  unitId?: string;
  amountAtomicEstimated: string;
  amountAtomicObserved?: string; // reserved — future observed commit lane
  outcome: "SUCCESS" | "PROVIDER_ERROR" | "CLIENT_TIMEOUT" | "RUN_ABORTED";
  runId?: string;
  llmCallId?: string;
  eventTime: string; // RFC 3339
}

export interface ReservationReleasedInput {
  reservationId: string;
  decisionId?: string;
  reasonCodes: readonly string[]; // ≥ 1 entry
  ledgerTransactionId?: string;
  runId?: string;
  llmCallId?: string;
  eventTime: string; // RFC 3339
}

export interface DecisionDeniedInput {
  decisionId: string;
  deniedKind: "DENY" | "STOP" | "STOP_RUN_PROJECTION" | "SKIP" | "APPROVAL_REQUIRED";
  reasonCodes: readonly string[]; // ≥ 1 entry; APPROVAL_REQUIRED ⇒ must include "approval_required"
  matchedRuleIds?: readonly string[];
  budgetId?: string;
  windowInstanceId?: string;
  unit?: string;
  unitId?: string;
  runId?: string;
  llmCallId?: string;
  eventTime: string; // RFC 3339
}

// ── Builders (pure) ─────────────────────────────────────────────────────
export function buildBudgetSnapshot(
  input: BudgetSnapshotInput, ctx?: BuildContext): SpendGuardAgUiEvent;
export function buildReservationCreated(
  input: ReservationCreatedInput, ctx?: BuildContext): SpendGuardAgUiEvent;
export function buildReservationCommitted(
  input: ReservationCommittedInput, ctx?: BuildContext): SpendGuardAgUiEvent;
export function buildReservationReleased(
  input: ReservationReleasedInput, ctx?: BuildContext): SpendGuardAgUiEvent;
export function buildDecisionDenied(
  input: DecisionDeniedInput, ctx?: BuildContext): SpendGuardAgUiEvent;

// ── Serialization + transport helper ────────────────────────────────────
export function canonicalEventJson(event: SpendGuardAgUiEvent): string;
export function encodeSse(event: SpendGuardAgUiEvent): string;
export type AgUiEmit = (event: SpendGuardAgUiEvent) => void | Promise<void>;

// ── Errors / version ────────────────────────────────────────────────────
export class AgUiEventValidationError extends Error {
  readonly field: string; // payload-key-style name of the offending field
}
export { VERSION } from "./version.js";
```

No other exports. No default export. NO re-export of `@spendguard/sdk` or
`@ag-ui/core` symbols (the package depends on neither at runtime).

### implementation.md §4.2 — validator rules (regexes are part of the cross-language contract)

| Validator | Rule | Throws |
|---|---|---|
| `requireNonEmpty(field, s)` | `typeof s === "string" && s.length > 0` (no trimming — exactness) | `AgUiEventValidationError(field)` |
| `requireAtomic(field, s)` | `/^(0\|[1-9][0-9]*)$/` — non-negative atomic decimal string, no sign, no leading zeros | same |
| `requireRfc3339(field, s)` | regex gate: `/^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(\.\d+)?(Z\|[+-]\d{2}:\d{2})$/` — format check only, no date parsing libs | same |
| `requireStringArray(field, a, {minLen})` | array of non-empty strings; `minLen` 0 or 1 per design §5 | same |
| `requireSafeInteger(field, n)` | `Number.isSafeInteger(n) && !Object.is(n, -0) && n >= 0` | same |
| `optionalEntry(field, s)` | returns `{ [field]: s }` when `s` is a non-empty string, else `{}` | never |

Python `_validate.py` mirrors these rules and regexes character-for-character
(the regexes are part of the cross-language contract — a string accepted by
one language and rejected by the other is a fixture-level break).

### implementation.md §3 — Bundle budget (build failure, not warning)

| Artifact | Cap |
|---|---|
| `dist/index.js` minified | **≤ 8 KB** |
| `dist/index.js` gzipped | **≤ 3 KB** |
| `npm pack` tarball | ≤ 25 KB |

### Builder special rules (implementation.md §4.1, verbatim comment block)

```
// buildDecisionDenied:
//   value.decision = "DENY"  (injected literal)
//   if (input.deniedKind === "APPROVAL_REQUIRED" &&
//       !input.reasonCodes.includes("approval_required")) {
//     throw new AgUiEventValidationError("reason_codes",
//       'denied_kind APPROVAL_REQUIRED requires reason_codes to include "approval_required" (ASP Draft-01 §2)');
//   }
//
// buildReservationCreated / buildDecisionDenied:
//   reason_codes / matched_rule_ids: emitted only when provided AND non-empty
//   (design §5.4/§5.7 omit-if-absent/empty), EXCEPT denied.reason_codes and
//   released.reason_codes which are REQUIRED with ≥ 1 entry.
```

## VERIFY-AT-IMPL markers owned by this slice

From review-standards §8 (slice column = 1). Each must be resolved with an actually-verified value recorded in this slice doc at impl time; inventing a value is a P0.

| Marker | Where | Pre-declared fallback |
|---|---|---|
| AG-UI BaseEvent `timestamp` field name + epoch-ms semantics | design §5.1 | If the pinned `@ag-ui/core` disagrees, do NOT change the envelope key in code — escalate for a design.md revision (per the §5.1 marker text: "adjust the envelope key here only via design.md revision"). |
| `@ag-ui/core` exact devDep pin + peer-range floor | design §10.2, impl §2 | Floor = "the oldest 0.0.x whose CustomEvent shape matches §5.1, checked when pinning the devDep" (design §10.2). If no published 0.0.x matches the §5.1 shape, that is a design.md-revision escalation, not a local fix. |
| `prepublishOnly` cross-package script path viability | impl §2 | "copy the script locally if the cross-package reference is brittle in CI" (impl §2 marker text). |
| `@ag-ui/core` CustomEvent type path + runtime schema existence | tests TP-28/29 | TP-29's own fallback: if no runtime schema is exported, "this test asserts the envelope key set `{type,name,value}` ⊆ pinned package's parsed shape" (tests.md TP-29). |
| AG-UI SSE frame shape (data-only) | design §7 | Resolved here, consumed by slice 3. Per design §7: "the canonical-JSON rule itself is locked regardless" — the `encodeSse` framing (`"data: " + canonical + "\n\n"`) does not move; the resolution only informs slice-3 demo consumers. |

### Marker resolutions — recorded at impl time (2026-06-10)

| Marker | Verified value | Primary / fallback | Evidence |
|---|---|---|---|
| AG-UI BaseEvent `timestamp` field name + epoch-ms semantics | Field is named exactly `timestamp`, optional number (`z.ZodOptional<z.ZodNumber>` on `BaseEventSchema` AND `CustomEventSchema`), epoch **milliseconds** | **PRIMARY** — design §5.1 envelope key stands; no design.md revision needed | Inspected published `@ag-ui/core@0.0.56` and `@ag-ui/core@0.0.27` `dist/index.d.ts`; epoch-ms confirmed by AG-UI first-party docs middleware example (`docs/sdk/js/client/middleware.mdx` in ag-ui-protocol/ag-ui stamps `Date.now()` into `timestamp`) and `@ag-ui/proto` int64 wire encoding. Recorded in `src/events.ts` header comment. |
| `@ag-ui/core` exact devDep pin + peer-range floor | devDep pin `0.0.56` (npm dist-tag `latest` as of 2026-06-10); peer range `>=0.0.27 <0.1.0` | **PRIMARY** — `0.0.27` is the OLDEST published 0.0.x on the registry and its `CustomEventSchema` already matches §5.1 (`{type: literal CUSTOM, name: string, value: any}` + optional `timestamp`/`rawEvent`) | `npm view @ag-ui/core versions` (registry list starts at 0.0.27); tarball type inspection of both versions. Pins live in `package.json`. |
| `prepublishOnly` cross-package script path viability | Cross-package reference is **brittle/wrong**: `../typescript-langchain/scripts/prepublish.sh` `cd`s into its OWN package dir and would version-check/build `@spendguard/langchain`, not this package | **FALLBACK** (pre-declared): scripts copied locally — `scripts/prepublish.sh` + `scripts/version-check.sh` + `scripts/size-budget.sh` | `sdk/typescript-langchain/scripts/prepublish.sh` line `cd "$(dirname "$0")/.."`. Recorded in `scripts/prepublish.sh` header comment. |
| `@ag-ui/core` CustomEvent type path + runtime schema existence | Type path `import("@ag-ui/core").CustomEvent` (= `z.infer<typeof CustomEventSchema>`); runtime schema `CustomEventSchema` **IS exported** → TP-29 uses the primary runtime-parse path, not the key-set fallback. Caveat: TS string enums are nominal — the literal `"CUSTOM"` is not assignable to `EventType.CUSTOM` although the runtime value is the identical string; TP-28 asserts structural assignability via a `type`-only re-tag (compile-time) + `EventType.CUSTOM === built.type` (runtime identity), the maximal honest check TS permits | **PRIMARY** (TP-29); TP-28 documented enum-nominality form | `dist/index.d.ts` of the exact pin; compile probe under `tsconfig.tests.json` (direct annotation fails ONLY on the `type` key). Recorded in `tests/agUiCompat.test.ts` header comment. |
| AG-UI SSE frame shape (data-only) | Confirmed data-only: the `@ag-ui/client@0.0.56` SSE parser splits frames on `\n\n`, consumes ONLY `data:`-prefixed lines (strips prefix + one optional leading space, joins multi-data lines, `JSON.parse`s), and ignores `event:`/`id:` lines entirely | **PRIMARY** — `encodeSse` framing unchanged; slice 3 demo consumers may rely on data-only frames | `@ag-ui/client@0.0.56` `dist/index.js` parser source. Recorded in `src/sse.ts` header comment. |

## Test/verification plan

Delivers TP-01..TP-31 plus the corpus (tests.md §10: "TP-01..TP-31; corpus `ag_ui_v1.json` + generator; TP-27 green against the freshly minted corpus").

| ID | One-line description (tests.md) |
|---|---|
| TP-01 | Each of the 5 builders returns `type: "CUSTOM"` and the exact §5.2 `name` string |
| TP-02 | Purity: deep-equal inputs → deep-equal events; 100 repeated calls → identical `canonicalEventJson` bytes |
| TP-03 | Clock-free: builders succeed with `Date.now` monkeypatched to throw |
| TP-04 | `timestampMs` provided → envelope `timestamp` equals it exactly; omitted → key ABSENT (not null, not 0) |
| TP-05 | `buildBudgetSnapshot` payload = exactly the §5.3 key set; `schema_version === "1"` |
| TP-06 | `buildReservationCreated` matches §5.4 key set; `decision` passes through `"ALLOW"` / `"ALLOW_WITH_CAPS"` verbatim |
| TP-07 | `buildReservationCommitted` matches §5.5; all four `outcome` values accepted verbatim; a 5th throws |
| TP-08 | Emits `amount_atomic_estimated`; `amount_atomic_observed` ABSENT unless supplied, verbatim when supplied |
| TP-09 | `buildReservationReleased` matches §5.6; Draft-01 §4 example `reason_codes` round-trip verbatim |
| TP-10 | `buildDecisionDenied` injects literal `decision: "DENY"` regardless of `deniedKind`; §5.7 key set |
| TP-11 | All five `deniedKind` values accepted verbatim as `denied_kind`; a 6th throws |
| TP-12 | Inputs not mutated; returned event frozen (`Object.isFrozen`) |
| TP-13 | created `reason_codes`/`matched_rule_ids`: non-empty → verbatim caller order; empty/omitted → key ABSENT |
| TP-14 | unit_id omission (P0, HARDEN_D05_UR): `undefined`/`""` → no key; non-empty → verbatim; on snapshot, created, committed, AND denied; `"unit_id":""` never appears corpus-wide |
| TP-15 | Empty required string (parameterized per builder/field) → `AgUiEventValidationError` with `field` naming the payload key |
| TP-16 | `requireAtomic` rejects `""`, `"-1"`, `"1.5"`, `"01"`, `"1e3"`, `" 1"`, `"+1"`; accepts `"0"`, `"1"`, `"100000"`, 40-digit string |
| TP-17 | RFC 3339 gate rejects `"2026-06-10"`, `"yesterday"`, `""`, epoch ints; accepts the two valid forms |
| TP-18 | Denied taxonomy: `reasonCodes: []` throws; APPROVAL_REQUIRED without `"approval_required"` throws citing ASP Draft-01 §2; with it → builds, no silent append, order preserved |
| TP-19 | released `reasonCodes: []` throws (≥ 1 required); created `reasonCodes` may be omitted |
| TP-20 | Recursive key sorting incl. nested objects inside `value` |
| TP-21 | No `": "`, `", "`, newline, trailing whitespace; UTF-8 without BOM |
| TP-22 | Unicode passthrough: CJK/emoji/astral raw UTF-8, not `\uXXXX`; control chars escape identically |
| TP-23 | Rejections: float, `NaN`/`Infinity`, `-0`, int > 2^53−1, `null` value, non-ASCII key, unpaired surrogate |
| TP-24 | `canonicalEventJson` idempotent: parse → canonicalize of own output is byte-identical |
| TP-25 | `encodeSse(e) === "data: " + canonicalEventJson(e) + "\n\n"` exact, every event type |
| TP-26 | Frame contains no interior newline |
| TP-27 | Every corpus vector: `canonicalEventJson(build*(inputs, ctx))` == `expected_canonical_json` byte-for-byte; `encodeSse` == `expected_sse` |
| TP-28 | Type-level assignability to the pinned `@ag-ui/core` `CustomEvent` under exact devDep pin |
| TP-29 | Runtime parse through @ag-ui/core's exported CUSTOM schema if any (fallback per marker table) |
| TP-30 | `dist/index.js` contains no `node:` import, no `require(`, no `@ag-ui/core` / `@spendguard/sdk` import |
| TP-31 | Size budget: minified ≤ 8 KB, gz ≤ 3 KB |

Corpus vector matrix (tests.md §6, ≥ 20 vectors): minimal + maximal per builder; `unit_id` absent vs present; `timestamp_ms` absent vs present incl. `timestamp_ms: 0` ("0 ≠ absent"); Unicode set (CJK + emoji + astral in `reason_codes`, U+001F in a `matched_rule_ids` entry); one vector per `denied_kind` (5) incl. the APPROVAL_REQUIRED + `"approval_required"` vector; one per `outcome` (4) plus one with `amount_atomic_observed`; a 40-digit `remaining_atomic`.

## Acceptance gates (slice subset per acceptance.md §8)

From `sdk/typescript-ag-ui/` unless noted:

```bash
# A1.1  install
npm install            # (or the workspace package-manager equivalent used by sdk/typescript-langchain/)
# A1.2  lint
npm run lint           # biome zero diagnostics
# A1.3  typecheck (src AND tests tsconfigs)
npm run typecheck
# A1.4  build
npm run build          # tsup emits dist/index.js + dist/index.d.ts
# A1.5  size budget (breach = non-zero exit)
npm run size
# A1.6  zero runtime deps / no node builtins / AG-UI never imported at runtime
grep -E "from ['\"](node:|@ag-ui/|@spendguard/sdk)" -r src/ || true
grep -cE "node:|@ag-ui/core" dist/index.js          # expect 0
# A1.7  zero deps + optional peer locked
cat package.json | python3 -c "import json,sys; p=json.load(sys.stdin); assert 'dependencies' not in p or not p['dependencies']; assert p['peerDependenciesMeta']['@ag-ui/core']['optional'] is True; print('OK')"

# A2.1  full suite + coverage floor (≥ 92 % stmt / 88 % branch)
npm run test
# A2.2  TP-01..TP-19
npx vitest run tests/builders.test.ts tests/validate.test.ts
# A2.3  TP-20..TP-26
npx vitest run tests/canonical.test.ts tests/sse.test.ts
# A2.4  TP-28..TP-31 under the EXACT pinned @ag-ui/core devDep
npx vitest run tests/agUiCompat.test.ts tests/bundle.test.ts

# A3.1  corpus exists, committed, ≥ 20 vectors, matrix satisfied (spot-check named vectors)
ls -la ../fixtures/cross-language/ag_ui_v1.json
# A3.2  TP-27 green — TS == corpus byte-for-byte
npx vitest run tests/crossLanguage.test.ts
# A3.4  empty unit_id never serialized (expect 0)
grep -c '"unit_id":""' ../fixtures/cross-language/ag_ui_v1.json
# A3.5  manual: 3 random vectors recomputed via python3 json.dumps(..., sort_keys=True, separators=(',',':')) == expected_canonical_json

# A4.1  packed-tarball export surface — exactly:
#   AgUiEventValidationError,SPENDGUARD_AG_UI_EVENT_NAMES,VERSION,buildBudgetSnapshot,
#   buildDecisionDenied,buildReservationCommitted,buildReservationCreated,
#   buildReservationReleased,canonicalEventJson,encodeSse
npm pack   # then install tarball in scratch dir and list Object.keys of the module
# A4.2  tarball content: only dist + README + LICENSE_NOTICES + CHANGELOG; ≤ 25 KB
tar -tzf spendguard-ag-ui-0.1.0.tgz | grep -E "src/|tests/|node_modules"   # expect empty

# A7.2 (TS half)  no hashing
grep -rE "blake2b|createHash|hashlib|crypto\.subtle" src/                   # expect empty
# A7.4 (TS half)  purity
grep -rn "Date.now\|Math.random\|process.env" src/                          # expect empty
```

## Anti-scope (NOT in this slice)

- **No Python files** — `spendguard.integrations.ag_ui` is slice 2 entirely (review-standards §12: "slice 1: no demo or Python files").
- **No demo, no overlay, no Makefile branches, no `verify_sse.py`** — slice 3.
- **No docs-site page, no repo-root README row, no repo-root / sdk/python CHANGELOG entries** — slice 3 (the package-local `sdk/typescript-ag-ui/README.md` + `CHANGELOG.md` DO land here).
- **No upstream ag-ui repo contribution** — vocabulary registration is follow-on outreach only (design §3); no upstream PRs/issues from D39 slices (acceptance A6.7).
- **No browser UI / rendering components** (design §3).
- **No gating/enforcement claims anywhere** — display-only, P0 (design §1.1, review-standards §1); the package can neither grant nor deny spend and no artifact may imply otherwise.
- **No queryBudget RPC work** — `client.queryBudget()` stays a D05 §9.4 placeholder; D39 never touches it (design §5.3 NB).
- **No edits** to `sdk/typescript/src/**`, `proto/**`, existing adapters, or the frozen D05 corpus `sdk/fixtures/cross-language/v1.json` (implementation §11).
- **No ID minting / hashing** — every ID is an input (design §11.6, review-standards §3).
- **No npm publish / git tag** — `0.1.0` is prepared only; publish is a release decision (acceptance §9).

## Backlinks

- [`design.md`](../../specs/coverage/D39_ag_ui/design.md) — §5 vocabulary, §6 unitId, §7 canonical, §8.1 API, §10 pinning, §12 slice plan
- [`implementation.md`](../../specs/coverage/D39_ag_ui/implementation.md) — §1.1 layout, §2 package.json, §3 budget, §4 TS skeletons
- [`tests.md`](../../specs/coverage/D39_ag_ui/tests.md) — §1 coverage, §2-§8 TP definitions, §6 corpus matrix, §10 mapping
- [`acceptance.md`](../../specs/coverage/D39_ag_ui/acceptance.md) — §1-§4, §7, §8 slice subset
- [`review-standards.md`](../../specs/coverage/D39_ag_ui/review-standards.md) — §1-§6, §8 marker table, §12 sign-off
