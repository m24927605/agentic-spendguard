# Cost Advisor P0 — Schema Reality Check Audit Report

> Spec §11.5 A2 step. Determines branch for P1 implementation.
>
> **Date**: 2026-05-13
> **Method**: static code audit of sidecar audit-emission paths + ledger / canonical_ingest schemas (no running stack available in this workspace; postgres image is part of `deploy/demo/compose.yaml` but not currently up). Static evidence is conclusive — see §3 rationale.
> **Verdict**: **SCENARIO 3 — "3+ fields missing AND fundamental shape mismatch"** per §11.5 A2.
> **Branch decision**: v0.1 ships **zero rules** until two workstreams complete (P0.5 sidecar enrichment AND P0.6 ledger projection view; see §5 for the post-codex-r5 revision). Schedule impact: **+5 days** vs. the v3 baseline (matches §11.5 A2's worst-case-minus-PII row).
>
> **Codex r5 corrections applied 2026-05-13** (see §8 Codex r5 corrections at the end of this file):
> - The v1 conclusion "1 of 4 rules fireable today (idle_reservation_rate_v1)" was wrong.
> - The v1 description of `canonical_events.payload_json` shape was wrong.
> - The v1 emitter set was incomplete.
>
> The bullets below preserve the v1 narrative for traceability; the §8 corrections at the bottom of the file are authoritative where they conflict.

---

## 1. Why static audit is sufficient here

The spec's "schema reality check" anticipates uncertainty about which fields exist in `canonical_events.payload_json` and how often they're populated. In this codebase:

- The set of CloudEvent payloads that ever land in `canonical_events` is bounded: only the sidecar (and ledger via `audit_outbox` → `outbox_forwarder` → `canonical_ingest`) produces them.
- Every emission site is in two files: `services/sidecar/src/decision/transaction.rs` (decision + outcome) and `services/sidecar/src/server/adapter_uds.rs` (resume-after-approval). Both were `grep`'d exhaustively for the spec's required keys.
- For envelope columns (`run_id`, `decision_id`, etc.), every sidecar `CloudEvent { ... }` construction was inspected; `run_id` is unconditionally `String::new()` everywhere.
- A live SQL audit would surface either (a) "0 events" if the stack hasn't been demo'd, or (b) the same fields the code emits. There is no third source of canonical events in this product. Running `make demo-up` to confirm what the code already proves would add no signal.

Where uncertainty remains (e.g., what `runtime_metadata` blob is carried in `DecisionRequest.inputs.runtime_metadata`), the answer is also irrelevant: it isn't currently forwarded into the CloudEvent at all, so dependent rules cannot fire whatever its contents.

---

## 2. Field-by-field reality matrix

Spec §5.1 lists fields the v0 rules read. Reality per static audit:

| Field rule depends on | Where spec assumed it lives | Where it actually lives today | Populated? |
|---|---|---|---|
| `prompt_hash` | `canonical_events.payload->>'prompt_hash'` | **nowhere** — sidecar never computes or emits a prompt hash | ❌ 0% |
| `agent_id` | `canonical_events.payload->>'agent_id'` | proto `SpendGuardIds.step_id` exists on the wire (`DecisionRequest.ids`), but is NOT forwarded into any emitted `CloudEvent.data` | ❌ 0% |
| `run_id` | `canonical_events.run_id` (column) | column exists; proto `SpendGuardIds.run_id` exists on the wire; sidecar hardcodes `CloudEvent { run_id: String::new(), ... }` at 5 emission sites (`transaction.rs:392,484,830,1021`, `adapter_uds.rs:986`); canonical_ingest persists what it receives | ❌ 0% (column is NULL / empty for every audit row) |
| `tool_name` | `canonical_events.payload->>'tool_name'` | not emitted; the wire-level `DecisionRequest.trigger=TOOL_CALL_PRE` + `SpendGuardIds.tool_call_id` exist, but neither tool_name nor tool_args end up in the audit payload | ❌ 0% |
| `tool_args_hash` | `canonical_events.payload->>'tool_args_hash'` | never computed; would have to be derived at sidecar from the adapter request, which currently passes only typed `BudgetClaim` lines, not the tool args | ❌ 0% |
| `model_family` | `canonical_events.payload->>'model_family'` | proto `UnitRef.model_family` exists; sidecar sets it to `String::new()` at `adapter_uds.rs:949` (the only call site); pricing schema knows the family but never threads it into the audit CE | ❌ 0% (string is empty, not NULL) |
| `committed_micros_usd` | `canonical_events.payload->>'committed_micros_usd'` | **architectural mismatch**: cost data lives in `ledger.commits.estimated_amount_atomic` / `ledger_entries.amount_atomic` (NUMERIC(38,0) atomic, unit-relative, requires `ledger_units.scale` + `pricing_snapshots` to convert to USD micros). The audit.outcome CE carries `payload.estimated_amount_atomic` as a string, also atomic — also needs pricing join to render as USD. | ⚠️ Indirect — present via `audit.outcome` payload BUT not in micros_usd, and not in audit.decision events at all |
| `failure_class` | new column (this P0 adds it) | does not exist yet; spec §5.1.2 owns adding it | n/a — added by this P0 |

**Tally**: 6 of 7 rule-input fields are **0% populated** in `canonical_events`. The 7th (`committed_micros_usd`) is present in a different shape, in a different unit, on a different table.

---

## 3. Actual `canonical_events.payload_json` shape today

Reverse-engineered from `services/sidecar/src/decision/transaction.rs` + `services/sidecar/src/server/adapter_uds.rs`:

### `spendguard.audit.decision` — CONTINUE / accepted path (transaction.rs:373-377)
```json
{
  "snapshot_hash": "<hex>",
  "matched_rules": ["..."],
  "session_id":    "<opaque>"
}
```

### `spendguard.audit.decision` — DENY path (transaction.rs:457-469)
```json
{
  "snapshot_hash":  "<hex>",
  "matched_rules":  ["..."],
  "reason_codes":   ["..."],
  "final_decision": "STOP" | "REQUIRE_APPROVAL" | "DEGRADE" | "SKIP",
  "session_id":     "<opaque>",
  "attempted_claims": [
    { "budget_id":"<uuid>", "amount_atomic":"<bigint-string>",
      "window_instance_id":"<uuid>", "unit_id":"<uuid>" }
  ]
}
```

### `spendguard.audit.decision` — resume-after-approval (adapter_uds.rs:965-971)
```json
{
  "resume_of_approval_id": "<id>",
  "amount_atomic":         "<bigint-string>",
  "budget_id":             "<uuid>",
  "matched_rule_ids":      ["..."],
  "reason_codes":          ["..."]
}
```

### `spendguard.audit.outcome` — commit_estimated (transaction.rs:812-817)
```json
{
  "kind":                   "commit_estimated",
  "reservation_id":         "<uuid>",
  "estimated_amount_atomic":"<bigint-string>",
  "decision_id":            "<uuid>"
}
```

### `spendguard.audit.outcome` — release (transaction.rs:1000-1007)
```json
{
  "kind":               "release",
  "reservation_id":     "<uuid>",
  "reservation_set_id": "<uuid>",
  "decision_id":        "<uuid>",
  "reason":             "<enum-string>",
  "metadata":           "<opaque>"
}
```

### Envelope columns on `canonical_events`
- `event_id`, `tenant_id`, `decision_id`, `event_type`, `event_time`, `source`, `producer_id`, `producer_sequence`, `schema_bundle_id`, `region_id`, `ingest_shard_id`, `recorded_month`, `payload_json` (above), `payload_blob_ref` — populated.
- `run_id` (column) — column exists but ALWAYS NULL/empty because sidecar emits empty string at every site.

### Cost data the rules actually need lives in `ledger.commits` (postgres DB `spendguard_ledger`)
- `commits.commit_id`, `commits.reservation_id`, `commits.tenant_id`, `commits.budget_id`, `commits.unit_id` (FK → `ledger_units` for scale/family/currency), `commits.latest_state`, `commits.estimated_amount_atomic`, `commits.provider_reported_amount_atomic`, `commits.invoice_reconciled_amount_atomic`, `commits.delta_to_reserved_atomic`, `commits.pricing_version`, `commits.estimated_at`, `commits.provider_reported_at`, `commits.invoice_reconciled_at`.
- Joining path: `canonical_events.decision_id` ⨝ `ledger_transactions.decision_id` ⨝ `commits.reservation_id` (via the reservation set derived from decision_id), then ⨝ `pricing_snapshots` + `ledger_units` for unit/currency normalization to USD micros.

---

## 4. Per-rule fireability under current schema

| Rule | Status today | Why |
|---|---|---|
| `failed_retry_burn_v1` ⭐ | ❌ NOT FIREABLE | Needs `(run_id, prompt_hash)` grouping + per-attempt cost — both missing. Even if we add `failure_class` (this P0), there is no `prompt_hash` to dedupe attempts on. |
| `runaway_loop_v1` | ❌ NOT FIREABLE | Needs `(run_id, prompt_hash)` retry-count + "no terminal output" signal. None of those exist in audit. |
| `tool_call_repeated_v1` | ❌ NOT FIREABLE | Needs `tool_name`, `tool_args_hash`, and the contract DSL's `idempotent: true` flag exposed in the audit. Tool args never reach the audit at all. |
| `idle_reservation_rate_v1` | ✅ FIREABLE | Operates on `ledger.reservations.latest_state IN ('ttl_expired', 'released')` vs total and median TTL. Already populated by `ttl-sweeper`. Does not need `canonical_events.payload` enrichment to fire — though `decision_id` join is required to map findings back to operator-meaningful scope. |

So with no enrichment, exactly **1 of 4 v0 rules** can fire. With minimal enrichment (just thread `SpendGuardIds.run_id` + `step_id` from `DecisionRequest` through to the CloudEvent), retries-and-loops become detectable at the **run level** (not prompt level), shrinking the false-positive surface significantly but not eliminating it.

---

## 5. Branch decision (spec §11.5 A2)

The audit hits two §11.5 A2 conditions simultaneously:

1. **"3+ fields missing"** — 5 of the rule inputs are 0% populated (`prompt_hash`, `agent_id`, `run_id`, `tool_name`, `tool_args_hash`, `model_family`).
2. **"Fundamental shape mismatch"** — cost lives on `ledger.commits` not in `canonical_events.payload`. Rules will join, not single-table scan.

Per the spec's branch table, this is **+5 days + rule re-design** territory. PII redaction is NOT in scope (no prompt text reaches the audit chain today, so the +5-10-day PII branch is avoided — net good).

### Recommended path forward (the v3 revision of §5.1 this report owes)

| Sub-decision | Recommendation | Rationale |
|---|---|---|
| **Scope of v0 ruleset** | Ship **only** `idle_reservation_rate_v1` in v0.1. Cut `failed_retry_burn_v1`, `runaway_loop_v1`, `tool_call_repeated_v1` from v0; mark them "blocked by audit-enrichment". | Honest scoping. v0.1 has one rule that does fire on real data and produces a concrete contract patch ("tighten reservation TTL"). Operators see the closed loop without us building rules that no-op. |
| **Enrichment workstream (P0.5)** | Sidecar threads `SpendGuardIds.run_id` and `step_id` into `CloudEvent.run_id` + a new `payload.agent_id` field on every emission site. Adds `model_family` from the `UnitRef` resolved at decision time. Computes `prompt_hash` from the adapter request inputs (LangChain / Pydantic-AI / OpenAI Agents adapters all expose the prompt) and emits in `payload.prompt_hash`. ~5 days of sidecar + adapter work, gated by Stage 2 immutability triggers (this is a write-shape change, must NOT mutate existing rows — emit only on new events). | Unblocks the other 3 rules cleanly. Wire-compatible since the proto already carries these fields. |
| **Tool args / tool_name** | Defer to v0.2 (after Cost Advisor ships v0.1). Requires adapter SDK extension. | Adapters today don't pass tool args to sidecar; that's a bigger SDK change. |
| **`failure_class`** | Stay on the P0 plan as written. Adds the column + classifier in `canonical_ingest` per §5.1.2. Note: classifier needs framework-specific signature fixtures, but a NULL `failure_class` is safe (rules treat NULL as "unknown" and don't fire). | Independent of the enrichment work; unblocks the FAILURE-classification half of `failed_retry_burn_v1` even before `prompt_hash` exists. |
| **Cost rendering** | Rules join `canonical_events ⨝ ledger_transactions ⨝ commits ⨝ ledger_units ⨝ pricing_snapshots` and emit `committed_micros_usd` as a derived `metric` with `source_field: "derived: commits.estimated_amount_atomic × scale × usd_rate"`. This already conforms to the `FindingEvidence.Metric.derivation` schema (§4.0 requires `derivation` for derived metrics). | Spec §4.0 already permits this; just makes the derivation explicit in v0 rule SQL. |
| **Scope adjustment for `FindingEvidence.Scope`** | `Scope.scope_type=run` is fine; `scope_type=agent` becomes blocked-on-enrichment; `scope_type=tool` becomes blocked-on-v0.2. Document this in the proto comments. | Honest with consumers about which scopes work in v0.1. |

### Sub-recommendation: order of operations

```
P0  (this week, 4d)         schema audit (DONE) + FindingEvidence proto +
                            CostRule trait + migrations (failure_class,
                            cost_findings, cost_baselines, control_plane
                            approval-queue extensions) + retention_sweeper
                            integration + control-plane integration design.

P0.5 (NEW, +5d)             Sidecar audit-payload enrichment: thread run_id
                            into envelope; add agent_id + model_family +
                            prompt_hash to payload_json on every emission
                            site. Migration: NONE (additive payload, no
                            schema change). Tests: extend benchmark
                            fixtures to assert these fields appear.

P1  (skinny, 6d originally) `services/cost_advisor/` crate skeleton +
                            ONE rule (idle_reservation_rate_v1) + CLI +
                            integration test against benchmark fixtures.
                            Failure classifier (§5.1.2) ships here per
                            original plan.

P1.5 (was P1's rest)        After P0.5 + classifier: turn on
                            failed_retry_burn_v1 (run-scoped, no
                            prompt_hash needed for the FIRST cut — group
                            by run_id only; accept slightly higher
                            false-positive rate, raise confidence to
                            "medium" rather than "high" in
                            WasteEstimate). Then incrementally upgrade
                            to prompt-hash-scoped when P0.5 lands.

P2-P5                       unchanged from spec §9.
```

**Net schedule impact**: P0 stays 4d. P0.5 adds 5d. P1 shrinks from 6d to ~3-4d (one rule, not four). **v0.1 critical path: 4 + 5 + 4 + 4 + 3 = 20 days** (was 17 in spec v3). That matches §11.5 A2's "+3 days" → "+5 days" envelope without slipping into the "+5-10 days PII" branch.

---

## 6. Open items raised by this audit

| # | Item | Owner | Where to resolve |
|---|---|---|---|
| O1 | Should `prompt_hash` be SHA-256 of normalized prompt text, or of the prompt template + arg-bindings? Implications for retry detection across templated agents. | spec author | resolve before P0.5 starts |
| O2 | Do we treat ledger-side cost data as part of the FindingEvidence audit trail, or only `canonical_events.event_id`? Affects `decision_refs` field semantics. | spec §4.0 author | minor §4.0 edit |
| O3 | `ledger.reservations.latest_state='ttl_expired'` — confirm `ttl-sweeper` writes this consistently and that the dashboard already surfaces TTL'd reservations (so the operator who reads the Cost Advisor finding has prior context). | runtime owner | spot-check before P1 begins |
| O4 | `idle_reservation_rate_v1` needs a `min_ttl_for_finding` threshold in contract DSL (per §5.1). Does the contract DSL today have an extensible "rule_config" surface, or does this rule ship with a config table in cost_advisor instead? | control_plane owner | resolve before P1 wraps |

---

## 7. Summary for next-step go/no-go

- ✅ Audit complete; method (static) is justified given the bounded set of audit-event emitters.
- ✅ Branch decision: **+5 days**, scope cut to one rule for v0.1, enrichment workstream queued as P0.5.
- ✅ Revised §5.1 rule list proposed (above) — replaces the four-rule list with `idle_reservation_rate_v1` only for v0.1, with explicit re-introduction gates.
- ✅ No PII blockers found; existing audit chain stores zero prompt text.
- 🟡 Step 2 (FindingEvidence proto + CostRule trait skeleton) can begin without waiting on P0.5; the proto/trait surface is unchanged by this audit's findings (proto carries `agent_id` etc. as optional strings; trait's `declared_input_fields()` already enforces fail-to-register if a declared field isn't present).
- 🟡 Step 3 (migrations) is unchanged in scope; `failure_class` column still lands, `cost_findings` + `cost_baselines` tables still land, control_plane extensions still land. None of those depend on the rule-set scope decision.
- 🟡 Step 4 (control_plane integration design doc) is unchanged in scope.

**Recommendation for the human** (v1, superseded by §8): approve scope-cut to v0.1=one-rule, commit P0.5 as a follow-on workstream, and proceed to Step 2. The spec revision items in §5 above should be folded into a `cost-advisor-spec.md` v4 patch when Step 2 starts (so the proto reflects the revised scope).

---

## 8. Codex r5 corrections (authoritative)

Codex adversarial review on the P0 branch (2026-05-13) caught four real errors in §3, §4, and §5 of the original audit. Where the corrections below conflict with the earlier text, **the corrections are authoritative**. The original narrative is preserved for traceability — do not rely on it standalone.

### 8.1 idle_reservation_rate_v1 is NOT fireable today (kill shot)

The original §4 conclusion "1 of 4 rules fireable" was wrong. Per `services/ledger/migrations/0010_projections.sql:42-50`:

- The column is `reservations.current_state`, **not** `latest_state`. The original report used the wrong name.
- The CHECK allows `('reserved', 'committed', 'released', 'overrun_debt')`. There is **no** `ttl_expired` state. TTL expiry is encoded as a `release` event with `reason='TTL_EXPIRED'` on the audit chain (the `release.rs` handler in `services/ledger`), not projected onto the `reservations` row's state.
- There is **no** `ttl_seconds` column. The table has `ttl_expires_at TIMESTAMPTZ` and `created_at TIMESTAMPTZ`; "TTL seconds" must be derived as `EXTRACT(EPOCH FROM (ttl_expires_at - created_at))`.
- There is no config home for the rule's `min_ttl_for_finding` threshold; the contract DSL today has no "rule_config" extensible surface.

**Revised verdict**: v0.1 has **zero rules** fireable today. The rule list in spec §5.1 cannot ship as-written.

### 8.2 Required new workstream: P0.6 ledger projection view

Before any rule (including `idle_reservation_rate_v1`) can fire, the ledger needs a derived view that surfaces the (reservation, derived_state, ttl_seconds, release_reason) tuple by joining `reservations` with `audit_outbox` release events. Suggested name: `reservations_with_ttl_status_v1` view. Owner: ledger team. Estimated work: ~2 days. Lands as P0.6, parallel to P0.5 sidecar enrichment.

### 8.3 canonical_events.payload_json shape was wrong

The original §3 dump of the audit.outcome payload shape (e.g. `{kind: "commit_estimated", reservation_id, ...}`) is what the sidecar puts on the CloudEvent `data` field. But `canonical_ingest` does NOT decode that field when persisting. Per `services/canonical_ingest/src/persistence/append.rs` and the `extract_cloudevent_payload()` helpers in the ledger handlers, the actual `canonical_events.payload_json` shape is:

```json
{
  "specversion":     "1.0",
  "type":            "spendguard.audit.outcome",
  "source":          "sidecar://...",
  "id":              "<uuid>",
  "time_seconds":    1234567890,
  "time_nanos":      0,
  "datacontenttype": "application/json",
  "data_b64":        "<base64 of the JSON above>",
  "tenantid":        "<uuid>",
  "runid":           "",
  "decisionid":      "<uuid>",
  "schema_bundle_id": "<uuid>",
  "producer_id":     "sidecar:...",
  "producer_sequence": 42,
  "signing_key_id":  "..."
}
```

Rules cannot do `payload_json->>'kind'` or `payload_json->>'estimated_amount_atomic'` directly — those keys are nested inside the base64-encoded `data_b64`. To extract, rules must `convert_from(decode(payload_json->>'data_b64', 'base64'), 'UTF8')::jsonb->>'estimated_amount_atomic'` — a `LATERAL` join is the clean Postgres pattern.

**Implication**: spec §6's Tier-2 baseline computation example using `(payload->>'committed_micros_usd')::bigint` is also wrong as written. The Tier-2 worker will need to decode `data_b64` or, better, JOIN to `ledger_transactions` / `commits` directly for cost data (which is what the original audit recommended anyway).

### 8.4 Emitter set was incomplete

The original §1 claim "the set of CloudEvent payloads that ever land in `canonical_events` is bounded to sidecar + ledger via audit_outbox" missed `services/webhook_receiver/src/handlers/webhook.rs`. That service constructs `spendguard.audit.decision` and `spendguard.audit.outcome` CloudEvents (lines 563-577) when a provider webhook lands a `provider_report` or `invoice_reconcile` operation. Those CloudEvents flow through the ledger's `audit_outbox` → `outbox_forwarder` → `canonical_ingest` chain, so they end up in `canonical_events` the same way sidecar emissions do.

The webhook_receiver emissions still do NOT carry `prompt_hash` / `agent_id` / `run_id` / `tool_name` / `tool_args_hash` / `model_family` (they only carry billing identifiers + provider event id), so the §4 conclusion that those fields are 0% populated remains correct. But the methodology claim that the static audit covered the entire emitter set was overconfident.

### 8.5 Revised workstream summary

| Workstream | Owner | Status | Purpose |
|---|---|---|---|
| P0 | cost_advisor | ✅ shipped (this branch) | proto + crate skeleton + migrations + integration design |
| P0.5 (NEW) | sidecar + adapter | open | thread `run_id`/`step_id` into CloudEvent + add `agent_id`/`model_family`/`prompt_hash` to `payload_json`'s `data` field. ~5 days. |
| P0.6 (NEW per codex r5 §8.1) | ledger | open | `reservations_with_ttl_status_v1` view joining reservations + release-event audit chain. ~2 days. |
| P1 (skinny) | cost_advisor | gated on P0.5 + P0.6 | runtime + `idle_reservation_rate_v1` rule SQL + CLI. ~4 days. |
| P1.5 | cost_advisor | gated on P0.5 + P0.6 | the other 3 rules (`failed_retry_burn_v1`, `runaway_loop_v1`, `tool_call_repeated_v1`) at run-scope. ~5 days. |

**Net new critical path to v0.1**: P0 (4d, done) + max(P0.5 5d, P0.6 2d) + P1 4d + P3 4d + P3.5 3d = **20 days** elapsed (16 days remaining). P0.5 and P0.6 run in parallel; the longer one (5d) sets the critical path. Matches §11.5 A2's +5-day envelope.

### 8.6 P1 readiness verdict (revised)

**🟡 P1 IS NOT ready to start immediately.** P1 is gated on both P0.5 and P0.6 landing. The original audit's "P1 can start immediately" claim was wrong — it presumed `idle_reservation_rate_v1` was fireable, which the schema review above shows it is not.

The P0 deliverables (proto + crate skeleton + 4 migrations + integration design doc) remain valid and useful — they are infrastructure that P1 / P0.5 / P0.6 all consume. But the runtime + first rule cannot land until the dependencies do.
