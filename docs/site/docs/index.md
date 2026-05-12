---
description: >-
  SpendGuard is an open-source runtime safety layer that gates every LLM call
  against a budget before the spend lands. Pre-call enforcement, immutable
  audit chain, framework adapters for Pydantic-AI, LangChain, and more.
---

# SpendGuard

> Runtime safety layer for AI agent budgets — sub-5ms per decision,
> immutable audit chain, deterministic policy enforcement.

## What it does

SpendGuard sits between your agent framework and the external actions
the agent takes (LLM calls, tool calls, sub-agent dispatches). At each
boundary, the sidecar consults a Contract DSL and the per-tenant budget
ledger to decide:

- ✅ **CONTINUE** — proceed with the action
- ⏸ **REQUIRE_APPROVAL** — pause until an operator approves
- 🔄 **DEGRADE** — apply a mutation patch (e.g. force smaller model)
- ⏭ **SKIP** — non-fatal skip
- ⛔ **STOP** — terminate the run

Every decision — `CONTINUE` or `STOP` — produces exactly one immutable
audit record in `canonical_events`. There is no path that produces an
effect without a corresponding audit row (Contract §6.1 invariant).

## Where it fits

```
agent step ─►  sidecar (<5ms) ─► ledger (atomic reserve / commit)
                  │                  │
                  │                  └─► audit_outbox ─► canonical_events
                  └─► contract evaluator (Stage 2 hot path)
```

## Why it matters

Agent runtimes burn money in unbounded ways:

- One bad prompt can drain $10K of GPT-4 quota
- A retry loop on a rate-limited tool call doesn't know it's draining
- Compliance teams can't tell *which decision led to which spend*

Existing options are vendor-specific (per-OpenAI, per-Anthropic) or
prompt-only (please-don't-spend-too-much, with a 26.67% violation rate
in [Microsoft AGT's red-team data](https://github.com/microsoft/agent-governance-toolkit)).

SpendGuard is the cross-runtime budget governance layer.

## Status (POC, 2026-05)

| Layer | Status |
|---|---|
| T (Trace) | ✅ end-to-end |
| L (Ledger) | ✅ Postgres SERIALIZABLE + audit_outbox |
| C (Contract DSL) | 🟡 evaluator hot path live; CEL deferred to v2 |
| D (Decision) | ✅ 8-stage transaction, end-to-end |
| E (Evidence) | ✅ audit_outbox → canonical_events |
| P (Proof) | 🟡 chain durable; per-event signing strict mode deferred |

8 demo modes all green: `decision`, `invoice`, `agent`, `release`,
`ttl_sweep`, `deny`, `agent_real_*`, `multi_provider_usd`.

## Get started

→ [Quickstart](quickstart.md) — zero to a green DENY demo in 5 minutes
→ [Concepts](concepts/architecture.md) — 6-layer architecture
→ [POC vs GA gates](poc-vs-ga.md) — what's production-ready vs not
→ [GA hardening slices](roadmap/ga-hardening-slices.md) — production
  design, implementation, test, and review gates
