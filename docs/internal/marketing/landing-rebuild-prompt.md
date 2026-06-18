# Landing Page Rebuild Prompt — agenticspendguard.dev

## GOAL

Rebuild the landing page of agenticspendguard.dev so that its visual
system, content rhythm, and tone match https://agentcontrol.dev/ within
±5%. The current state (PR #81 on m24927605/agentic-spendguard, branch
`feat/astro-starlight-redesign`) gets the technical scaffold right but
the visual treatment and content expression are still too "docs-site".
Throw both away and restart from agentcontrol.dev as the reference.

## WHAT TO KEEP (technical scaffold is correct)

- Astro 6 + Starlight 0.39 + Tailwind v4 in `docs/site-v2/`
- The 32 ported markdown pages under `src/content/docs/` — those remain
  as the DOCS surface, untouched
- `astro.config.mjs` sidebar (for the docs surface only)
- `public/CNAME`, `.github/workflows/docs-deploy.yml`, trailingSlash,
  URL preservation

## WHAT TO THROW AWAY

- Current `src/content/docs/index.mdx` — every paragraph of prose,
  every "Honest about where we are" card, every multi-tier disclaimer.
  agentcontrol.dev declares; it does not justify. Match that posture.
- Current `src/styles/global.css` — rewrite from scratch to match
  agentcontrol's spacing, typography hierarchy, and section rhythm.
  Do not iterate on the existing CSS; delete it.
- The Starlight sidebar on the landing surface. The landing is a
  single-column marketing page with TOP NAV ONLY — no sidebar.
  Put the docs sidebar back only when the URL is under `/docs/*`.

## IMPLEMENTATION

Create `src/pages/index.astro` as a standalone Astro page (outside the
Starlight content collection) so you have full layout control on the
landing. Move every existing `/content/docs/` page under a `/docs/*`
URL prefix. The Starlight-rendered docs site lives entirely at
`/docs/*`; the `/` route is your custom landing.

## VISUAL SYSTEM (replicate exactly from agentcontrol.dev)

- **Layout:** single-column, centered, max-width ~1100px. Top nav with
  logo + 4-5 links + GitHub icon. No sidebar.
- **Background:** near-black `#0a0a0a`. Section backgrounds at `#0d0d0d`
  or identical (use very subtle horizontal hairlines `#1f2028` to
  separate sections instead of color blocks).
- **Typography:**
  - Font: Inter, with system-ui fallback
  - H1 (hero): `clamp(2.5rem, 5vw, 4.25rem)`, font-weight 800,
    letter-spacing -0.025em, line-height 1.05
  - Subhead: 1.25rem, color `#c2c4cc`, max-width 640px, centered,
    line-height 1.5
  - H2 (section title): 2rem, font-weight 700, letter-spacing -0.015em
  - Body: 1rem, color `#c2c4cc` on dark
  - Code: ui-monospace stack, 0.95rem, background `#161720`,
    border-radius 0.5rem, no border
- **Hero section:** ~6rem vertical padding top + bottom. H1 → 1rem
  spacing → subhead → 2rem spacing → CTA pair → 4rem spacing →
  optional small architectural diagram (3 boxes in a row with arrows).
- **CTAs:** primary = solid `#3F51B5` indigo, white text, padding
  0.75rem 1.5rem, border-radius 0.5rem, font-weight 600. Secondary =
  outlined with border `#2f3038`, transparent background, color
  `#f2f2f4`. Pair them side-by-side with 0.75rem gap. On hover:
  primary lightens to a higher-luma indigo; secondary border brightens
  to `#585b66`.
- **Feature cards:** 3-column grid (1 col mobile). Card = border
  `#1f2028`, no background (or `#0d0d0d`), padding 1.5rem, border-radius
  0.75rem. Inside: title (1rem white semibold) + ONE SENTENCE
  description (0.95rem `#c2c4cc`).
- **Section spacing:** 5–7rem between major sections.

## CONTENT EXPRESSION (this is what's still wrong — match agentcontrol's rhythm)

