# Show HN draft — SpendGuard

**Status**: not yet sent. Hold until two preconditions clear:

1. Google Search Console shows the docs site has been crawled (≥1 indexed page in the Coverage report).
2. At least one framework integration PR (Pydantic-AI / LangChain / OpenAI Agents SDK / Microsoft AGT) has been merged into the upstream repo — gives social proof in the thread comments.

Per `docs/seo-plan.md` §1 Lever 4 sequencing: dev.to / Medium technical deep-dive follow-up goes out 1–2 weeks after the HN post, NOT before.

---

## Title options

HN truncates titles around 80 characters. Pick one when ready to send.

1. **`Show HN: SpendGuard – stop the LLM bill before it lands`** *(55 chars; tight, mirrors README tagline)*
2. **`Show HN: SpendGuard – pre-call budget caps for AI agent LLM spend`** *(62 chars; keyword-loaded, less punchy)*
3. **`Show HN: I built a runtime gate after my agent burned $400 overnight`** *(70 chars; story-shaped)*
4. **`Show HN: Stripe-style auth/capture for LLM budgets`** *(51 chars; abstract, intriguing to infra readers)*

**Recommended**: #1. Falls back to #3 if you want to A/B the story-shaped variant on a different day.

---

## Body

```text
SpendGuard is an open-source runtime gate that refuses LLM API calls
which would exceed a budget — *before* the request goes to the
provider. Think Stripe-style auth/capture, but for token spend.

Why I built it: a Pydantic-AI agent hit a tool error at 3 AM and
retried the same gpt-4o call ~30 times before anyone noticed. The
bill landed in the dashboard six hours later. The standard "track
usage, send alerts" loop is reconciliation, not control — you see the
bill *after* it lands. I wanted the gate to live in the request path.

How it works:

- Per-pod sidecar (Rust, tonic gRPC over a Unix Domain Socket) holds
  an auth/capture ledger backed by Postgres.
- SDK adapters wrap the framework's model object: every Model.request()
  reserves against a budget *before* making the upstream LLM call.
- Over-budget calls raise DecisionStopped — the upstream request never
  ships.
- Idempotent reservations: a retried call with identical inputs
  collapses onto the original reservation, so a 47-retry loop allocates
  one reservation, not 47.
- Every decision is signed (Ed25519 or AWS KMS ECDSA P-256) and
  recorded in an append-only audit chain.
- L0 → L3 enforcement strength: from advisory SDK logs at L0 up to a
  provider-key gateway at L3 where the agent never sees the API key.

Framework adapters today: Pydantic-AI, LangChain, LangGraph, OpenAI
Agents SDK, Microsoft AGT.

What it isn't:

- Not a usage dashboard. It gates calls; you still need your own
  analytics for trends.
- Not a billing system. The provider still bills for calls that go
  through.
- Not yet validated end-to-end on a real K8s cluster. Helm chart
  template-renders cleanly, but the kind validation is a tracked
  follow-up.

Tech stack: Rust 1.91 sidecar, Postgres 15 ledger with append-only
audit_outbox and DB-enforced immutability triggers, mTLS gRPC between
every service, signed CloudEvents, Python SDK on PyPI.

Repo:  https://github.com/m24927605/agentic-spendguard
Docs:  https://m24927605.github.io/agentic-spendguard/
PyPI:  https://pypi.org/project/spendguard-sdk/

Apache 2.0. Solo project; happy to take feedback on the design,
especially the contract DSL surface and the reservation TTL semantics
under crash-restart.
```

---

## Pre-flight checklist

Verify each before clicking submit. Two minutes of due diligence saves a botched launch.

- [ ] Google Search Console: Coverage report shows ≥10 indexed pages
- [ ] At least one framework PR merged upstream (use to drop into a comment: "we're in LangChain's examples")
- [ ] `make demo-up` works on a clean checkout — test on a fresh container or VM. Readers will try this within 5 minutes
- [ ] Repo README first screen is clean (no broken badges, no stale TODOs)
- [ ] PyPI install path works end-to-end: `pip install 'spendguard-sdk[pydantic-ai]'` + the snippet from `docs/site/docs/integrations/pydantic-ai.md` actually runs
- [ ] You have 2–3 hours of focused, uninterrupted time IMMEDIATELY after posting to answer comments
- [ ] Laptop plugged in, coffee/water within reach
- [ ] HN front page checked — if a huge story is dominating (election, major outage, big launch), wait a day

---

## Posting strategy

### Timing

- **Day of week**: Tuesday or Wednesday
- **Time**: 8–10 AM US Pacific (peak US dev traffic on HN). 7 AM Pacific = 10 AM Eastern, catches US East Coast morning + US West Coast pre-work
- **Avoid**: Mondays (catchup from weekend), Fridays (US winding down), weekends (lower engagement), major news days

### The first 30 minutes are critical

- HN's ranking algorithm front-loads on early upvote velocity. 4–5 upvotes in the first hour can push to the front page. After 2 hours of low engagement, the post is effectively dead.
- **Do NOT ask friends to upvote.** HN detects vote rings via IP / account-age heuristics. Detected vote rings → shadow-ban → permanent strike against your account.
- **Do answer every comment thoughtfully and quickly.** Reply within minutes, not hours. Active threads stay alive.
- **Tone**: technically honest, not defensive. "X is just Y with extra steps" deserves a calm, code-referenced reply about the actual differences, not a marketing pivot.

### During the thread

- Pin a top-level comment of your own with a pre-empt of the most likely question ("Why Rust? Why not Go?" or "How does this differ from LangSmith / Helicone / Portkey?") and a thorough answer. Shows you've thought about prior art.
- Take notes on questions you can't answer — they're your product roadmap.
- If a framework maintainer comments (Pydantic-AI, LangChain core team), engage with extra care; those threads are gold.
- Don't argue with trolls. One-sentence reply with code reference, then move on.

### After the thread closes

- Whether front page or not, write a 2–4 day follow-up blog post titled "What I learned from launching X on HN". That post itself becomes a natural backlink + content piece.
- If it does well, add a "Featured on HN" callout to the README.
- If it sinks (most posts get <50 points), don't re-post. Don't beg for upvotes. Wait 2–3 months and post a "Show HN: X — major update" with a real new feature instead.

---

## What success looks like

| Outcome | What it means |
|---|---|
| **Front page (top 30)** | 5–30K unique visitors. 100–500 GitHub stars. 1–3 inbound recruiter / partnership pings. Permanent backlink. |
| **Trending (rank 30–60)** | 1–5K visitors. 30–100 stars. The thread comments may still be high-quality. |
| **Sinks (<30 points)** | 100–500 visitors. Backlink still permanent and indexed. Sometimes the *commenters* turn into early users — quality > quantity. |

The downside is bounded (you spend 3–4 hours of attention) and the upside is asymmetric (a successful HN launch can shape the next 3–6 months of inbound).

---

## After-action: the deep-dive follow-up (1–2 weeks later)

A technical post titled around the *pattern*, not the product. Examples:

- "How to build Stripe-style auth/capture for LLM budgets"
- "Append-only audit chains for AI agent decisions"
- "Why your AI agent's budget alerts are reconciliation, not control"

Cross-post:

- **dev.to** — more code, more bullets, audience reads in browser tabs
- **Medium** — more narrative, more theory, audience reads in app
- **r/programming** — lead with the most counterintuitive technical claim
- **r/MachineLearning** — frame around the model-cost angle, not the policy angle

Each platform sees the same content with subtle adaptation. These pieces age well and accumulate Google traffic for years; HN gets the spike, the deep-dives get the long tail.
