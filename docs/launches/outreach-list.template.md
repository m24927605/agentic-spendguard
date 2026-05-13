# Outreach list — TEMPLATE (private, do NOT commit a filled copy)

> The actual list of names + companies + contact details lives in
> `docs/launches/outreach-list.md` which is **gitignored**. This file
> is the public schema; keep your filled list local.
>
> Why local: cold-emails to specific people aren't artifacts the
> public benefits from, and committing them would (a) leak personal
> contacts to scrapers and (b) tempt re-targeting after the
> recipients politely declined.

## Schema

For each target, fill these fields. Keep ≤10 active targets at a
time — more than that, and the personalization budget gets thin and
the response rate craters.

```yaml
- company: ""
  segment: ""             # platform | ai-infra | compliance | framework-maintainer
  size: ""                # 50-200 | 200-1k | 1k-10k | 10k+
  why_them: ""            # one line on why this company specifically maps to SpendGuard's wedge
  trigger: ""             # the specific recent thing — post / job posting / OSS commit
  contact:
    name: ""
    role: ""              # platform engineer / AI infra lead / head-of-platform / staff eng
    channel: ""           # mastodon | bluesky | hn-comment | github-issue | email | linkedin
    handle_or_email: ""
  pitch_dimension: ""     # which Agentic SpendGuard dimension to lead with for this target
                          # (audit-chain | multi-tenant | contract-dsl | helm-aws | reservation-pattern)
  status: ""              # not-yet-sent | sent | replied-yes | replied-no | declined | followed-up
  last_touched: ""        # YYYY-MM-DD
  notes: ""               # any context that doesn't fit elsewhere
```

## Filling it

1. **Don't fabricate triggers.** If you can't write a real `trigger`
   line, drop the target. There are 50,000 platform engineers; the
   ones with a real trigger respond at 5-10x the rate.
2. **One contact per company.** Sending to multiple people at the
   same org reads as spray.
3. **Match `pitch_dimension` to the segment**:
   - compliance/regulated industries → audit-chain
   - SaaS platforms / AI gateways → multi-tenant
   - framework maintainers (LangChain / LlamaIndex / etc) → contract-dsl + framework adapters
   - AWS-heavy infra teams → helm-aws
   - agent-framework users → reservation-pattern + pre-call gate
4. **Status hygiene:** mark `declined` after 2 weeks of no reply.
   Never mark `replied-no` ambiguously — that's a `declined`.

## Out of scope for this list

- CFOs (codex must-fix #3 — wrong audience)
- Generic AI/ML teams without platform/infra responsibility
- Investor outreach (different funnel)
- Recruiters / community managers (they're not buyers)

## Where to find candidates

- **Hacker News comments** on posts about LLM cost / observability —
  filter for substantive comments, not "interesting!" replies.
- **Job postings** for "AI infra lead" / "platform engineer (AI
  systems)" / "head of platform engineering" at companies that
  publicly use LLMs at scale.
- **GitHub orgs** with non-trivial Python/TypeScript LLM code that
  imports `openai` / `anthropic` AND has Helm charts or Terraform
  modules — signals a platform team behind the LLM work.
- **Conference talk schedules** — KubeCon, AWS re:Invent, Strange
  Loop, RailsConf, PyCon — speakers on "LLM infrastructure" or
  "agent reliability" track.
- **Mastodon / Bluesky technical hashtags** on the same topics.

## What NOT to do

- Buy a list from Apollo / ZoomInfo. Trigger-less outreach has 0.5%
  response rate at best.
- "Personalization at scale" — write each note manually or don't
  send.
- Re-target someone who said no.
