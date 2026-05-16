# Show HN draft — Agentic SpendGuard

**Status**: not yet sent. Hold until **two** remaining preconditions clear:

1. ⏳ Google Search Console shows the docs site (`agenticspendguard.dev`) has been crawled (≥1 indexed page in the Coverage report).
2. ⏳ At least one framework integration PR (Pydantic-AI / LangChain / OpenAI Agents SDK / Microsoft AGT) has been merged into the upstream repo — gives social proof in the thread comments.
3. ✅ **Reproducible benchmark vs AgentGuard / AgentBudget published.** Lives at `benchmarks/runaway-loop/` — `make benchmark` reproduces in ~30 seconds. Headline numbers vs the same $1.00 budget / 100-call runaway: SpendGuard −10% (5 calls @ $0.90, ReservationDenied at #6); agentbudget +8% (post-call BudgetExhausted at #6); agent-guard +1700% (silently bypassed by self-hosted endpoint).
4. ✅ **Auto-instrument HTTP proxy verified e2e against real OpenAI** (2026-05-17): `DEMO_MODE=proxy make demo-up` → real `gpt-4o-mini` call through `OPENAI_BASE_URL=http://localhost:9000/v1` → CONTINUE returns 200 + ledger writes `commit_estimated` row; STOP returns 429 + `BUDGET_EXHAUSTED` body, **the upstream OpenAI HTTP request never fires**. 3 wire bugs (pg pool acquire_timeout, PricingFreezeMismatch on LLM_CALL_POST, smoke-test threshold) were caught and fixed during this validation.

Per `docs/seo-plan.md` §1 Lever 4 sequencing: dev.to / Medium technical deep-dive follow-up goes out 1–2 weeks after the HN post, NOT before.

> **Positioning v2 (post-egress-proxy, 2026-05-17)**: lead with the **1-env-var auto-instrument claim** (`OPENAI_BASE_URL=...`, no SDK install, no code change). That's the Helicone-class onboarding bar, but with fail-closed enforcement, KMS-signed audit chain, and operator-approval workflow underneath. The previous "enterprise infra differentiation" lead (Stripe auth/capture / multi-tenant / L0-L3) becomes the SECOND-HALF differentiator vs Helicone / Portkey / LiteLLM (who all proxy your traffic but make decisions post-hoc). Drop "pre-call budget caps" framing (AgentGuard owns that wedge) AND drop sidecar-UDS-first framing (that's wrapper-mode, not the launch path).

---

## Title options (rebrand-safe, ≤80 chars)

HN truncates titles around 80 characters. Pick one when ready to send.

1. **`Show HN: OPENAI_BASE_URL=... and your agent now has a hard-cap budget`** *(69 chars; v2 lead with 1-env-var; concrete, technical, copy-paste-able)*
2. **`Show HN: Agentic SpendGuard – 1 env var, hard-cap your OpenAI bill`** *(66 chars; v2 lead, brand-prefixed; safer if HN audience is brand-curious)*
3. **`Show HN: I built a runtime gate after my agent burned $400 overnight`** *(70 chars; story-shaped, brand-neutral; great for AskHN-adjacent audience)*
4. **`Show HN: Agentic SpendGuard – KMS-signed budget gate for AI agent runs`** *(70 chars; v1 — reserve for follow-up post once wedge is established)*
5. **`Show HN: Agentic SpendGuard – Stripe-style auth/capture for LLM budgets`** *(71 chars; v1 — abstract; reserve for follow-up infra audience)*

**Recommended**: #1 — concrete, copy-paste-able, no jargon, the actual mental model. Reach for #2 if the audience seems brand-curious. #3 if you want a story-shaped opener. #4 / #5 reserve for the 1–2 week follow-up post once the 1-env-var wedge has landed. The original "pre-call budget caps" framing is dropped (AgentGuard owns that wedge).

---

## Body

