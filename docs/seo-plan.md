# SpendGuard SEO & Distribution Playbook

**Status**: planning artifact. Internal — not part of the public mkdocs site. Translate items here into shippable PRs (one lever per branch).

**Audience**: anyone driving SpendGuard's organic discovery — landing pages, docs site, GitHub README, framework integrations, blog content, backlinks.

**Premise**: SpendGuard is an open-source dev tool, not e-commerce. SEO leverage points are different. We do *not* compete for broad head terms like "AI cost"; we compete for *intent-loaded long-tail queries* and *framework-coupled searches* that developers already type when they have the exact problem we solve.

---

## 0. Context — what SEO means here

For a developer-tooling project, organic discovery has three doors:

1. **Search** (Google) — developers Googling specific symptoms and framework problems
2. **Code search** (GitHub) — developers grepping for tags, file patterns, integration examples
3. **Aggregators** (HN, Reddit, awesome-lists, framework docs) — high-authority backlinks that lift the first two

Most "SEO advice" optimizes for keywords. For us, the highest-leverage move is *being the obvious answer when a developer types their problem*. That requires (a) a page that names the problem in the developer's own words, (b) a code snippet they can copy, (c) framework names in the title so the search ranks for `<framework> + budget`.

What we are *not* doing in this plan:
- Paid SEM / Google Ads
- Content farms / AI-generated keyword pages
- Link-buying schemes
- Anything that requires marketing budget approval

---

## 1. The five levers, in execution order

