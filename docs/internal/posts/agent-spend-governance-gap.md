# The Agent Spend Governance Gap

*Why token counters aren't enough, and what a real solution looks like.*

---

At 02:47 UTC on a Tuesday, a customer-support agent at a mid-sized SaaS company hits a rate-limited internal tool. The retry policy kicks in. The agent loop re-plans, re-prompts, re-tries — each retry a fresh `gpt-4o` call with the full conversation history in context.

By 03:27, one stuck conversation has consumed about $380 in tokens. By 04:00, three other tenants are doing the same thing, each blissfully unaware. The on-call SRE finds out at 09:00 the next morning when the OpenAI dashboard refreshes the invoice line.

The post-mortem starts: *"We didn't know until the bill arrived."*

This is the standard failure mode of every agent system built in 2026. And the surprising thing is **how thoroughly the existing observability stack fails to fix it**.

## The three layers that already exist

If you go looking, there are standards for everything *around* the bug above:

1. **OpenTelemetry GenAI semantic conventions** describe the LLM call beautifully — token counts, model name, latency, agent steps. As of March 2026, the semconv is the de facto standard for LLM tracing. It is also, by design, **observability**. It tells you what happened. It cannot decline a call.

2. **FOCUS 1.0** (FinOps Open Cost & Usage Specification) standardizes how provider bills get reconciled into your data warehouse. Excellent for monthly chargeback. Arrives **days** after the spending. Cannot decline a call.

3. **OAuth / OIDC / APS / AgentID / ERC-8004** all answer "who is this agent." Identity layers are essential and mature. None of them know whether the agent has $5 of budget left.

These three layers cover a square three sides of a problem. The fourth side is missing.

## What the missing side actually does

The missing side is **pre-call budget enforcement** — and the pattern is already familiar to anyone who has shipped a payments integration. Stripe calls it auth/capture:

1. **Authorize** — reserve the worst-case spend before any actual money moves.
2. **Capture** — once the operation succeeds and you know the real amount, commit it.
3. **Refund** the difference. Cancel on failure. Sign every step. Make it idempotent.

Applied to LLM tokens:

1. Before the agent's `chat.completions` call, ask an enforcement authority: *"can this tenant afford `output_token=200` against budget `acme-eng-2026-05`?"*
2. ALLOW → the provider gets called. DENY → the agent gets an `HTTP 403`, the provider clock never starts, the invoice clock never starts.
3. After the response, commit the **real** `completion_tokens=87`. Refund the 113 that didn't get used.
4. Every Reserve and Commit emits a signed CloudEvent into an append-only audit log.

This isn't novel. Stripe shipped it in 2011. The novelty is that nobody applies it to LLM tokens — even though the failure mode (runaway loops at 02:47 burning hundreds of dollars in 40 minutes) is exactly the failure mode auth/capture was designed to prevent.

## Why "we use LiteLLM budgets" is not the answer

Most teams that have thought about this at all reach for the budget feature in their LLM gateway. LiteLLM has team budgets, agent iteration budgets, max_budget per key. Portkey has budgets. Helicone has limits. Cloudflare AI Gateway has cost caps.

These are useful. They are also, individually, none of the following:

- **Atomic.** Per-key counters race under concurrent calls. Two simultaneous requests can both pass a check that should have denied the second one.
- **Transactional.** A reservation that gets refunded if the call fails is a different primitive from a counter that gets incremented after the fact. Counters don't refund.
- **Auditable.** A signed receipt that says "decision DENY because `BUDGET_EXHAUSTED`, made at 02:47:13 against budget `acme-eng-2026-05`, by authority `sg.acme.internal`" is a different thing from a row in a dashboard. One survives subpoena. The other doesn't.
- **Portable.** Switching from LiteLLM to a self-hosted gateway should not lose your spend-governance contracts. Today, it does.

These products are gateway features, optimized for the vendor's product. There's no standard underneath. There needs to be one.

## Five properties of a real solution

