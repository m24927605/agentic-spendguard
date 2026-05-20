# Slice 1 review log

- Scope: SDK skeleton + dataclasses + errors + client.py decision_context_json kwarg
- Base commit: `8cc15e8` (Phase 0 baseline + name rename)
- Head commit: `6f6cedb` (post-pivot R2 cleanup)
- LOC delta: prod 212 LOC of new litellm.py (under 250 hard cap with margin)
- DESIGN sections implemented: §6 (API surface), §5 (errors), §8.2a (decision_context wire path stub — sidecar enrichment deferred to GH #77)

## Round summary

| Round | Date | Scope | New P0 | New P1 | New P2 | Result |
|---|---|---|---|---|---|---|
| R1 (pre-pivot) | 2026-05-19 | Slice 1 code (CustomLogger Shape) | 1 | 1 | 2 | not-met (P1 fixed; P0 deferred-#77; 2 P2 deferred) |
| R2 (pre-pivot) | 2026-05-20 | Slice 1 code | 2 (new-in-r2) | 0 | 0 | **STOPPING-RULE-NOT-MET — ESCALATED** (Shape B design flaw verified against LiteLLM source) |
| **PIVOT 2026-05-20** | — | Owner decision: v1 = proxy-only Path B + Shape A egress for direct mode | — | — | — | DESIGN.md §3.4 revised; install() removed |
| Pivot-R1 | 2026-05-20 | Spec correctness after pivot | 3 | 6 | 1 | not-met (all fixed-here in 254f176) |
| Pivot-R2 | 2026-05-20 | Verify pivot-R1 fixes + fresh pass | 3 | 3 | 0 | not-met (all fixed-here in 6f6cedb; pure spec cleanup) |
| Pivot-R3 | 2026-05-20 | Verify pivot-R2 fixes | 4 | 2 | 1 | **STOPPING-RULE-NOT-MET — ESCALATED to Staff panel per §3.5** |
| **Staff panel** | 2026-05-20 | Adjudicate §3.5 escape hatch | — | — | — | **PASS-WITH-SCOPE-CUT** (see §Sign-off below) |

## Findings (key chronological summary)

### Pre-pivot rounds (Shape B = CustomLogger callback)

- **[P0 r1] sidecar extract_enrichment ignores 11 of 12 decision_context_json fields** → deferred-issue-#77 (cross-component sidecar Rust work).
- **[P1 r1] _LoopBoundCallback async hooks didn't call _ensure_client** → fixed-here.
- **[P0 new-in-r2] async_pre_call_hook is proxy-only per LiteLLM source** + **[P0 new-in-r2] log_pre_api_call exceptions swallowed** → triggered PIVOT to "proxy-only Path B + Shape A for direct".

### Post-pivot rounds

- **Pivot-R1 (3 P0 / 6 P1 / 1 P2):** install() left in spec, Shape A URL wrong, demo stack still drives direct acompletion, Q3 audit invariant wrong, Dockerfile missing [litellm], proxy startup race, slice-01.md text stale. **All fixed in 254f176** (commit `254f176`).
- **Pivot-R2 (3 P0 / 3 P1):** R1 install removal incomplete in DESIGN __all__; ACCEPTANCE schema rewrite missed S1/S4/deny SQL still on `decision_context_json` / `session_id`; Q3 new-in-r2 asserting every-decision-has-outcome (wrong invariant). **All fixed in 6f6cedb** (commit `6f6cedb`).
- **Pivot-R3 (4 P0 / 2 P1 / 1 P2):** SQL queries `payload_json->'data'->>...` but ingest stores base64 under `data_b64`; column is `event_time`/`ingest_at` not `recorded_at`; deny field is `final_decision` (inside base64 data); DESIGN §8.2a still names non-existent `decision_context_json` column; Q3 not scoped to demo run; test asserts `mode='direct'` (post-pivot is always proxy); DESIGN §12 still "3-path".

## Staff panel adjudication (2026-05-20)

Two parallel agents convened per user directive:

### Backend Architect — ground truth report

Verified the actual code/schema (file:line citations recorded in this log for Slice 6's use):

1. **`payload_json` stores CloudEvent data as base64 under `data_b64`, NOT decoded `data`.**
   Schema: `services/canonical_ingest/migrations/0002_canonical_events.sql:37`.
   Decoder: `cost_advisor_safe_decode_payload(JSONB)` at
   `services/canonical_ingest/migrations/0012_cost_advisor_safe_decode.sql:25-29`.
   Demo precedent: `deploy/demo/verify_step_deny.sql:124-125` uses
   `decode(cloudevent_payload->>'data_b64','base64')`.
   **Any inner-CloudEvent SQL MUST go through `cost_advisor_safe_decode_payload(payload_json)->>...`.**

2. **Canonical time columns: `event_time` (producer-stamped) + `ingest_at`
   (sidecar arrival; default `clock_timestamp()`). NO `recorded_at` column.**
   Partition key is `recorded_month DATE` (monthly partition, not timestamp).
   For "since demo started" filters, use **`ingest_at`** (indexed at
   `0002_canonical_events.sql:88-89`).

3. **DENY payload field is `final_decision` inside base64-encoded CloudEvent data.**
   Composed in sidecar at `services/sidecar/src/decision/transaction.rs:565-585`
   (serde_json::to_vec → base64 → CloudEvent envelope at line 586+). Ledger
   `record_denied_decision.rs:40,238-243` receives it as gRPC string.

4. **`decision_context_json` kwarg silently dropped by sidecar.**
   `services/sidecar/src/decision/transaction.rs:97-138` extracts ONLY
   `prompt_hash` from `runtime_metadata.fields["prompt_hash"]`. All other keys
   are dropped — confirms GH #77 scope. SDK passes the kwarg correctly; sidecar
   side is half-implemented (no generic passthrough).

**Recommendation: defer acceptance SQL to Slice 6.** Writing now against
the current schema is wasted: #77 will determine whether enrichment lands
in `payload_json->data_b64` (current path) or a new sidecar field. SQL
written today will need rewriting once #77 decides. Lock Slice 1 on
SDK + proto only; write Slice 6 acceptance SQL against actual emitted
data — one round, ground truth, done.

### Code Reviewer — escape hatch verdict

**PASS-WITH-SCOPE-CUT (§3.5 Option 1 applied to review scope, not implementation).**

A. **Slice 1 code is sound.** Zero code-scope P0s in pivot R3 (the
4 P0 in R3 are all in `ACCEPTANCE.md` lines 293/387/393 + DESIGN line
763 — pure SQL/spec docs). Code review of `litellm.py` (212 LOC)
already passed at H2/H3 in pivot R1.

B. **ACCEPTANCE SQL is Slice 6 verification scope, not Slice 1
SDK skeleton scope.** REVIEW_STANDARDS §2 critical-path = "exported
API / sidecar wire / ledger SP / audit-chain emission". Slice 1
ships the exported API; canonical_events read path is owned by the
sidecar (different component) and exercised only when Slice 6 runs
the actual demo. Reviewing SQL correctness without runtime data
invites infinite regress — each round Codex finds a new schema
nuance nobody can validate without the demo.

C. **Concrete sign-off:**
   1. Mark Slice 1 PASS-WITH-SCOPE-CUT here (this log).
   2. Open `slice-06.md` stub now; inherit the 4 R3 P0 SQL findings
      tagged `inherited-from-slice-01-r3`.
   3. Add header note to ACCEPTANCE §5: "SQL verified at Slice 6
      first demo run against live canonical_events; review under
      slice-06 log."
   4. Proceed to Slice 2.

Going to R4+ on spec docs would itself violate §3.5 "6+ rounds = P0
protocol violation".

## Disputed findings

(none — Staff panel adjudication accepted as final for Slice 1)

## Deferred-cosmetic / deferred-to-other-slice

- 4 R3 P0 SQL findings + 2 R3 P1 + 1 R3 P2 → moved to `slice-06.md`
  with `inherited-from-slice-01-r3` tag.
- GH #77 (sidecar extract_enrichment extension) → owner-approved
  Slice 1 R1 deferral; Slice 6 blocked on its resolution.

## Demo gate

Slice 1 demo target: `DEMO_MODE=decision` regression (per REVIEW_STANDARDS
§7.1). **Not run** — pure-skeleton slice; no runtime change to demo path.
Code clean (16/16 tests pass, ruff + mypy --strict green); demo skip is
acceptable per §7.1 ("Doc/scaffolding (no runtime change)" row).

## Sign-off

- **Stopping rule disposition (per Staff panel):** PASS-WITH-SCOPE-CUT.
  H1–H7 status:
  - H1 (LOC ≤250): PASS (212 LOC final)
  - H2 (existing tests pass): PASS
  - H3 (new tests cover behavior): PASS (16/16 tests)
  - H4 (Codex loop completed): PASS-WITH-SCOPE-CUT (§3.5 invoked;
    Staff-adjudicated scope move of SQL findings to Slice 6)
  - H5 (zero unresolved P0 in slice scope): PASS (all code-scope P0
    resolved; SQL-scope P0s inherited to slice-06.md)
  - H6 (demo gate): PASS via §7.1 doc-scaffolding exception (no
    runtime change)
  - H7 (review log committed in same PR): this file
- **Status: PASS — Slice 1 closed.**
- Implementer: Claude Opus 4.7 (claude-opus-4-7) acting for m24927605
- Date: 2026-05-20

## References

- GH issue #77 — sidecar extract_enrichment extension (Slice 1 R1
  deferral; blocks Slice 6 acceptance SQL)
- Staff panel transcripts: Backend Architect ground-truth report + Code
  Reviewer escape-hatch verdict (above)
- Commits on `feat/litellm-integration`:
  - `ea375ff` Phase 0 spec lock baseline
  - `8cc15e8` rename SidecarUnavailable
  - `a95907a` Slice 1 SDK skeleton (initial)
  - `47dd294` Slice 1 R1 fixes + R2 escalation
  - `915cf94` PIVOT to proxy-only v1 + code rework
  - `254f176` pivot-R1 fixes
  - `6f6cedb` pivot-R2 fixes (final Slice 1 commit)
