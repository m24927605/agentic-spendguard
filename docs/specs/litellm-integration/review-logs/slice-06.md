# Slice 6 review log (STUB — opened 2026-05-20)

> Status: NOT STARTED. Inherits acceptance-SQL findings from Slice 1
> pivot R3 per Staff panel adjudication (see `slice-01.md` §Staff
> panel adjudication).
>
> Slice 6 = `DEMO_MODE=litellm_real` ALLOW + DENY proxy-driven demo.
> First slice that actually exercises canonical_events SQL gates;
> therefore the appropriate owner of the inherited findings.

## Inherited findings from Slice 1 pivot R3

These 4 P0 + 2 P1 + 1 P2 SQL/schema findings transferred from Slice 1
review per Staff panel (Code Reviewer) — they pertain to SQL gates
that cannot be validated without running the live demo, so review
moved to where the demo lands.

### inherited-from-slice-01-r3 — P0

- **[P0] ACCEPTANCE.md:293 (Q1/Q2/Q3 SQL)** — queries
  `payload_json->'data'->>...` but ingest stores CloudEvent data as
  base64 under `payload_json.data_b64` (not a decoded `data` object).
  Fix: use `cost_advisor_safe_decode_payload(payload_json)->>...`
  decoder per `services/canonical_ingest/migrations/0012_cost_advisor_safe_decode.sql:25-29`.
- **[P0] ACCEPTANCE.md:387 (deny-path SQL)** — references
  `canonical_events.recorded_at` which does not exist. Use
  `ingest_at >= $step2_start` (indexed) instead. Apply same scoping
  to Q2/Q2b/Q3.
- **[P0] ACCEPTANCE.md:393 (budget-exhausted assertion)** — checks
  `payload_json->>'decision' = 'STOP'`. Actual deny payload writes
  `final_decision` inside base64-encoded CloudEvent data
  (`services/sidecar/src/decision/transaction.rs:565-585`). Decode
  and assert `data->>'final_decision' = 'STOP'`.
- **[P0] DESIGN.md:763 (§8.2a)** — still names non-existent
  `canonical_events.decision_context_json` column. Restate:
  `decision_context_json` is SDK→sidecar wire input; canonical
  verification reads `payload_json.data_b64` (decoded) once GH #77
  lands the sidecar-side passthrough.

### inherited-from-slice-01-r3 — P1

- **[P1] ACCEPTANCE.md:318 (Q3)** — only queries
  `canonical_events_global_keys` which has no `run_id`/payload
  fields; cannot scope to "this LiteLLM run". Join back to
  `canonical_events` and filter by `ingest_at >= $demo_start` plus
  decoded `integration='litellm'`, or restate as a global invariant.
- **[P1] TEST_PLAN.md:185** — `test_decision_context_json_fields`
  asserts `mode='direct'` when `user_api_key_dict is None`. Post-pivot
  v1 has no direct-mode callback path; remove the direct assertion;
  direct LiteLLM belongs to Shape A (egress proxy) recipe.

### inherited-from-slice-01-r3 — P2

- **[P2] DESIGN.md:1001** — §12 slicing hint still says "3-path
  page"; Slice 10 spec already reduced to 2-path. Change to
  "2-path page".

## Ground-truth schema reference (Slice 1 Staff panel — Backend Architect)

**Use these citations when writing Slice 6 acceptance SQL + verify_step
files. Verified 2026-05-20.**

| Question | Answer | Citation |
|---|---|---|
| Where does CloudEvent `data` live in `canonical_events`? | base64 string under `payload_json.data_b64`; NOT decoded JSON | `0002_canonical_events.sql:37` + `0012_cost_advisor_safe_decode.sql:25-29` |
| What decoder helper exists? | `cost_advisor_safe_decode_payload(JSONB)` returns decoded JSONB, NULL on malformed | `0012_cost_advisor_safe_decode.sql:25-29` |
| Time columns? | `event_time` (producer-stamped, can skew) + `ingest_at` (sidecar arrival, indexed). NO `recorded_at` | `0002_canonical_events.sql:35,44,88-89` |
| Partition key? | `recorded_month DATE` (monthly partition) | `0002_canonical_events.sql:47` |
| DENY payload field name? | `final_decision` inside the base64 data; composed at sidecar (NOT ledger) | `services/sidecar/src/decision/transaction.rs:565-585` |
| LiteLLM-specific fields (12-field decision_context_json) — when do they land? | When GH #77 extends `extract_enrichment` to pass `runtime_metadata` keys other than `prompt_hash` through. Until then: NULL/missing. | `services/sidecar/src/decision/transaction.rs:97-138` |

## Slice 6 acceptance — when to start

1. **GH #77 must land first.** Without sidecar enrichment of LiteLLM
   audit fields, the SQL Q2 cross-join will return 0 rows regardless
   of whether the callback fires. Slice 6's demo gate is gated on #77.
2. **Then write Slice 6 verify_step_litellm_real.sql against the
   actual emitted CloudEvent data**, using the decoder pattern above.
3. Update ACCEPTANCE.md §5 SQL block to match Slice 6's verified
   queries (delete the placeholder Q1/Q2/Q3 written speculatively in
   Phase 0).

## Demo gate — pending Slice 6 execution

Not run.

## Sign-off — pending Slice 6 implementation start

(empty until Slice 6 begins)