```text
TL;DR — set OPENAI_BASE_URL=http://localhost:9000/v1 and your existing
agent code (LangChain, openai-agents, Pydantic-AI, raw openai-python —
any of them) now has a hard-cap budget. The proxy fail-closes STOP
decisions before the HTTP request reaches OpenAI. Every CONTINUE and
every STOP is signed and written to an append-only Postgres audit
chain. No SDK install, no code change.

Why I built it: a Pydantic-AI agent hit a tool error at 3 AM and
retried the same gpt-4o call ~30 times before anyone noticed. The
bill landed in the dashboard six hours later. The standard "track
usage, send alerts" loop is reconciliation, not control — you see the
bill *after* it lands. I wanted the gate to live in the request path.

How the 1-env-var path works:

- Local Rust egress proxy (axum). User points their existing OpenAI()
  client at http://localhost:9000/v1. Zero code change beyond the
  BASE_URL.
- For each incoming request, the proxy calls a sidecar (Rust, tonic
  gRPC over a Unix Domain Socket) that holds an auth/capture ledger
  backed by Postgres. Reservation is held BEFORE the upstream HTTP
  request fires.
- CONTINUE: proxy forwards to OpenAI byte-identical, parses
  usage.total_tokens from the response, commits the real spend,
  returns the response unmodified to the client.
- STOP: proxy returns 429 + a structured error body
    {"error": {"code": "spendguard_blocked",
               "details": {"reason_codes": ["BUDGET_EXHAUSTED"], ...}}}
  The upstream HTTP request never fires; no provider charge.
- Pricing for the reservation is FROZEN at PRE-call time from a
  signed contract bundle, so the COMMIT can't be poisoned by a
  pricing-table edit mid-flight.
- Every decision is signed (Ed25519 or AWS KMS ECDSA P-256) and
  appended to an audit chain backed by DB-enforced immutability
  triggers.
- For idempotent retries, identical inputs collapse onto the same
  reservation — a 47-retry runaway allocates one reservation, not 47.

How this is different from Helicone / Portkey / LiteLLM:

  Those proxy your traffic and decide *after* the call (rate limit,
  log, retry, alert). SpendGuard fails closed BEFORE the call hits
  the wire when a budget would be breached. The audit chain isn't a
  log — it's a tamper-evident ledger you can hand to compliance.

For the harder cases — REQUIRE_APPROVAL, model-DEGRADE, multi-step
spend planning — there's also a wrapper-mode SDK with adapters for
Pydantic-AI, LangChain, LangGraph, OpenAI Agents SDK, Microsoft AGT.
But the 80% wedge is the 1-env-var proxy: hard-cap + audit chain.

Reproducible head-to-head benchmark in `benchmarks/runaway-loop/`
(`make benchmark`, ~30s, no real provider $$ spent — uses a mock
OpenAI endpoint). Same fixture (100 attempted calls, $1.00 cap,
$0.18 per call) through three drop-in tools:

  spendguard:   5 calls @ $0.90 (-10% vs $1)  ReservationDenied at #6
  agentbudget:  6 calls @ $1.08 (+8%)          BudgetExhausted at #6
  agent-guard:  100 calls @ $18  (+1700%)      no abort

agentbudget overshoots because enforcement is post-call (the 6th
call lands on the wire, *then* it raises). agent-guard doesn't
enforce at all because its HTTP-level interception is hardcoded to
api.openai.com / api.anthropic.com and silently no-ops the moment
you point an OpenAI client at a self-hosted base URL — which is
what happens the second you put a gateway in front of your provider.

What it isn't:

- Not a usage dashboard. It gates calls; you still need your own
  analytics for trends.
- Not a billing system. The provider still bills for calls that go
  through (the value is what *doesn't* happen).
- Not yet validated end-to-end on a real K8s cluster. Helm chart
  template-renders cleanly, but the kind validation is a tracked
  follow-up.
- The benchmark uses a reservation-gateway shim, not the full
  production sidecar. The shim isolates the reservation dimension;
  see benchmarks/runaway-loop/README.md → "Honest critiques of this
  benchmark" for the unvarnished list.
- v0.1 proxy is OpenAI Chat Completions only and non-streaming.
  SSE pass-through is v0.2; Anthropic + Bedrock adapters after that.

Tech stack: Rust 1.91 sidecar + axum egress proxy, Postgres 15
ledger with append-only audit_outbox and DB-enforced immutability
triggers, mTLS gRPC between every service, signed CloudEvents,
Python SDK on PyPI for the wrapper-mode path.

Repo:       https://github.com/m24927605/agentic-spendguard
Docs:       https://agenticspendguard.dev/
Quickstart: https://github.com/m24927605/agentic-spendguard#quick-start--auto-instrument-1-env-var
Benchmark:  https://github.com/m24927605/agentic-spendguard/tree/main/benchmarks/runaway-loop
PyPI:       https://pypi.org/project/spendguard-sdk/  (wrapper-mode SDK; not needed for proxy mode)

Apache 2.0. Solo project; happy to take feedback on the proxy's
fail-closed semantics, the FROZEN-at-PRE pricing invariant, the
contract DSL surface, and the reservation TTL behavior under
crash-restart.
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

- Pin a top-level comment of your own with a pre-empt of the most likely question. With the v2 1-env-var lead the predictable challenge is **"how is this different from Helicone / Portkey / LiteLLM?"** — answer: those proxy your traffic and decide POST-call (log, alert, retry). SpendGuard fails closed PRE-call when a budget would be breached + signs every decision into a Postgres audit chain w/ DB-enforced immutability triggers. The proxy + sidecar split also means an attacker who compromises the proxy can't rewrite history. Link the spec at `docs/specs/auto-instrument-egress-proxy-spec.md` §4.2 (fail-closed invariant) for credibility. Also pre-empt "Why Rust? Why not Go?" — answer: zero-GC sidecar in the hot path; tonic gRPC + axum integrate cleanly; we have 6+ months of Rust ledger code already.
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
