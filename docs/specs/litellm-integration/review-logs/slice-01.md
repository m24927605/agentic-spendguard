# Slice 1 review log

- Scope: SDK skeleton + dataclasses + errors + client.py decision_context_json kwarg
- Base commit: `8cc15e8` (Phase 0 baseline + name rename)
- Head commit (after R1 fixes, pre-rework): see HEAD on `feat/litellm-integration`
- LOC delta: prod 263 / hard cap 250 (overage +13 documented in §6.5)
- DESIGN sections implemented: §6 (API surface), §5 (errors), §8.2a (decision_context wire path)

## Round summary

| Round | Date | New P0 | New P1 | New P2 | New P3 | Fixed-here | Deferred | Result |
|---|---|---|---|---|---|---|---|---|
| 1 | 2026-05-19 | 1 | 1 | 2 | 0 | 1P1 + 1P2 | 1P0 (#77) + 1P2 (slice 2) | not-met |
| 2 | 2026-05-20 | 2 (new-in-r2) | 0 | 0 | 0 | — | — | **STOPPING-RULE-NOT-MET — ESCALATED** |

## Findings (chronological)

### Round 1 (2026-05-19)

- **[P0] client.py:449** — `decision_context_json` flattened into `runtime_metadata` Struct, but
  sidecar's `extract_enrichment` only extracts `prompt_hash`; other 11 LiteLLM audit fields silently
  dropped before `canonical_events.decision_context_json` is written.
  → **deferred-issue-#77** (owner approval 2026-05-20: Python SDK is complete; sidecar Rust
  extension is cross-component out of SDK PR scope).
- **[P1] litellm.py:157** — `_LoopBoundCallback` lacks async hook overrides that call
  `_ensure_client()` before super().
  → **fixed-here** in HEAD: added 3 hook overrides + regression test
  `test_loop_bound_callback_async_hooks_call_ensure_client_first`. Drives +13 LOC over the
  250 hard cap; owner-accepted (§6.5).
- **[P2] test_litellm_skeleton.py:143** — ADR-005 test calls `cb.log_pre_api_call()` directly
  instead of through LiteLLM dispatcher.
  → deferred to Slice 2 integration tests (no real LiteLLM acompletion call in Slice 1 SDK-skeleton scope).
- **[P2] test_litellm_skeleton.py:16** — `pytest.importorskip` skips the missing-extra test in the
  exact env where it matters.
  → **fixed-here**: new `tests/test_litellm_missing_extra.py` runs without litellm installed.

### Round 2 (2026-05-20) — STOPPING-RULE-NOT-MET, ESCALATED

- **[P0 new-in-r2] litellm.py:115** — `async_pre_call_hook` is **proxy-only** per LiteLLM source.
  Verified: `grep -rn 'async_pre_call_hook(' litellm/` returns hits ONLY in `litellm/proxy/`
  modules; ZERO invocations in `litellm/litellm_core_utils/litellm_logging.py` (the direct-mode
  path). Direct `litellm.acompletion()` does NOT invoke this hook — Slice 6 step 1 ALLOW + step 2
  DENY (direct-mode demo) would silently fail-OPEN. The killer-invariant deny demo CANNOT prove
  fail-closed for direct mode under Shape B as designed.
- **[P0 new-in-r2] litellm.py:142** — Sync `log_pre_api_call` IS invoked in direct path, BUT
  wrapped in `try / except Exception` at `litellm_logging.py:45887` → exception is logged via
  `verbose_logger.exception` and **swallowed**. Provider call proceeds regardless. ADR-005
  fail-closed claim is unenforceable through the documented hook.

**Disposition:** ESCALATED to owner per REVIEW_STANDARDS.md §3.5. **Owner decision 2026-05-20
(revised — superseded the initial monkey-patch choice):**
**v1 proxy-only Path B + Shape A egress proxy for direct mode** (DESIGN.md §3.4 v1 revised).
The earlier "monkey-patch acompletion" option was reconsidered — monkey-patching LiteLLM's
dispatch is fragile to upstream changes and adds significant SDK code (~250 LOC) to maintain
a parallel call path. Instead: Shape B (CustomLogger callback) is locked to LiteLLM **proxy
mode only** (where `async_pre_call_hook` is actually invoked); direct in-process callers route
through the EXISTING SpendGuard egress proxy via `litellm.api_base = "http://localhost:9000/v1"`
(zero new SDK code; reuses `auto-instrument-egress-proxy` infrastructure that already ships).
DESIGN.md §3.4 + IMPLEMENTATION.md Slice 1-9 updated. The current Slice 1 commit (`a95907a`)
ships dataclasses + skeleton; the pivot commit (`915cf94`) removes `install()` + `log_pre_api_call`
override; subsequent pivot-R1 fixes update specs + remove `install()` from `__all__`.

## Disputed findings

(none — all P0s either fixed-here, deferred with approval, or escalated)

## Deferred-cosmetic aggregation

- LOC overage +13 (263 vs 250 hard cap): driven by R1 P1 fix that owner accepted as in-scope; no
  follow-up issue.

## Demo gate

Slice 1 demo target: `DEMO_MODE=decision` regression. **Not run** — Slice 1 is paused pending
DESIGN.md §3.4 rework per the escalation above.

## Sign-off

- **Stopping rule NOT met** at round 2.
- H1 (LOC ≤250): FAIL (263, +13 over)
- H2 (existing tests pass): PASS (16/16 new tests, no regressions)
- H3 (new tests cover behavior): PASS
- H4 (Codex loop completed): N/A — escalated
- H5 (zero unresolved P0): FAIL — 2 new P0s require DESIGN rework before resumption
- H6 (demo gate): not run
- H7 (review log committed): this file
- **Status: BLOCKED on DESIGN.md §3.4 redesign + IMPLEMENTATION.md Slice 1-7 rework.**
- Implementer: Claude Opus 4.7 (claude-opus-4-7) acting for m24927605
- Date: 2026-05-20

## References

- GitHub issue #77 — sidecar extract_enrichment extension (R1 P0 deferral)
- LiteLLM source verified: hook routing differs between proxy and direct mode
- REVIEW_STANDARDS.md §3.5 (escape hatch: escalate to owner)
- DESIGN.md §3.4 (Shape B recommendation — needs revision per R2 P0s)
