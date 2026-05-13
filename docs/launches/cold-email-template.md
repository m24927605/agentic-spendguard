# Cold-email template — Agentic SpendGuard outreach

For: platform engineering leads, AI infra leads, head-of-platform,
or compliance engineering. **Not** for CFOs, finance, procurement, or
generic AI ops — those audiences will land on the wrong dimension of
the pitch and the conversation derails into ROI vs governance.

The template assumes you have a concrete reason to reach out — a
recent post they wrote, a recent runaway-cost incident in their
public postmortem, a job posting that signals they're staffing
platform-side LLM ops, or an open-source repo of theirs that ingests
LLM cost data. Don't send cold-cold; reference the trigger.

---

## Template (≤180 words)

Subject (pick whichever is most accurate — never "[name], quick question"):

- `agent runaway cost — pre-call gate, not post-hoc dashboard`
- `KMS-signed audit chain for LLM spend (open source, AWS Helm)`
- `your <thing they shipped/wrote> + a question about LLM spend governance`

Body:

> Hi [first name],
>
> [Trigger sentence — the specific thing they wrote / shipped /
> talked about that's the reason for reaching out. One line max. If
> you can't write this honestly, don't send the email.]
>
> I'm [name] working on Agentic SpendGuard
> ([github.com/m24927605/agentic-spendguard](https://github.com/m24927605/agentic-spendguard)) —
> open-source pre-call budget enforcement for LLM agents. The
> dimension teams keep telling me matters and isn't covered by
> Helicone / Portkey / LiteLLM is **what evidence you can hand to
> compliance after the bill lands**: KMS-signed append-only audit
> chain, Stripe-style auth/capture ledger, operator approval flow
> on borderline calls.
>
> Reproducible benchmark vs the two closest open-source tools (it
> stops 1 call early; AgentBudget overshoots by 8% on the same
> scenario; AgentGuard silently no-ops with a self-hosted endpoint):
> [github.com/m24927605/agentic-spendguard/blob/main/benchmarks/runaway-loop/RESULTS.md](https://github.com/m24927605/agentic-spendguard/blob/main/benchmarks/runaway-loop/RESULTS.md).
>
> Two questions, no demo pitch:
>
> 1. Does any of this map to something you've actually been trying
>    to ship? If "no — totally different problem", that's the most
>    useful answer.
> 2. If yes — would 20 minutes next week be useful for me to learn
>    where the abstraction breaks for your stack? I'm not selling
>    anything.
>
> Either way, the repo's open and Apache-2.0. Happy to take feedback
> in a GitHub issue if email is overhead.
>
> [name]

---

## What to vary per recipient

- **Trigger sentence** — must be specific, must be honest. If you
  can't write one, the recipient is the wrong target.
- **The dimension they care about** — for compliance-heavy orgs,
  lead with the audit chain. For multi-tenant SaaS platforms, lead
  with multi-tenant scoping. For agent-framework maintainers, lead
  with the contract DSL + framework adapters. For AWS-platform
  teams, lead with the Helm + Terraform.
- **The one specific question** — tailor it. "Does this map to a
  thing you've been trying to ship?" is the default, but if you
  know they're actively building cost gates, "what would you have
  to see in a self-hosted budget gate before it goes in front of
  prod traffic?" is sharper.

## What NOT to vary

- Length. ≤180 words. If it doesn't fit, the trigger sentence isn't
  sharp enough or the pitch is overcomplicated.
- The "no demo pitch" disclaimer. Platform engineers' inboxes are
  90% sales pitches; this is the differentiator.
- The "if no, that's the most useful answer" out. If you can't
  honestly accept "no, this is a wrong target," you'll come across
  as another sales touch.

## What to skip entirely

- "Quick" anything in subject lines. Quick-question / quick-call /
  quick-coffee is sales-speak.
- "Solving runaway costs" framing. Every LLM-cost vendor leads
  there. Lead with **what compliance / platform gets that they
  can't get from a dashboard**.
- Calendar links in the first email. Ask for the conversation; book
  it in the reply.
- LinkedIn DMs as the primary channel. Email > LinkedIn for
  platform-engineering leads. (Mastodon / Bluesky public reply on
  their relevant post is often higher signal than DM.)

## Channels to use, in order

1. **Public reply** to a relevant post they wrote — Mastodon,
   Bluesky, X, Hacker News comment. Higher signal than email,
   and shows up in their notifications without adding inbox load.
2. **GitHub issue** on their public repo if there's a relevant
   technical hook ("we're using your library and considering
   adding pre-call cost gating").
3. **Email** to their professional address. Use Hunter / Apollo
   for finding it; never use a personal Gmail you scraped.
4. **LinkedIn** as a fallback only.

## Channels to NOT use

- **CFO Slack communities** — wrong audience (codex review
  must-fix #3); CFOs don't deploy infra, and the conversation
  derails into ROI/finance instead of governance.
- **Sales-style sequences** with multiple "bumping this up" follow-ups.
  One follow-up after 7-10 days is fine. Anything more is noise.
- **Cold call**. Nobody answers their phone for inbound.

## Outreach list

The actual list of target companies + contacts lives outside this
repo (intentionally — names, emails, and personal context belong in
1Password / Linear / a private notes file, not git history). See
the local-only `docs/launches/outreach-list.md` template alongside
this file for the schema.
