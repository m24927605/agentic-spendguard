# `DEMO_MODE=maf_python_real`

End-to-end docker-compose demo for the
`spendguard.integrations.agent_framework` adapter — the Microsoft Agent
Framework (MAF) Python middleware integration (the Python half of D07).

```bash
make demo-up DEMO_MODE=maf_python_real
```

## What it boots

Layered on top of the base `deploy/demo/compose.yaml` stack
(`postgres + ledger + canonical-ingest + sidecar + webhook-receiver +
outbox-forwarder`):

| Service | Role |
| ------- | ---- |
| `counting-stub` | In-network mock OpenAI `/v1/chat/completions` provider. Counts every call; serves `/_count` to readers. |
| `maf-python-runner` | Python 3.12 container — `pip install -e`s `spendguard-sdk[agent-framework]` from the in-tree source, then runs `examples/maf-python/run.py --real` against the sidecar UDS, drives `SpendGuardMiddleware.process(context, call_next)` against a counting-stub-backed `call_next`. |

## What it asserts

Three calls drive through the SpendGuard MAF middleware:

1. **ALLOW** — small message within budget. `request_decision(LLM_CALL_PRE)`
   returns `CONTINUE`, `call_next()` posts to the counting-stub, counter
   `+1`, `emit_llm_call_post(SUCCESS)` commits the reservation.
2. **DENY** — message tagged `trigger-deny: please block me`. The
   `call_next` body carries `spendguard_estimate_override=2000000000`
   which the sidecar's contract evaluator picks up; the evaluator emits
   `SPENDGUARD_DENY`; the middleware raises `DecisionDenied`; the inner
   counting-stub HTTP **never fires**. Counter `+0` (proves the gate
   fires pre-call).
3. **ALLOW2** — second small message within budget. Replaces D04 /
   D06 / D08's STREAM step (streaming gating is v0.1.x non-goal —
   `design.md §3`). Counter `+1`.

Success line on a clean run (LOCKED — CI greps for the exact spelling):

```
[demo] maf_python ALL 3 steps PASS (ALLOW + DENY + ALLOW2)
```

## Verify gates

After the runner exits, the Makefile runs:

```bash
make demo-verify-maf-python
```

Which executes [`verify_step_maf_python.sql`](../verify_step_maf_python.sql)
against `spendguard_ledger` and a cross-DB `canonical_events` count
against `spendguard_canonical`. The gates:

- `ledger_transactions.reserve >= 2` (ALLOW + ALLOW2 each produce
  EXACTLY one reservation).
- `ledger_transactions.commit_estimated >= 2` (ALLOW + ALLOW2).
- `ledger_transactions.denied_decision >= 1` (DENY step).
- `audit_outbox` carries `>= 2` decision rows in the last 5 min.
- `canonical_events` carries `>= 2` decision + `>= 1` outcome rows.
- INV-2 strict-order: first reservation row precedes first outcome row.

## Anti-scope

This overlay does NOT redeclare `postgres`, `sidecar`, `ledger`,
`canonical-ingest`, `bundles-init`, `pki-init` or any other base
service — the Makefile target brings up the base stack first, then
this overlay.

## Known cross-slice gap

The `--real` end-to-end run currently surfaces the same D05
`UnitRef.unit_id empty` substrate-side validation error D04 / D06 /
D08 also surface against the same sidecar. The adapter's wire shape
and middleware contract are independently verified by the pytest
suite (`sdk/python/tests/integrations/agent_framework/test_middleware.py`);
the `--mock` mode exercises the bracket end-to-end without going
through the substrate's `ReserveSet` validator. A future SDK-side
`unit_id` broadening lands the `--real` demo green here and across
the sibling D04 / D06 / D08 modes simultaneously.

## Related

- [`examples/maf-python/`](../../../examples/maf-python/) — the demo's
  source.
- [`sdk/python/src/spendguard/integrations/agent_framework/`](../../../sdk/python/src/spendguard/integrations/agent_framework/)
  — the adapter package.
- [`deploy/demo/maf_dotnet/`](../maf_dotnet/) — .NET sibling demo, same
  3-step matrix.
- [`deploy/demo/maf_python_with_agt/`](../maf_python_with_agt/) —
  MAF middleware wrapping the AGT policy engine (composite of D07 +
  the shipped `spendguard.integrations.agt` module).
