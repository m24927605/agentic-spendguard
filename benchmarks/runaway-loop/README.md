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
                                                 │ HTTP
                                                 ▼
                                    +--------------------------+
                                    │ spendguard_shim          │
                                    │ (FastAPI reservation     │
                                    │  gateway — POC subset)   │
                                    +--------------------------+

   ─► /results/{runner}.json (each runner's self-report)
   ─► analyzer reads both, applies pricing table, writes RESULTS.md
```

The `spendguard_shim` is **not** the production sidecar. It is a
FastAPI service that implements only the reserve/commit/release
mechanic — see "Honest critiques of this benchmark" below for what
that means and doesn't mean. The production sidecar (UDS gRPC,
mTLS to ledger, KMS-signed audit chain, contract DSL, etc.) is
exercised by `make demo-up` from the repo root.

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

| Model | Input ($/1M) | Output ($/1M) | Source |
|---|---:|---:|---|
| `gpt-4o` | $2.50 | $10.00 | https://openai.com/api/pricing/ |
| `gpt-4o-mini` | $0.15 | $0.60 | https://openai.com/api/pricing/ |
| `o1` | $15.00 | $60.00 | https://openai.com/api/pricing/ |
| `o1-mini` | $1.10 | $4.40 | https://openai.com/api/pricing/ |
| `claude-3-5-sonnet-20241022` | $3.00 | $15.00 | https://www.anthropic.com/pricing |

Snapshot date: 2026-05. The point of using a *centralized* table —
instead of trusting each library's internal pricing — is so that the
"ground truth" number is the same number a human would compute from
an OpenAI invoice, regardless of how stale or partial each library's
internal table happens to be.

If a runner sends a model name that isn't in this table, the
analyzer **errors loudly** rather than silently scoring it as $0.
That avoids the failure mode where a typo in a runner makes its
spending look free.

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

## Honest critiques of this benchmark

Reasons a hostile reviewer can — correctly — call this narrow:

1. **The SpendGuard runner uses a shim, not the production sidecar.**
   `spendguard_shim/app.py` is a FastAPI service that implements the
   reserve/commit/release mechanic and nothing else. The production
   sidecar adds UDS gRPC + mTLS, idempotency keys, fencing tokens,
   pricing freezes, contract DSL evaluation, KMS-signed audit chain,
   and TTL-managed reservations (see `services/sidecar/`,
   `services/ledger/`, `sdk/python/src/spendguard/client.py`). Those
   are documented qualitatively below but **not measured here**. The
   shim is the headline-finding's lower bound — production behaviour
   is at least this good plus all the audit-and-correctness work the
   benchmark doesn't exercise. Follow-up: swap the shim for a runner
   that talks to the real sidecar over UDS.

2. **The SpendGuard runner has exact-cost clairvoyance.** The
   reservation amount (`RESERVATION_USD=0.18`) is hardcoded equal to
   the actual per-call cost. A more honest scenario would jitter
   actual cost (e.g. ±30% of reservation) and exercise the
   release-on-overestimate path. With perfect knowledge, a pre-call
   gate trivially beats a post-call gate; the interesting question
   is what happens under real-world cost noise. A follow-up scenario
   will introduce that.

3. **AgentGuard's drop-in HTTP interception is domain-filtered**, not
   strictly limited to `api.openai.com`. It matches hostnames
   containing `openai.com` / `anthropic.com` (per the upstream
   source). Pointing the OpenAI client at a self-hosted proxy /
   gateway / vLLM endpoint bypasses interception. The benchmark
   surfaces this as "no abort, full overshoot." Run the same script
   against `https://api.openai.com/v1` (with a real key) and
   AgentGuard will track and abort correctly. The finding is that
   AgentGuard's cost control silently disappears the moment you put a
   gateway in front of your provider — which most production teams do.

4. **AgentBudget's loop detection is disabled here.** The runner sets
   `max_repeated_calls=100000` so the benchmark measures budget
   enforcement, not the heuristic that fires on identical-content
   loops. Loop detection is a separate, valuable AgentBudget feature
   that this benchmark doesn't exercise.

5. **Sub-second test, single client, deterministic mock.** No
   concurrency, no streaming, no provider rate limits, no real
   network latency, no retry behaviour from the upstream. This is a
   methodology test, not a production load test. `LATENCY_MS=10` and
   the mock LLM uses blocking `time.sleep` inside an async route,
   which would fall apart with concurrent agents.

6. **Mock LLM only — no real $$ is at risk.** Switching to a real
   provider would cost real money per run; the trade-off is that the
   ground-truth $ figure is computed by the analyzer rather than
   billed by the provider.

7. **The "What this doesn't measure" capability table is documentation,
   not benchmark output.** The "yes" entries for SpendGuard on
   audit-chain / multi-tenant / approvals reflect what the production
   sidecar does (see `services/`, `proto/`, the demo) — not what this
   benchmark exercised.

If any of these limitations would change a decision you're making,
say so and we'll add a scenario.

## Known limitations (operational)

- The benchmark is sequential, not parallel, to keep the call counts
  unambiguous. Production-realistic concurrency is a separate
  scenario.
- `RESULTS.md` is regenerated every run; commit it to lock in a
  snapshot.

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
