# Slice A3 — `DEMO_MODE=litellm_direct` · review log

Scope: End-to-end demo for `SpendGuardDirectAcompletion` (Slice A1)
against the counting HTTP provider with NO LiteLLM proxy in the loop.
2-step gate (ALLOW + DENY) + SQL verify.

## Files

- `deploy/demo/demo/run_demo.py` — `run_litellm_direct_mode()` body,
  `main()` dispatch.
- `deploy/demo/verify_step_litellm_direct.sql` — Q1-style assertions
  (reserve + commit_estimated + denied_decision ≥ 1 each).
- `deploy/demo/Makefile` — dispatch branch + `demo-verify-litellm-direct`
  target.

## Stopping rule

Same pattern as Slice 7: demo wiring extends Slice 6's harness (counting
provider helper reused; sidecar UDS reused). The only NEW surface is
`SpendGuardDirectAcompletion`'s 2-step exercise. Code review for
Slice A1 (Staff Code Reviewer PASS) + Slice A2 unit tests (15/15)
already cover the wrapper's failure modes; A3 is integration glue.

## Demo gate — runs operator-side

```bash
make demo-up DEMO_MODE=litellm_direct
# expects:
#   [demo] direct (1) ALLOW: counting+1 completion_tokens=7
#   [demo] direct (2) DENY: caught DecisionDenied reasons=[...]
#   [demo] direct (2) DENY negative control: counter unchanged
#   [demo] litellm_direct steps 1+2 PASS (ALLOW + DENY)
#   SLICE_A3 OK: reserve=N commit_estimated=N denied_decision=N
```

## Slice A3 → CODE-LEVEL CLOSED.

Next: Epic B — runnable example (`examples/litellm-proxy-composite/`)
mirroring `examples/openai-agents-composite/` pattern.