A standard for agent spend governance — a real one, not a checkbox — has to do all five of these:

1. **Pre-call gating, not post-hoc accounting.** Detection at the 11th call, not the next morning.
2. **Atomic transactional reservations.** Reserve worst case, commit real, refund overshoot. Concurrent reserves never both succeed past the cap.
3. **Cryptographically signed audit chain.** Every decision is a signed CloudEvent landing in append-only storage your SIEM can subscribe to.
4. **Identity-layer-neutral.** Compose with APS, AgentID, ERC-8004, plain tenant strings, whatever your stack uses. Don't reinvent identity.
5. **Provider-neutral.** Same wire protocol for OpenAI, Anthropic, Bedrock, self-hosted vLLM. A protocol your customers can implement against your service or someone else's.

I'd argue these are non-negotiable. Drop any one and you're back to a vendor-specific budget counter, which is what the industry has today and which is what produces the 09:00 post-mortems.

## Where this goes next

Three things are happening, more or less simultaneously, in May 2026:

- An upstream canonical-verb set for budget reservation is already incubating in Tymofii Pidlisnyi's [`agent-governance-vocabulary`](https://github.com/aeoess/agent-governance-vocabulary). The file is [`crosswalk/budget_reservation.yaml`](https://github.com/aeoess/agent-governance-vocabulary/blob/main/crosswalk/budget_reservation.yaml) — `crosswalk_type: domain_incubation`, two production implementations crosswalked as of 2026-05-13 (goodmeta and Cycles), with a documented promotion path that lands the verbs as canonical once a third production implementer surfaces and the `proposed` verbs reach two implementers. The verb set — `reserve`, `commit`, `release`, `refund`, `query_budget` — is the right place to anchor.
- A draft wire protocol — the [Agent Spend Protocol (ASP) Draft-01](https://github.com/m24927605/agentic-spendguard/blob/main/docs/specs/agent-spend-protocol/draft-01.md) — is the agent-runtime binding of that upstream verb set. Apache 2.0. Reserve / Commit / Release / Refund / Audit semantics, signature discipline, transaction model. Not bound to any one implementation.
- An [OpenTelemetry GenAI extension proposal](https://github.com/m24927605/agentic-spendguard/blob/main/docs/internal/proposals/otel-genai-spend-extension.md) puts the spend-decision events on the same GenAI span as the provider call itself. Existing OTel dashboards keep working; spend governance becomes just another span event.

There is also a reference implementation, [SpendGuard](https://agenticspendguard.dev), built by the same group. It is Apache 2.0 and runs as a Rust sidecar today, with adapters for LiteLLM, OpenAI Agents SDK, LangChain, LangGraph, Pydantic-AI, and Microsoft Agent Governance Toolkit. But the reference implementation is not the protocol. The point of writing the spec is to make sure alternative implementations are possible — and welcome.

## What I'd like from you

If you run agent workloads in production, the most useful thing you can do is read the [ASP draft](https://github.com/m24927605/agentic-spendguard/blob/main/docs/specs/agent-spend-protocol/draft-01.md) and open an issue against anything that doesn't match how your real-world budgets work. Multi-Authority settlement, multi-provider FX, DEGRADE routing — these are all open questions in Draft-01 because the right answer comes from people who have hit the walls.

If you're working on an LLM gateway, an enforcement product, or an observability platform — the OTel extension is the path of lowest friction to interop. We'd love feedback in the OTel GenAI SIG.

If you're working on adjacent agent governance infrastructure — identity, attestation, settlement, execution boundaries — let's get terminology aligned now so we don't end up with seven incompatible vocabularies in two years. The vocabulary repo above is the natural place.

The agent economy is being built. The instrument panel needs more than tokens-per-second.

---

*Comments welcome on [GitHub](https://github.com/m24927605/agentic-spendguard/issues), [Hacker News](https://news.ycombinator.com/from?site=agenticspendguard.dev), or by replying to this post.*
