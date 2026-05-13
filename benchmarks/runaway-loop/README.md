# Runaway-loop benchmark

A reproducible head-to-head of three LLM cost-control libraries on the
same fixture: an agent that wants to make 100 LLM calls against a
$1.00 budget.

## What it measures

- **Did enforcement actually fire?** Did the library stop the loop, or
  did all 100 calls go through?
- **At what cost?** Ground-truth $ spent, computed from a centralized
  published-pricing table the analyzer applies to every (model, input
  tokens, output tokens) tuple the mock LLM observed.
- **Self-report vs reality.** What the library *thinks* it stopped at
  vs what *actually* hit the wire.

The point is **not** to embarrass the other tools — most of them solve
a real problem. The point is to make the dimensions that vary between
tools concrete and reproducible, instead of a marketing-page table.

## Tools under test

| Runner | Library | Language | Mode |
|---|---|---|---|
| `agentbudget` | [`agentbudget`](https://github.com/sahiljagtap08/agentbudget) | Python | drop-in `init("$1.00")` |
| `agentguard` | [`agent-guard`](https://github.com/dipampaul17/AgentGuard) | Node.js | drop-in `init({ limit: 1 })` |
| `spendguard` | this repo's SDK + sidecar | Python (SDK) + Rust (sidecar) | pre-call reservation against a real ledger |

The SpendGuard runner lives in [`compose.spendguard.yml`](./compose.spendguard.yml)
because it depends on the demo sidecar/ledger stack. See "SpendGuard
runner" below.

## How it's wired

```
                    +----------------+
   X-Runner header  |   mock LLM     | ──► /var/log/mock_llm.jsonl
   identifies the   | (FastAPI,      |     ground-truth call log
   calling runner.  |  port 8080)    |
                    +-------▲--------+
                            │
                            │ /v1/chat/completions
                            │
   ┌──────────────┐   ┌─────┴────────┐   ┌──────────────┐
   │ agentbudget- │   │ agentguard-  │   │ spendguard-  │
   │   runner     │   │   runner     │   │   runner     │
   │  (Python)    │   │  (Node.js)   │   │  (Python)    │
   └──────────────┘   └──────────────┘   └──────┬───────┘
                                                 │ UDS gRPC
                                                 ▼
                                    +-----------------------+
                                    │ SpendGuard sidecar +  │
                                    │ ledger (Phase 1B)     │
                                    +-----------------------+

   ─► /results/{runner}.json (each runner's self-report)
   ─► analyzer reads both, applies pricing table, writes RESULTS.md
```

## Reproducing

```bash
make benchmark
```

That brings up the mock LLM, runs each runner sequentially against
it, and writes `RESULTS.md` to this directory.

To wipe state and start over:

```bash
make benchmark-clean
```

To inspect what each runner did:

```bash
make benchmark-logs
```

## Scenario

Defined in [`scenario.yaml`](./scenario.yaml). Defaults:

- Budget: **$1.00**
- Max attempted calls: **100**
- Model: `gpt-4o`
- Per-call usage: 8,000 input tokens + 16,000 output tokens
- Per-call cost (published list price): **$0.18**

The per-call cost is deliberately not an integer divisor of the
budget — `$1.00 / $0.18 = 5.55 calls` — so a tool with **post-call**
enforcement (run the call, *then* check if budget is busted) overshoots
by one call, and a tool with **pre-call** reservation (refuse the call
if reserving would push us over) stops cleanly at five.

At those numbers, an unenforced loop spends $18.00 (18x the budget) by
the time it finishes 100 calls.

## Pricing table

The analyzer's ground-truth $ comes from the published list prices
embedded in [`analyze/analyze.py`](./analyze/analyze.py):

| Model | Input ($/1M) | Output ($/1M) |
|---|---:|---:|
| `gpt-4o` | $2.50 | $10.00 |
| `gpt-4o-mini` | $0.15 | $0.60 |
| `o1` | $15.00 | $60.00 |
| `o1-mini` | $1.10 | $4.40 |
| `claude-3-5-sonnet-20241022` | $3.00 | $15.00 |

Prices are the public list rates as of 2026-05. The point of using a
*centralized* table — instead of trusting each library's internal
pricing — is so that the "ground truth" number is the same number a
human would compute from an OpenAI invoice, regardless of how stale
or partial each library's internal table happens to be.

## Known limitations

- **Mock LLM only** — no real provider $$ is spent. The mock returns
  deterministic usage tokens; the analyzer multiplies by the public
  pricing table.
- **AgentGuard's drop-in mode only intercepts calls to `api.openai.com`.**
  Pointing the OpenAI client at a different base URL (self-hosted
  proxy, vLLM, Ollama, LiteLLM gateway, mock server) bypasses its
  HTTP-level interception entirely. The benchmark surfaces this as
  "no abort, full overshoot."
- **SpendGuard runner (Phase 1B) is a separate compose layer** because
  it requires the sidecar/ledger stack from `deploy/demo`.

## SpendGuard runner

> Phase 1B — TODO. Will use `compose.spendguard.yml` to bring up a
> minimal sidecar + ledger + Postgres subset alongside the mock LLM
> and run [`runners/spendguard/runner.py`](./runners/spendguard/) for
> head-to-head numbers.

## Files

- [`scenario.yaml`](./scenario.yaml) — shared scenario definition.
- [`mock_llm/`](./mock_llm/) — FastAPI mock OpenAI endpoint with
  per-call ground-truth log.
- [`runners/agentbudget/`](./runners/agentbudget/) — Python runner.
- [`runners/agentguard/`](./runners/agentguard/) — Node.js runner.
- [`runners/spendguard/`](./runners/spendguard/) — Phase 1B.
- [`analyze/`](./analyze/) — pricing-table aggregation + RESULTS.md
  generator.
- [`compose.yml`](./compose.yml) — Phase 1A orchestration.
- [`Makefile`](./Makefile) — one-command bring-up.
- [`RESULTS.md`](./RESULTS.md) — generated; rerun `make benchmark` to
  refresh.
