# Slice 5 — Failure release + retry handling · adversarial review log

Slice scope: Implement `async_log_failure_event` + `_classify_failure`
in `sdk/python/src/spendguard/integrations/litellm.py`. Release the
stashed reservation via `emit_llm_call_post(outcome=FAILURE|CANCELLED,
..._atomic="0")`. Per ADR-002, each LiteLLM retry attempt has a
distinct `litellm_call_id` → distinct `decision_id` → distinct
reservation; pre-call reserves, this hook releases. TTL sweep is the
durable backstop (`FAILURE_MODES.md`).

## Round summary

| Round | Reviewer | Verdict | Headline |
| --- | --- | --- | --- |
| R1 | Codex (`gpt-5.5`, low reasoning) | **PASS** | Zero P0/critical-P1. Five P2 concerns noted, not escalated. |
| R2 | Staff panel (Code Reviewer + Backend Architect, parallel) | **PASS** | Zero P0/critical-P1. Six P2 observations, all docstring/cosmetic. |

## R1 outcome — Codex `gpt-5.5`

Quote: *"On release error swallowing, the diff is internally
consistent: SpendGuardError is swallowed to avoid masking the original
LiteLLM failure, and the reservation is left for TTL sweep. The other
highlighted risks do not rise to critical severity in the shown code.
String-form 'cancelled' can false-positive, multi-reservation releases
only first, non-SpendGuardError bubbling could interfere with the
original error path, and success-then-failure idempotency may depend
on downstream semantics, but none is demonstrably catastrophic from
this isolated diff without broader contract evidence."*

`VERDICT: PASS`.

## R1 P2 hardening (committed `1d6b5a5`)

Although R1 did not escalate, the substring `"cancelled" in
exception.lower()` was hardened proactively:

- Replaced naive substring with word-boundary regex
  `(?:^|[^A-Za-z])cancell?ed(?:$|[^A-Za-z])` (case-insensitive).
- Accepts both British "cancelled" and American "canceled".
- Rejects "uncancelled", "cancellation_not_allowed", "precancelled",
  "cancelledness" etc.
- Underscore handling: `\b` alone is too loose (treats `_` as a word
  char), so `[^A-Za-z]` is preferred — verified via test on
  "operation_canceled by user" → CANCELLED.
- Added 2 regression tests: American spelling + word-boundary rejection.

## R2 outcome — Staff panel (Codex CLI was unreliable on this slice; Staff panel substituted per user mandate "如有問題請組織 Staff 等級以上的專業團隊")

### Code Reviewer
- **F1 (P2)** Docstring/implementation mismatch on the swallow contract — docstring says "Release errors are SWALLOWED" but only `SpendGuardError` is. Recommendation: tighten docstring to match the inline comment.
- **F2 (P2)** Word-boundary regex would not match a bare type name like `"CancelledError"` if some LiteLLM version stringifies it. Narrow edge — `isinstance` branch handles the live instance and `str(asyncio.CancelledError())` is empty.
- **F3 (P2)** `len(reservation_ids) != 1` log message says "releasing first only" even for 0-reservation case where nothing is released.
- Other concerns (release swallow, multi-reservation, non-SpendGuard bubble, empty reservations, stash pop ordering) verified safe.
- `VERDICT: PASS`.

### Backend Architect
- All six architectural concerns explicitly addressed: TTL sweep
  durability tradeoff is correct; F4 audit coverage holds under
  multi-reservation defensive path; concurrent retry race is
  prevented by call_id-keyed stash; non-SpendGuardError bubble is
  intentional contract; empty-reservations pop is correct; word-
  boundary regex is sufficient for ASCII provider error surfaces in v1.
- Six P2 observations (none load-bearing). Mostly polish:
  documentation suggestions for FAILURE_MODES.md, optional `reservation_id`
  in WARN log payload, future i18n consideration when Bedrock/Vertex
  i18n error messages become a signal.
- `VERDICT: PASS`.

## Stopping rule (§3.4) — MET

- (A) Prior P0 — none open from Slice 5.
- (A') Critical-P1 — R1 noted only P2; R2 confirmed.
- (B) Zero new P0/critical-P1 in either R2 reviewer's report.
- (C) N≥2 — R1 (Codex) + R2 (Staff panel).

## Tests

- `tests/test_litellm_failure_unit.py` — 13 tests passing.
- Full suite: 103 pass; ruff clean (litellm-scoped); mypy strict clean.

## P2 deferred (acceptable per stopping rule)

The 5+6 = 11 P2 observations across the two rounds (with overlap)
focus on docstring tightening, defensive log text, optional i18n
hardening, and ops-payload enrichment. None affect F2/F4/F7
invariants. They are deferred to Slice 8/Slice 10 documentation /
operations polish.

## Slice 5 → CLOSED. Next: Slice 6 — Demo `litellm_real` ALLOW + DENY.