Ordering reflects **time-to-first-organic-visit per hour-of-work spent**, not absolute importance. Long-term, backlinks (#4) compound the most; short-term, integration pages (#1) close the conversion fastest.

### Lever 1 — Framework integration pages (highest ROI, do first)

**Action**: For each framework already integrated in `sdk/python/src/spendguard/integrations/`, create a dedicated doc page on the public site.

Frameworks already wired (verify against repo before writing):
- Pydantic-AI
- LangChain
- LangGraph
- OpenAI Agents SDK
- Microsoft AGT

**Location**: `docs/site/docs/integrations/<framework>.md` (subdir already exists per `mkdocs.yml` nav scaffold).

**Why**: developers searching "LangChain token cost limit", "Pydantic-AI budget", "OpenAI Agents SDK rate limit" don't find competing pages because most prior art either doesn't gate the *pre-call* boundary or doesn't integrate with these specific frameworks. We rank cleanly with a focused page per framework.

**Page template** (apply uniformly to every integration page):

```markdown
# <Framework> budget control with SpendGuard

> One-paragraph problem statement using the exact phrasing devs Google
> for. Example: "Stop your LangChain agent from retrying a $4 gpt-4o
> call 47 times before anyone notices."

## Why you'd want this

3–5 bullets describing the failure mode SpendGuard prevents in this
specific framework. Be concrete; reference real symptoms.

## Setup (60 seconds)

\`\`\`bash
pip install 'spendguard-sdk[<framework>]'
\`\`\`

## Wire it up

A complete, copy-pasteable code block. The reader should be able to
paste this into a fresh file and have it work. Use the framework's
idiomatic style, not SpendGuard's preferred style.

## What you get

- Pre-call budget reservation (the LLM call doesn't happen if over
  budget)
- Audit trail with signed entries
- Approval flow for over-budget retries (link to /contracts/approval)

## Common patterns

2–3 sub-headers covering the most-Googled follow-up questions:
"How do I set a per-tenant budget?", "How do I handle approvals?",
"How do I test this without burning real tokens?"

## Related

- [Quickstart](../quickstart.md)
- [Contract DSL](../contracts/dsl.md)
- [Other integrations](./)
```

**Deliverables checklist** (one PR per page, mergeable independently):
- [ ] `docs/site/docs/integrations/pydantic-ai.md`
- [ ] `docs/site/docs/integrations/langchain.md`
- [ ] `docs/site/docs/integrations/langgraph.md`
- [ ] `docs/site/docs/integrations/openai-agents.md`
- [ ] `docs/site/docs/integrations/agt.md`
- [ ] `mkdocs.yml` nav entry per page
- [ ] Each page's H1 contains the framework name verbatim

**Success metric**: within 4–6 weeks of publishing, the page ranks on Google page 1 for `<framework> budget` or `<framework> cost limit`. Check via incognito search, not your logged-in Google account.

---

### Lever 2 — Problem-first landing pages (high ROI, do second)

**Action**: Write narrative landing pages targeting symptom-shaped queries, not feature-shaped ones.

**Location**: `docs/site/docs/use-cases/<slug>.md` (create the dir).

**Target queries to own** (each gets one page; pick the 3 highest-intent first):

| Query | Slug | Angle |
|---|---|---|
| `limit openai token spend per request` | `pre-call-budget-cap.md` | Stripe-style auth/capture pattern |
| `stop runaway langchain agent costs` | `agent-runaway-protection.md` | The 3 AM retry-loop horror story |
| `llm cost reservation pattern` | `reservation-pattern.md` | Architecture deep-dive, technical SEO target |
| `pre-call budget enforcement for ai agents` | `pre-call-enforcement.md` | Why post-hoc dashboards are not control |
| `ai agent approval workflow human in the loop` | `human-approval-flow.md` | The require_approval decision kind |
| `multi-provider llm cost normalization` | `multi-provider-usd.md` | Token-kind → USD conversion |

**Why**: these queries have *low competition + high intent*. Someone Googling "stop runaway LangChain agent costs" is past the awareness stage — they're shopping for a solution today.

**Page structure** (loose template, vary the voice per page):

```markdown
# <The exact problem, phrased as a noun phrase>

> 1-paragraph lede that mirrors the searcher's mental state.
> "It's 3 AM. Your agent has retried the same gpt-4o call 47 times.
> By the time you notice, you've burned $400."

## Why the standard answer doesn't work

The "track usage, send alerts" reconciliation pattern. Why it fails.

## The pattern that does

Conceptual explanation with one diagram (ASCII or Mermaid).

## Show me the code

Minimum-viable code snippet (one of the framework integrations).

## Read more

Links to:
- The relevant integration page (Lever 1)
- The relevant spec under /reference/
- The roadmap entry if this capability is partial
```

**Deliverables**: 3 pages from the table above (the top 3 highest-intent ones), each with internal links to Lever-1 pages.

**Success metric**: page appears in Google Search Console for the target query within 30 days of indexing. Top 10 within 90 days.

---

### Lever 3 — README as an SEO asset (one afternoon, do third)

**Action**: Optimize `README.md` at repo root for Google ranking, not just human readers. Google indexes GitHub READMEs aggressively and ranks them well for relevant queries.

**Why**: developers Googling `agentic-spendguard` or related capability terms land on the GitHub repo page first. The README *is* a landing page; treat it as one.

**Audit checklist**:
- [ ] First paragraph contains the primary capability phrase ("runtime budget enforcement for AI agents", "pre-call cost gate for LLMs", or similar — pick one and use it verbatim).
- [ ] First H2 or H3 explicitly names the problem ("Stop the bill before it lands" is good; reinforce it).
- [ ] Every diagram has alt-text describing what it shows (Google's image index reads alt-text, screen readers do too).
- [ ] Anchor text on inbound links to other repo pages uses descriptive phrases, not "click here" or "see this".
- [ ] Cross-links to the docs site (when public) and to the integration pages.
- [ ] Badges that link back to authoritative external sources (PyPI, license, build status) — these are mini-backlinks Google notices.
- [ ] One canonical "What this is, in one sentence" line near the top, suitable for Google's snippet.

**Don't do**:
- Stuff keywords artificially
- Add a long "table of contents" at the top (pushes content below the fold)
- Use ASCII art that breaks mobile rendering

**Deliverables**: a single PR titled `docs: SEO pass on root README`. Mostly word-level edits.

---

### Lever 4 — Backlinks (slow burn, do fourth)

**Action**: Earn backlinks from sources Google trusts for developer tooling. Backlinks compound; expect zero short-term return but the strongest long-term ranking effect.

**Tactics, ordered by effort × yield**:

1. **PRs into upstream framework repos**
   - Open a PR to `pydantic/pydantic-ai`, `langchain-ai/langchain`, etc., adding SpendGuard to their list of integrations / examples / community packages.
   - These PRs frequently get merged for popular framework expansions.
   - Each merged PR = one high-authority backlink that compounds for years.

2. **awesome-list inclusion**
   - `awesome-llmops`, `awesome-agents`, `awesome-ai-safety` — submit a PR adding SpendGuard with a one-line description.
   - Low effort, decent yield.

3. **HN Show HN post**
   - One well-timed Show HN with a story-shaped title earns a flood of direct visits *and* a permanent backlink on news.ycombinator.com.
   - Coordinate with Lever 2 (the landing pages should be live before the post).
   - Title formula: concrete-narrative + dollar figure. "We watched our agent burn $400 in 6 hours; here's what we built to stop it."

4. **Technical deep-dive blog posts**
   - One post on dev.to and one on Medium (cross-posted to /r/programming and HN), titled around the *technical pattern*, not the product.
   - Examples: "Stripe-style auth/capture for LLM budgets", "Append-only audit chains for agent decisions".
   - Embed code snippets that link back to the repo.

5. **Conference / meetup talks**
   - Lower urgency. Long lead time. Submit to relevant CFPs but don't gate other levers on this.

**Deliverables checklist**:
- [ ] 5 framework-repo PRs drafted (one per integrated framework)
- [ ] 3 awesome-list submissions
- [ ] 1 HN Show HN post drafted (held until Levers 1, 2, 3 are live)
- [ ] 1 dev.to + 1 Medium post drafted

**Success metric**: 10+ unique referring domains within 90 days, measured in Google Search Console.

---

### Lever 5 — Technical SEO config (necessary, do last because it's quick)

**Action**: configure mkdocs-material's built-in SEO features. Most are one-line changes. Marginal individually but cheap to do all at once.

**Location**: `docs/site/mkdocs.yml` + plugin config.

**Checklist** (one PR, all changes together):
- [ ] `site_url:` set to the public canonical URL. mkdocs-material emits `<link rel="canonical">` automatically once this is set.
- [ ] `site_description:` written by a human, 150–160 characters, mentions the primary capability phrase.
- [ ] Enable the `social` plugin (mkdocs-material): auto-generates Open Graph + Twitter Card images per page.
- [ ] Confirm `sitemap.xml` is emitted on build (mkdocs-material does this by default).
- [ ] Add `robots.txt` to `docs/site/docs/` pointing to the sitemap.
- [ ] Per-page `meta.description:` front-matter on the top 10 pages (landing, quickstart, each integration). Don't let mkdocs auto-truncate the first paragraph as the description — write it.
- [ ] Open Graph image: confirm one is being generated by the social plugin; for the homepage, override with a hand-crafted image showing the dashboard or the "agent → sidecar → STOP" diagram.
- [ ] Structured data (JSON-LD): if the social plugin doesn't emit `SoftwareApplication` schema, add a small `extra_javascript` hook to inject it on the homepage only.

**Don't do**:
- Add a separate analytics pixel for every social network
- Add cookie banners unless legally required (most dev tools don't need one)
- Hide content behind email gates (drops indexing rank)

**Deliverables**: a single PR titled `docs(site): technical SEO config (canonical, OG, sitemap, descriptions)`.

---

## 2. Keyword target list

Curated long-tail queries with low competition + high intent. Use these as page slugs (Levers 1 + 2) and as anchor text in cross-links.

**Highest priority** (every one of these should resolve to a SpendGuard page within 90 days):

```
limit openai token spend per request
stop runaway langchain agent costs
ai agent budget cap pre-call
llm cost reservation pattern
pre-call budget enforcement for ai agents
langchain budget control
pydantic-ai cost limit
openai agents sdk rate limit
human approval flow llm
multi-provider llm cost normalization
```

**Second tier** (target opportunistically when writing Lever-2 pages):

```
agent runaway protection
llm spend dashboard alternative
prevent gpt-4 retry loop
llm api key gateway open source
ai agent fail-closed budget
signed audit log llm decisions
contract dsl for ai agents
microsoft agt policy engine integration
```

**Negative keywords** (avoid in page copy — they attract the wrong audience):

```
free ai
unlimited tokens
bypass rate limit
crack openai
```

---

## 3. Sequencing summary

```
Week 1: Lever 5 (technical SEO config) — one afternoon
        Lever 3 (README pass) — one afternoon
Week 2: Lever 1 PRs land — one integration page per day
Week 3: Lever 2 — top 3 use-case pages
Week 4: Lever 4 — first batch of framework-repo PRs + awesome-list submissions
Week 6: HN Show HN post, dev.to article (coordinated)
```

Ordering rationale:
- 5 + 3 are quick wins that benefit everything downstream — do them first so Lever 1/2 pages launch with proper OG cards and canonical URLs.
- 1 before 2 because integration pages have higher intent and more direct conversion paths than narrative pages.
- 4 last because backlinks pointing at incomplete / un-optimized pages waste link equity.

---

## 4. Out of scope (for now)

- Paid search (Google Ads, LinkedIn) — defer until organic baseline established
- Newsletter sponsorships — defer until product has a clearer "land" event to measure against
- SEO for the dashboard UI / control plane — those are operator surfaces, not discovery surfaces
- Internationalization / multi-language docs — defer until English organic is healthy

---

## 5. How we'll know it's working

Three measurable signals, in priority order:

1. **Organic clicks from Google Search Console** — segment by Lever-1 vs Lever-2 pages. Lever-1 pages should accumulate framework-name queries; Lever-2 should accumulate problem-shaped queries.
2. **Referring domains in Search Console** — count of distinct domains linking to anywhere on the site. Compounds from Lever 4.
3. **GitHub stars + PyPI download trajectory** — lagging indicator; useful to confirm the funnel converts after the click.

What we explicitly do **not** chase:
- Pure traffic numbers (volume without intent is noise)
- DA / DR scores from third-party SEO tools (they're proxies and noisy)
- Bounce rate (developer docs sites legitimately get high bounce; not a reliable signal here)

---

## 6. Open questions

- **Do we have a public docs URL yet?** Lever 5 (canonical URL) and Lever 4 (backlinks) both depend on a stable public URL. If the docs site isn't deployed yet, that becomes a Lever-0 prerequisite.
- **Who owns the HN post timing?** The Show HN window matters; one shot per project effectively.
- **Translations?** Mandarin/Japanese developer audiences are sizeable for AI tooling. Out of scope here but worth a future plan.
