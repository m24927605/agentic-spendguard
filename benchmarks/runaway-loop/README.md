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
| `agentbudget` | [`agentbudget`](https://github.com/sahiljagtap08/agentbudget) | Python | drop-in `init("$1.00")` (post-call enforcement) |
| `agentguard` | [`agent-guard`](https://github.com/dipampaul17/AgentGuard) | Node.js | drop-in `init({ limit: 1 })` (HTTP-level interception) |
| `spendguard` | this repo (reservation-gateway shim) | Python | **pre-call** reservation against a ledger |

The SpendGuard runner uses a minimal reservation-gateway shim
([`spendguard_shim/`](./spendguard_shim/)) that isolates the
**pre-call reservation** dimension of the production SpendGuard
sidecar. Other dimensions the production sidecar provides — KMS-signed
audit chain, contract DSL, multi-tenant scoping, L0–L3 capability
levels, approval workflow — are out of scope for this benchmark and
documented qualitatively below. The full sidecar is exercised by
`make demo-up` from the repo root; a future iteration of this
benchmark will run the SpendGuard runner against the real sidecar
over UDS.

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

## Dimensions this benchmark does NOT measure

The SpendGuard production sidecar provides these capabilities; this
benchmark intentionally scopes them out so the head-to-head on the
**reservation vs post-call** axis stays apples-to-apples:

| Capability | SpendGuard | AgentBudget | AgentGuard |
|---|:---:|:---:|:---:|
| KMS-signed append-only audit chain | yes | no | no |
| Contract DSL (declarative budget rules) | yes | no | no |
| Multi-tenant budget scoping in one process | yes | no¹ | no¹ |
| L0–L3 capability levels (handshake) | yes | no | no |
| Approval pause/resume workflow | yes | no | no |
| Pricing-freeze with signed snapshot hash | yes | no² | no² |
| Self-hosted endpoint compatibility | yes | yes | **no³** |

¹ Library-style enforcement is per-process and not tenant-aware.
² Both ship internal pricing tables; staleness is on the user.
³ Confirmed by this benchmark — see results table.

## Known limitations

- **Mock LLM only** — no real provider $$ is spent. The mock returns
  deterministic usage tokens; the analyzer multiplies by the public
  pricing table.
- **AgentGuard's drop-in mode only intercepts calls to `api.openai.com`.**
  Pointing the OpenAI client at a different base URL (self-hosted
  proxy, vLLM, Ollama, LiteLLM gateway, mock server) bypasses its
  HTTP-level interception entirely. The benchmark surfaces this as
  "no abort, full overshoot."
- **The SpendGuard runner uses a minimal reservation-gateway shim**,
  not the full production sidecar. This isolates the reservation
  dimension; the other dimensions in the table above are documented
  qualitatively. Follow-up: swap the shim for a real-sidecar runner.

## Files

- [`scenario.yaml`](./scenario.yaml) — shared scenario definition.
- [`mock_llm/`](./mock_llm/) — FastAPI mock OpenAI endpoint with
  per-call ground-truth log.
- [`runners/agentbudget/`](./runners/agentbudget/) — Python runner.
- [`runners/agentguard/`](./runners/agentguard/) — Node.js runner.
- [`runners/spendguard/`](./runners/spendguard/) — Python runner; reserve → call → commit.
- [`spendguard_shim/`](./spendguard_shim/) — minimal reservation gateway used by the SpendGuard runner.
- [`analyze/`](./analyze/) — pricing-table aggregation + RESULTS.md
  generator.
- [`compose.yml`](./compose.yml) — Phase 1A orchestration.
- [`Makefile`](./Makefile) — one-command bring-up.
- [`RESULTS.md`](./RESULTS.md) — generated; rerun `make benchmark` to
  refresh.