- One-sentence declarative headlines. No clauses, no parentheticals,
  no apologetic hedges.
- Subhead = WHAT + HOW in one sentence, ~20 words max.
- Feature cards = title + ONE SENTENCE. Never paragraphs.
- "How it works" = 3 visual boxes with one-line labels each. Not prose.
- Use cases = 6 single-word categories with icons. No descriptions
  on the landing.
- Code sample = one realistic snippet, ~10 lines, with `# expected
  output` annotations inline.
- Eliminate: "Why this exists" failure-mode story, "Honest about
  where we are" cards, every paragraph that justifies or hedges.
  Send people to `/docs/` for the nuance. The landing declares.

## CONCRETE COPY (SpendGuard-flavored, in agentcontrol's voice)

- **Subhead:** "Reserve budget before the provider is called, sign every
  decision, and stop runaway agents in p50 ≤10ms."
- **Primary CTA:** "Get Started" → `/docs/quickstart/`
- **Secondary CTA:** "View on GitHub" → `https://github.com/m24927605/agentic-spendguard`

### Section "How it works" — 3 boxes

1. **Reserve** — "Atomic per-tenant ledger debit before the provider call"
2. **Commit** — "Read response.usage, commit real spend, refund overshoot"
3. **Audit** — "KMS-signed CloudEvent for every reserve / commit / reject"

### 6 feature cards (title + ONE sentence each)

- **Pre-call reservations** — "Atomic budget debit before the provider
  is hit. Fail-closed when exhausted."
- **Signed audit** — "Every decision is a KMS-signed CloudEvent
  landing in your SIEM."
- **Multi-tenant isolation** — "Per-tenant ledgers. One runaway agent
  cannot drain another tenant."
- **Stripe-style auth/capture** — "Reserve the worst case, commit
  the real spend, refund overshoot."
- **p50 ≤10ms decisions** — "Measured per SLO contract NF1, not
  aspirational."
- **Framework-agnostic** — "Adapters for LiteLLM, OpenAI Agents SDK,
  LangChain, LangGraph, Pydantic-AI, Microsoft AGT."

### Code sample (one ~10-line snippet, inline output annotations)

```python
import litellm
from spendguard.litellm import enforce_budget

litellm.callbacks = [enforce_budget(tenant="acme")]

response = await litellm.acompletion(
    model="gpt-4o",
    messages=[{"role": "user", "content": "..."}],
)
# ❌ HTTP 403 BUDGET_EXHAUSTED  — provider was never called
```

### Use case categories (6 single words with icons)

Multi-tenant · Compliance · SLO · Audit · Cost · Egress

### Optional testimonials/logos section

Leave placeholders with 3 logo slots + "Working on a deployment?
Tell us." link to GitHub Discussions. Do not invent quotes.

## DELIVERABLE

- New `src/pages/index.astro` for the landing
- Rewritten `src/styles/global.css` that does NOT inherit from the
  current file — start from a blank file and only add what the
  landing + Starlight docs actually need
- Existing `/content/docs/*` pages re-routed under `/docs/` prefix
  (sidebar config in `astro.config.mjs` updated accordingly)
- `npm run build` green; manual local preview confirms parity with
  agentcontrol.dev side-by-side at 1280×800 viewport
- Commit on the existing branch `feat/astro-starlight-redesign`
  (don't open a new PR; this stacks on PR #81)

## CONSTRAINTS

- No emojis in copy or code
- Commits as `m24927605@gmail.com`
- `Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>`
- If the visual still feels off after a build, adjust SPACING and
  TYPOGRAPHY HIERARCHY before touching colors. The single biggest
  agentcontrol-vs-current gap is breathing room, not color.

## SUCCESS CRITERIA

Open agentcontrol.dev and agenticspendguard.dev side-by-side in two
browser windows at 1280×800. Visual rhythm, font scale, button
density, section spacing, and tonal register should be
indistinguishable to a casual viewer who is not reading the actual
words. If they look like two different design systems, iterate again.
