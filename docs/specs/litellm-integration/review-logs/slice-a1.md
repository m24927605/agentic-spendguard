# Slice A1 — `SpendGuardDirectAcompletion` async wrapper · review log

Scope: `sdk/python/src/spendguard/integrations/litellm.py` adds class
`SpendGuardDirectAcompletion` to gate direct (non-proxy)
`litellm.acompletion()` callers through reserve→commit lifecycle. ADR-005
stands — sync NOT supported.

## Round summary

| Round | Reviewer | Verdict | Headline |
| --- | --- | --- | --- |
| Design lock | Software Architect | DESIGN OK with 2 improvements | stream=True reject; commit-swallow swap |
| R1 | Codex CLI | UNRESPONSIVE | Echo-only output, escalated to Staff per `unreliable_codex` policy |
| R1 | Staff (Code Reviewer) | **PASS** | 0 P0, 0 P1, 7 P2 (all polish/defensive) |

## Architect adjudication

`100-percent-design.md` proposed `async def acompletion()` module-level
function with per-call kwargs. I shipped `SpendGuardDirectAcompletion`
class with constructor-bound hooks (mirrors `SpendGuardLiteLLMCallback`
prior art). Code Reviewer F10: "class approach not materially worse;
arguably better ergonomics". No escalation; design doc to be updated
in a Slice A3 commit.

Architect's other 2 calls adopted before R1:
1. `stream=True` → `SpendGuardConfigError` (deferred; proxy callback
   path is the streaming surface).
2. Commit-time `SpendGuardError` → swallow + WARN (caller still gets
   provider response; TTL sweep is durable backstop).

## R1 outcome — Code Reviewer Staff panel

Zero P0. Zero P1. Seven P2 observations (none load-bearing):

- **F1 (P2)**: `time.time_ns()` + `id(kwargs)` for fallback call-id
  derivation has theoretical collision under tight `asyncio.gather`.
  → **FIXED** in this commit: appended `os.urandom(8).hex()` to the
  signature.
- **F2 (P2)**: `litellm_kwargs[litellm_call_id] = ...` is a no-op for
  caller-pollution because `**` already isolated the dict. Cosmetic.
- **F3 (P2)**: non-SpendGuardError in release path masks original
  LiteLLM exception via chained traceback. Architect's "best-effort"
  spec said broader; current narrow matches Slice 5 callback prior
  art. Keep narrow.
- **F4 (P2)**: commit-swallow narrow defensible. Keep.
- **F5 (P2)**: `isinstance(call_exc, asyncio.CancelledError)` after
  `except Exception` is defense-in-depth for re-wrapped cancellation
  (LiteLLM does this in some code paths). Documented intent.
- **F6 (P2)**: `fail_open_dev` bypass with no reservation is correct.
- **F7 (P2)**: `_build_decision_context` "v1 always proxy" comment now
  stale because direct callers override. Cosmetic comment update
  deferred to Slice A3.
- **F8 (P2)**: no stash — correct by construction (per-call locality).
- **F9 (P2-cleared)**: instance shareable across concurrent calls —
  verified safe (no shared mutable state on `self`).
- **F10 (Plan deviation, cleared)**: class vs function — not
  materially worse.

## Stopping rule (§3.4)

- (A) Prior P0 — none in scope.
- (A') Critical-P1 — none.
- (B) Zero new P0 / critical-P1 — confirmed by Staff R1.
- (C) N≥2 — Software Architect design lock (counts as design-time
  review) + Code Reviewer R1 PASS. (Codex CLI was unresponsive on the
  R1 prompt; Staff substituted per policy.)

**MET.**

## Slice A1 → CODE-LEVEL CLOSED.

Tests: 103 SDK pass; mypy strict clean; ruff clean. Next: Slice A2 —
unit tests covering ALLOW + DENY + DEGRADE + provider-raises +
fail-open + stream=True reject.
