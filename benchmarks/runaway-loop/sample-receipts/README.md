# Receipts — what the benchmark captured

These files are snapshotted by the analyzer on every `make benchmark`
run. They're the "show me the evidence" artifacts behind the headline
numbers in [`../RESULTS.md`](../RESULTS.md).

## `spendguard-ledger.jsonl`

The Agentic SpendGuard reservation-gateway shim's append-only ledger.
Every reserve / commit / release / reserve_denied event with timestamp.

### What this run shows

```
RESERVE     $0.18   spent=0.00   →   commit $0.18   spent_after=0.18
RESERVE     $0.18   spent=0.18   →   commit $0.18   spent_after=0.36
RESERVE     $0.18   spent=0.36   →   commit $0.18   spent_after=0.54
RESERVE     $0.18   spent=0.54   →   commit $0.18   spent_after=0.72
RESERVE     $0.18   spent=0.72   →   commit $0.18   spent_after=0.90
RESERVE     $0.18   ──────────── → DENIED 402   would_exceed_budget
                                   spent=0.90, budget=1.00, remaining=0.10
```

The 6th call **never made it to the upstream LLM**. The shim returned
HTTP 402 *before* the runner could send `chat.completions.create()`,
and the runner aborted. Ground-truth $ spent on the wire (per the
mock LLM's call log): exactly **$0.90**.

This is the structural shape of the production sidecar's
`Decide` → `PublishEffect` flow (see
[`sdk/python/src/spendguard/client.py`](../../../sdk/python/src/spendguard/client.py)),
minus KMS signing, idempotency keys, and the contract DSL. The
production sidecar's audit chain produces the same event shape with
a chained signature on each row.

## `mock-llm-calls.jsonl`

Every HTTP call to the mock OpenAI endpoint, segregated by the
`X-Runner` header. This is the source of truth for ground-truth $.
Use it to cross-check what each library *actually* sent on the wire
vs what it *thinks* it sent.

### What this run shows (call counts per runner)

| Runner | Wire calls observed |
|---|---:|
| `agentbudget` | 6 |
| `agentguard` | 100 |
| `spendguard` | 5 |

Multiply each (model, input_tokens, output_tokens) tuple by the
analyzer's published-pricing table to recover the dollar number.

## Regenerating

```bash
make benchmark
```

Receipts are overwritten on every run. Commit the snapshot you want
to lock in.
