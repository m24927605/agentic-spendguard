# D39 — AG-UI spend-event family (`@spendguard/ag-ui` + `spendguard.integrations.ag_ui`)

**Status:** Spec — LOCKED 2026-06-10. Coverage deliverable D39.
**Parent strategy:** [`framework-coverage-2026-06.md`](../../../strategy/framework-coverage-2026-06.md).
**Upstream contracts:**
- [`D05_ts_sdk_substrate/design.md`](../D05_ts_sdk_substrate/design.md) — D39 consumes substrate-derived IDs; it derives NONE of its own.
- [`docs/specs/agent-spend-protocol/draft-01.md`](../../agent-spend-protocol/draft-01.md) — ASP Draft-01 field vocabulary is reused verbatim wherever a concept overlaps (§5 mapping tables below).

**Owner sub-agent:** Frontend Developer (TS slice), Backend Developer (Python slice).

> **LOCKED design.md trumps slice docs.** Where a slice doc disagrees with this
> file, this file wins (coverage build-plan §1.2 P0; D05/7 slice-author bug
> pattern). Schema text in §5 is a verbatim contract — slices copy it exactly.

## 1. Problem and positioning

AG-UI ([ag-ui-protocol/ag-ui](https://github.com/ag-ui-protocol/ag-ui), MIT,
CopilotKit-governed) is the event-based protocol between an agent backend and
its frontend. It has first-party integrations across LangGraph, CrewAI,
Microsoft Agent Framework, Google ADK, AWS Strands, Mastra, Pydantic AI, Agno,
LlamaIndex, and AG2; AWS Bedrock AgentCore Runtime ships managed AG-UI support
(2026-03). `@ag-ui/core` runs ~3.6M npm downloads/month. There is **zero**
cost/budget/usage event prior art on AG-UI (spec + repo issue sweep,
2026-06-10) — SpendGuard is first-mover on the spend vocabulary.

Today a SpendGuard-guarded agent enforces budgets at the sidecar, but the human
watching the agent's frontend sees **nothing**: no remaining budget, no "this
call was reserved then committed", and — worst — a denied call surfaces as an
opaque agent error instead of "budget guardrail denied this call for reason X".
D39 closes the presentation gap.

### 1.1 Display-only — AG-UI CANNOT gate (P0 positioning lock)

**AG-UI does not touch the LLM call path.** It sits between the agent backend
and the frontend, post-decision, presentation-side. SpendGuard can NOT enforce,
gate, deny, reserve, or block anything via AG-UI, and no D39 artifact — code,
comment, README, docs page, demo log line, CHANGELOG entry — may state or imply
otherwise. Enforcement stays where it is: the existing adapters + sidecar
(pre-dispatch reservation over gRPC/UDS, post-call commit/release, signed audit
chain). D39 is a **vocabulary + pure emitters + demo** play: it renders
decisions that the enforcement layer already made.

Every D39 README and docs page MUST carry this notice verbatim:

> **Display-only.** AG-UI events are a presentation surface. SpendGuard
> enforcement happens in the SpendGuard adapters and sidecar before the
> provider call; these events report decisions already made and can neither
> grant nor deny spend.

Violations are review Blockers (review-standards.md §1).

## 2. Goals

1. **Event vocabulary**: a `spendguard.*`-namespaced family of AG-UI `CUSTOM`
   events (AG-UI's sanctioned extension slot: `name` + `value`), with payload
   field names reusing ASP Draft-01 vocabulary wherever the concept overlaps.
   Exact names + schemas locked in §5.
2. **TS emitters**: npm package `@spendguard/ag-ui`, version `0.1.0`,
   Apache-2.0, in-tree at `sdk/typescript-ag-ui/` (same top-level layout as
   `sdk/typescript-langchain/`). Pure builder functions: SpendGuard state in →
   AG-UI `CUSTOM`-shaped plain object out. Plus a canonical serializer and a
   thin transport-agnostic SSE encode helper.
3. **Python emitters**: module `spendguard.integrations.ag_ui` mirroring the
   same builders 1:1 (snake_case), shipping in the next `spendguard-sdk` minor.
4. **Cross-language byte-equivalence**: for identical inputs, the TS and Python
   canonical serializations are byte-identical (§7), proven by a shared fixture
   corpus under `sdk/fixtures/cross-language/`.
5. **Demo that really runs** (demo-as-quality-gate): compose overlay
   `deploy/demo/ag_ui_events/` where an instrumented agent run against the real
   sidecar emits the `spendguard.*` events over an SSE endpoint, and a verify
   gate asserts the exact event sequence + non-empty required fields with the
   same strictness as the SQL gates elsewhere (§9).
6. Dependency-light: builders have **zero runtime dependencies** and do NOT
   hard-depend on `@ag-ui/core` / `ag-ui-protocol` at runtime (§8).

## 3. Non-goals

- **Any enforcement semantics through AG-UI** — see §1.1. Not now, not later;
  the protocol position makes it impossible, not merely out of scope.
- **Browser UI / frontend rendering components** — a CopilotKit/React widget
  consuming these events is a natural follow-on, but the D39 hard gate is the
  asserted SSE/event-log content, not pixels.
- **Upstream contribution** — registering the `spendguard.*` vocabulary with
  the ag-ui docs/repo (they accept community event-family documentation) is a
  **follow-on outreach item only**, tracked outside the D39 build. Do not open
  upstream PRs from D39 slices.
- **Adapter auto-wiring** — automatically emitting these events from inside the
  existing LangChain/DSPy/etc. adapters. v0.1.0 ships builders; the integration
  glue lives in the consumer (and in our demo). Auto-wiring hooks are a
  candidate D39.1.
- **AG-UI `StateSnapshot`/`StateDelta` shared-state binding** — a JSON-Patch
  budget state channel is plausible v0.2; CUSTOM events only in v0.1.0.
- **`RAW` event passthrough** — not needed; CUSTOM is the sanctioned slot.
- **New ID derivation or hashing** — payloads carry IDs already derived by the
  substrate (`decision_id`, `reservation_id`, `llm_call_id`, `run_id`). The
  BLAKE2b cross-language byte-equivalence P0 stays in the substrate (D05 §13);
  D39 builders never hash and never mint IDs.
- **Tier-2 events** (`spendguard.budget.window_rolled`,
  `spendguard.reservation.expired`, `spendguard.approval.pending`, …) — the
  ASP audit taxonomy is larger than the display minimum. v0.1.0 ships the five
  events in §5; additions require a design.md revision.

## 4. Architecture

```
            (enforcement plane — unchanged by D39)
agent code ──► SpendGuard adapter ──► sidecar UDS gRPC ──► ledger + audit chain
                    │
                    │  DecisionOutcome / CommitEstimated ack / DecisionDenied err
                    ▼
            (presentation plane — D39)
            build*() pure builders ──► AG-UI CUSTOM event objects
                    │
                    ├──► encodeSse(event) ──► SSE frame ──► AG-UI frontend
                    └──► any AgUiEmit transport the host app already has
```

Builders are **pure**: no clock reads, no RNG, no env reads, no I/O, no global
state. The same input always yields the same object — this is what makes the
cross-language byte-equivalence contract testable and what keeps the package
trivially safe to call from any host runtime (browser bundles included; no
`node:` imports anywhere in `src/`).

## 5. Event vocabulary — LOCKED (verbatim contract)

### 5.1 Envelope

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

### 5.2 Event names

| Constant | AG-UI CUSTOM `name` | Emitted when |
|---|---|---|
| `budgetSnapshot` | `spendguard.budget.snapshot` | Host app wants to render current budget state (run start, periodic refresh) |
| `reservationCreated` | `spendguard.reservation.created` | Sidecar `reserve` returned ALLOW / ALLOW_WITH_CAPS (SpendGuard `CONTINUE` / `DEGRADE`) |
| `reservationCommitted` | `spendguard.reservation.committed` | `commitEstimated` acked |
| `reservationReleased` | `spendguard.reservation.released` | `release` acked (abort / timeout / cancel path) |
| `decisionDenied` | `spendguard.decision.denied` | Sidecar denied the call pre-dispatch (any deny-class outcome) |

These five strings are the public vocabulary. Renames, additions, or removals
after this spec merges require a re-spec (review-standards §2 P0).

### 5.3 `spendguard.budget.snapshot` — payload

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
`[VERIFY-AT-IMPL: if the queryBudget wire lands before COV_D39_03, the demo
MUST source the snapshot from it instead of seed env vars.]`

### 5.4 `spendguard.reservation.created` — payload

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

### 5.5 `spendguard.reservation.committed` — payload

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

### 5.6 `spendguard.reservation.released` — payload

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

### 5.7 `spendguard.decision.denied` — payload

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

### 5.8 ASP Draft-01 mapping summary

| ASP Draft-01 concept | ASP field/value | D39 reuse |
|---|---|---|
| Budget identity | `budget_id` | verbatim key, all events |
| Window scope | `window_instance_id` | verbatim key |
| Unit slug | `unit` | verbatim key |
| Amount convention | `amount_atomic_*` / `*_atomic` decimal strings | verbatim convention everywhere |
| Decision enum | `ALLOW` / `ALLOW_WITH_CAPS` / `DENY` | verbatim values; SpendGuard `CONTINUE`→`ALLOW`, `DEGRADE`→`ALLOW_WITH_CAPS` |
| Approval pattern | `DENY` + reason code `"approval_required"` | verbatim (validated by builder) |
| Reservation identity | `reservation_id` | verbatim key |
| Decision identity | `decision_id` | verbatim key |
| Rationale | `reason_codes` (string[]) | verbatim key |
| Event clock | `event_time` (RFC 3339) | verbatim key |
| Reserve TTL | `ttl_expires_at` | verbatim key |
| Commit amount | `amount_atomic_observed` | reserved key (emitted only when observed lane exists); `amount_atomic_estimated` is the documented SpendGuard-delta sibling |
| Release reason | reason in `reason_codes` | verbatim |
| NOT reused | `kid`, `audit_event_signature`, CloudEvent envelope | display events are unsigned UI hints, NOT audit records — consumers must never treat them as the audit chain (README requirement, review-standards §1.4) |

## 6. unitId invariant (HARDEN_D05_UR)

Any payload that references a unit MUST carry `unit_id` when the caller has it.
**Never emit an empty `unit_id`**: if the input `unitId` is `undefined`/`None`
or the empty string, the builder **omits the key entirely** — documented here
and asserted by tests (tests.md TP-14/TA-14). The same omit-if-empty rule
applies uniformly to every optional string field (`run_id`, `llm_call_id`,
`decision_id` on released, `budget_id` on denied, …): empty string and absent
are the same thing and serialize identically — this collapse is load-bearing
for cross-language byte-equivalence.

## 7. Canonical JSON — cross-language byte-equivalence rule (LOCKED)

The substrate has **no general-purpose canonical-JSON helper** today
(`computePromptHash` canonicalizes a prompt string; `ids` use `\x1f`-joined
canonical strings; the sidecar signs canonical protobuf — none of these
serialize arbitrary JSON objects). D39 therefore locks its own rule, scoped to
D39 events:

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

`[VERIFY-AT-IMPL: AG-UI's reference SSE transport frame shape — confirm
data-only frames (no `event:`/`id:` fields) against the pinned @ag-ui/core
client before locking demo consumers on it; the canonical-JSON rule itself is
locked regardless.]`

**Fixture corpus**: `sdk/fixtures/cross-language/ag_ui_v1.json`, minted in
slice 1 by the TS reference generator, frozen, then consumed byte-for-byte by
both the TS and Python suites (D05 corpus discipline: never edit in place; new
vectors → `ag_ui_v2.json`). ≥ 20 vectors per tests.md §4.

## 8. Public API surface — LOCKED (verbatim signatures)

### 8.1 TypeScript — `@spendguard/ag-ui` (`sdk/typescript-ag-ui/src/index.ts`)

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

### 8.2 Python — `spendguard.integrations.ag_ui`

```python
# Event-name constants (values identical to §5.2)
BUDGET_SNAPSHOT: str       # "spendguard.budget.snapshot"
RESERVATION_CREATED: str   # "spendguard.reservation.created"
RESERVATION_COMMITTED: str # "spendguard.reservation.committed"
RESERVATION_RELEASED: str  # "spendguard.reservation.released"
DECISION_DENIED: str       # "spendguard.decision.denied"
SPENDGUARD_AG_UI_EVENT_NAMES: tuple[str, ...]  # all five, §5.2 table order

# Frozen-dataclass inputs mirroring §8.1 field-for-field (snake_case):
@dataclass(frozen=True, slots=True)
class BudgetSnapshotInput: ...        # budget_id, window_instance_id, unit,
                                      # unit_id=None, remaining_atomic,
                                      # reserved_atomic, spent_atomic, as_of
@dataclass(frozen=True, slots=True)
class ReservationCreatedInput: ...
@dataclass(frozen=True, slots=True)
class ReservationCommittedInput: ...
@dataclass(frozen=True, slots=True)
class ReservationReleasedInput: ...
@dataclass(frozen=True, slots=True)
class DecisionDeniedInput: ...

# Builders (pure; return a plain dict shaped per §5.1)
def build_budget_snapshot(
    input: BudgetSnapshotInput, *, timestamp_ms: int | None = None
) -> dict[str, Any]: ...
def build_reservation_created(
    input: ReservationCreatedInput, *, timestamp_ms: int | None = None
) -> dict[str, Any]: ...
def build_reservation_committed(
    input: ReservationCommittedInput, *, timestamp_ms: int | None = None
) -> dict[str, Any]: ...
def build_reservation_released(
    input: ReservationReleasedInput, *, timestamp_ms: int | None = None
) -> dict[str, Any]: ...
def build_decision_denied(
    input: DecisionDeniedInput, *, timestamp_ms: int | None = None
) -> dict[str, Any]: ...

# Serialization + transport helper
def canonical_event_json(event: Mapping[str, Any]) -> str: ...
def encode_sse(event: Mapping[str, Any]) -> str: ...
AgUiEmit = Callable[[Mapping[str, Any]], None]

class AgUiEventValidationError(ValueError):
    field: str
```

Optional-field defaults are `None`; `None` and `""` both mean "omit" (§6).

## 9. Demo design — `DEMO_MODE=ag_ui_events` (slice 3)

Overlay `deploy/demo/ag_ui_events/docker-compose.yaml` layered on the base
stack (postgres + sidecar + ledger + outbox-forwarder), same layering as
`deploy/demo/langchain_ts/docker-compose.yaml`. Reuses the langchain_ts demo's
seeded tenant/budget/window/unit IDs and its `counting-stub` provider pattern.

Services:

- `counting-stub` — by-value copy of the langchain_ts overlay's mock OpenAI
  provider (overlay-independence convention).
- `ag-ui-runner` — Node 20 container (langchain-runner staging pattern) running
  `examples/ag-ui-events/index.mjs`:
  1. `SpendGuardClient` connect + handshake on the sidecar UDS
     (`@spendguard/sdk`, real gRPC, real ledger).
  2. Build `spendguard.budget.snapshot` from the demo seed values passed via
     env (`SPENDGUARD_BUDGET_ID` / `SPENDGUARD_WINDOW_INSTANCE_ID` /
     `SPENDGUARD_UNIT_ID` / `SPENDGUARD_DEMO_OPENING_BALANCE_ATOMIC`), with
     `reserved_atomic="0"` / `spent_atomic="0"` — true at fresh-stack start and
     cross-checked against the ledger by the verify gate, so nothing is
     fabricated. (queryBudget RPC does not exist yet — §5.3 note.)
  3. **ALLOW step**: `client.reserve(...)` (substrate-derived
     `run_id`/`llm_call_id`/`decision_id` via `newUuid7` /
     `deriveIdempotencyKey` — D39 derives nothing) → build
     `reservation.created` from the `DecisionOutcome` → HTTP call to
     `counting-stub` → `client.commitEstimated(outcome="SUCCESS")` → build
     `reservation.committed`.
  4. **DENY step**: `client.reserve` with a claim exceeding the remaining
     budget → catch `DecisionDenied` → build `decision.denied` → assert the
     counting-stub hit counter did NOT increase (runner-side proof that the
     deny was enforced by the sidecar, not by AG-UI — the demo log line says
     exactly that).
  5. Serve HTTP on `:8077`: `GET /healthz`; `GET /events` replays all recorded
     frames (`encodeSse` output, in emission order) then closes.
- `sse-probe` — one-shot `curlimages/curl` service (compose `profiles:
  ["verify"]`) that fetches `http://ag-ui-runner:8077/events` to stdout.

**Hard gate** (`make demo-verify-ag-ui-events`, acceptance.md §5):

1. Capture: `sse-probe` output → host temp file.
2. `deploy/demo/ag_ui_events/verify_sse.py` asserts (exact, not `>=`, since
   the capture is one fresh scripted run):
   - exactly 4 `data:` frames; order: `budget.snapshot`,
     `reservation.created`, `reservation.committed`, `decision.denied`;
   - every required field of §5.3-§5.7 present and non-empty;
   - `unit_id` present and non-empty on snapshot/created/committed (demo
     passes `SPENDGUARD_UNIT_ID`);
   - `created.reservation_id == committed.reservation_id` and
     `created.decision_id == committed.decision_id`;
   - `denied.decision == "DENY"` and `denied.reason_codes` non-empty;
   - every frame's payload re-serializes to the identical bytes under the §7
     rule (wire == canonical form);
   - prints `RESERVATION_ID=<uuid>` for step 3.
3. Display↔ledger join: psql asserts the `RESERVATION_ID` from the event
   stream exists in the ledger `reservations` table for the demo tenant —
   display events provably describe real ledger state.
   `[VERIFY-AT-IMPL: exact reservations PK column name for the join.]`
4. `deploy/demo/verify_step_ag_ui_events.sql`: house-style ledger gates
   (`reserve >= 1`, `commit_estimated >= 1`, `denied_decision >= 1` for the
   demo tenant; `>=` per the SQL-gate robustness convention).

A browser UI is OPTIONAL and out of scope; the asserted SSE content is the
gate.

## 10. Version-pinning / 0.x churn isolation — LOCKED

AG-UI is entirely 0.x (`@ag-ui/core` 0.0.x, Python `ag-ui-protocol` 0.1.x,
~15 releases/month, no spec stability policy). Strategy:

1. **Own the types.** §8 interfaces are OURS, structurally matching the AG-UI
   CUSTOM event shape. No AG-UI import appears anywhere in `src/` (TS) or the
   module (Python). Consumers of `@spendguard/ag-ui` never need AG-UI packages
   installed.
2. **Optional peer / extra for typed users.**
   - TS: `peerDependencies: { "@ag-ui/core": "<0.1.0" }` with
     `peerDependenciesMeta: { "@ag-ui/core": { "optional": true } }`.
     `[VERIFY-AT-IMPL: floor of the range — set to the oldest 0.0.x whose
     CustomEvent shape matches §5.1, checked when pinning the devDep.]`
   - Python: `pyproject` extra `ag-ui = ["ag-ui-protocol>=0.1.19,<0.2"]`.
     `[VERIFY-AT-IMPL: latest ag-ui-protocol version at impl time.]`
3. **Pinned compat tests, soft schema.** devDeps pin EXACT versions
   (`@ag-ui/core@0.0.56`-era; `ag-ui-protocol==0.1.19`
   `[VERIFY-AT-IMPL: exact latest pins]`). Compat tests (tests.md TP-2x/TA-2x)
   assert (a) type-level assignability of `SpendGuardAgUiEvent` to the AG-UI
   `CustomEvent` type, (b) runtime validation of built events through AG-UI's
   own parser/schema when one is exported
   `[VERIFY-AT-IMPL: whether @ag-ui/core exports a zod schema for CUSTOM and
   the exact Python import path, e.g. ag_ui.core.CustomEvent]`. If an AG-UI
   release breaks the compat test, OUR wire shape does not move — we ship a
   compat patch or a documented incompatibility note; `schema_version` in every
   payload gives consumers the lever.
4. **Renovate/bump cadence**: compat-test devDep bumps are routine maintenance;
   a compat failure is a P1, not a wire change.

## 11. Locked design decisions

1. **CUSTOM events, not RAW, not State** — CUSTOM is AG-UI's documented
   extension mechanism (`name` + `value`); RAW is provider passthrough and
   State implies a lifecycle we don't own. No re-litigation.
2. **Display-only, forever, by protocol position** — §1.1. Any enforcement
   claim is a review Blocker.
3. **Builders are pure and clock-free** — `event_time` / `as_of` /
   `timestampMs` are inputs. Determinism is what makes byte-equivalence
   testable.
4. **Zero runtime deps; AG-UI packages optional-typed-only** — §10.
5. **ASP names verbatim where concepts overlap; deltas named differently** —
   `amount_atomic_estimated` is deliberately NOT called
   `amount_atomic_observed` (§5.5). Truthful naming over cosmetic alignment.
6. **No new hashing / no ID minting in D39** — IDs arrive as inputs from the
   substrate. BLAKE2b byte-equivalence stays a D05 substrate P0.
7. **Canonical JSON = sorted-keys + UTF-8 + no-whitespace + ASCII keys +
   integer-only numbers** (§7) — locked here because no substrate helper
   exists; D39-scoped, not a repo-wide canonicalization standard.
8. **Empty optional string ≡ absent ⇒ omit key** (§6) — uniform rule, with
   `unit_id` as the named HARDEN_D05_UR invariant.
9. **Demo emits 4 events** (snapshot, created, committed, denied);
   `released` is fixture/unit-tested only in v0.1.0 — the demo's deny step
   never creates a reservation to release, and adding an abort step would
   grow the demo past the display-play size. Documented non-gap.
10. **Five events only in v0.1.0** — additions are a design.md revision, not a
    slice decision.

## 12. Slice plan

| Slice | Title | Size |
|---|---|---|
| `COV_D39_01_ts_pkg` | `@spendguard/ag-ui` package: names/types/builders/canonical/sse/errors + full TS test suite + mint `sdk/fixtures/cross-language/ag_ui_v1.json` (reference generator) + AG-UI pinned compat test | M |
| `COV_D39_02_py_mirror` | `spendguard.integrations.ag_ui` mirror + pyproject `ag-ui` extra + Python suite consuming the frozen fixture corpus byte-for-byte + pinned compat test | M |
| `COV_D39_03_demo_docs` | `deploy/demo/ag_ui_events/` overlay + `examples/ag-ui-events/` runner + `verify_sse.py` + SQL gate + Makefile `DEMO_MODE=ag_ui_events` branches + docs page + CHANGELOGs + repo-root README row | M |

Total: **3 slices** (locked input cap: 2-3). Justification: the vocabulary and
the TS reference implementation must land together (the fixture corpus is
minted from the TS builders, so splitting them buys nothing); Python must
follow the frozen corpus, not co-evolve with it (independent implementation
against frozen bytes IS the cross-language check); demo + docs land last per
demo-as-quality-gate so they exercise both published surfaces. Two slices would
force the Python mirror to review inside one of the other two and lose the
frozen-corpus discipline; four would slice below useful review granularity.

Acceptance gates in [`acceptance.md`](./acceptance.md); slice-by-slice review
checklist in [`review-standards.md`](./review-standards.md).
