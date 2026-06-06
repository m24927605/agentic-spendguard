# D03 — `OPENAI_BASE_URL` Drop-in Landing Page — `design.md`

> Status: Doc-first spec, lands before any slice. Scope-lock document.
> Sibling docs: `implementation.md` (slice plan + page skeleton), `tests.md`
> (link / MDX / build / screenshot regression), `acceptance.md` (concrete
> ship gates), `review-standards.md` (per-slice R1-R5 gate).
> Build plan reference: `docs/strategy/framework-coverage-build-plan-2026-06.md` §2.1 (Tier 1 #D03).
> Strategy reference: `docs/strategy/framework-coverage-2026-06.md` Pattern 2.
> Audience: Project owner (sign-off), Technical Writer implementer (target),
> R1-R5 reviewer (verification).

---

## 1. What we are shipping and why

A single page on `docs.agenticspendguard.dev` that turns the Pattern 2
ecosystem — every tool that already accepts an OpenAI-compatible base URL —
into one drop-in install table. The reader lands on the page, finds the
tool they already use in a 14-row matrix, copies one env var or one config
line, and SpendGuard is in the request path on the next call. No SDK
install, no code change, no rebuild.

This is the marketing wedge for the framework-coverage workstream. Pattern 1
(model-abstraction middleware) ships as N adapter packages; Pattern 3 (egress
proxy + CA install) ships as the `spendguard install` script gated by
D02. Pattern 2 is the lane where SpendGuard wins by removing every step
between "I saw the announcement" and "it's live in my agent." A single page
is the entire deliverable; the bar is "did a developer using one of these
14 tools take the action in under 90 seconds."

### 1.1 In-scope tools (14)

The set is locked to the Pattern 2 column in `framework-coverage-2026-06.md`.
Each tool is included only if (a) it supports a custom OpenAI-compatible base
URL with at most one config line / env var change in mainline (not nightly),
(b) the change does not require recompiling the tool, and (c) the
maintainer's docs publicly document the setting on 2026-06-06.

| # | Tool | Setting kind | Strategy memo citation |
|---|------|--------------|------------------------|
| 1 | LiteLLM (proxy mode) | env `OPENAI_API_BASE` | Pattern 2 row 1 |
| 2 | Aider | env `OPENAI_API_BASE` | Pattern 2 row 2 |
| 3 | Continue | YAML `apiBase` in `config.yaml` | Pattern 2 row 3 |
| 4 | Cline / Roo Code (BYOK) | UI: Custom OpenAI provider, base URL field | Pattern 2 row 4 |
| 5 | OpenHands (BYOK) | UI: LLM custom endpoint | Pattern 2 row 5 |
| 6 | Goose | env `OPENAI_HOST` (native) | Pattern 2 row 6 |
| 7 | Zed AI | TOML `api_url` (native) | Pattern 2 row 7 |
| 8 | GitHub Copilot CLI (BYOK) | env `COPILOT_PROVIDER_BASE_URL` (GA 2026-04-07) | Pattern 2 row 8 |
| 9 | Tabnine Enterprise | Admin UI: BYO LLM endpoint | Pattern 2 row 9 |
| 10 | AnythingLLM | Admin UI: Custom OpenAI-compatible base URL | Pattern 2 row 10 |
| 11 | LobeChat | UI: Custom base URL (native) | Pattern 2 row 11 |
| 12 | Cody self-hosted Enterprise | Sourcegraph relay endpoint | Pattern 2 row 12 |
| 13 | Augment (BYOK) | UI: LLM custom endpoint | Pattern 2 row 13 |
| 14 | Dify (Model Provider Plugin) | Plugin manifest: custom provider plugin | Pattern 2 row 14 |
| 15 | CrewAI Studio (via LiteLLM) | Indirected through LiteLLM row 1 | User directive (entry 15) |

Row 15 is included per the user directive at deliverable kickoff. CrewAI
Studio reuses the LiteLLM proxy path; the table entry redirects to row 1
rather than duplicating the configuration. This keeps the per-tool count
honest at 14 distinct configuration mechanisms (row 15 is documentation
of the indirection).

### 1.2 Coordination with D33 and D34

D33 (AnythingLLM recipe) and D34 (LobeChat recipe) are sibling deliverables
that ship per-tool deep-dive pages. D03 contains the **table entry** —
the one-line settings call-out and the "Open the recipe →" link. The
**per-tool walkthrough** with screenshots and step-by-step lives in the
D33/D34 dedicated pages and is linked from D03. This deliverable does not
ship those pages; it ships their table rows and the outbound link
targets. If D33/D34 have not yet shipped at D03 merge time, the link
targets resolve to placeholder stubs created by D03 SLICE 1 — see
`implementation.md` §4.3.

### 1.3 Anti-scope

Items explicitly out of scope of D03 (the line that separates docs from
infrastructure):

- **No tool-specific recipe page.** Per-tool deep dives are D33 / D34 /
  the future D30 (claude-agent-sdk egress proxy recipe). D03 is the index.
- **No new SpendGuard code.** D03 is documentation only; the egress
  proxy / sidecar / ledger surfaces it talks about already exist. If a
  page would require a code change to be correct, the change moves to
  the relevant infrastructure deliverable and D03 documents the state
  after that change ships.
- **No Pattern 1 / Pattern 3 content.** Pattern 1 (SDK middleware) and
  Pattern 3 (egress proxy + CA install) get their own landing surfaces
  inside `/docs/integrations/` and `/docs/install/`. D03 links to them
  in the page footer but does not describe them.
- **No verified-end-to-end label on tools we have not run.** The
  README `## 🧰 What works today` table is the only surface that carries
  the green check for `openai-python`, `LangChain ChatOpenAI`,
  `LangGraph`, `openai-agents shorthand`, and streaming. The D03 table
  uses a separate **install-attested** column (see §3.2 below); the
  verified-by-real-API column on the existing README is NOT replicated
  on D03 to avoid drift between the two surfaces.
- **No comparison-to-competitors table.** The Cloudflare / Portkey /
  Databricks comparison is a separate marketing page (also referenced
  in the strategy memo); D03 references it in the page footer.
- **No marketing analytics integration in spec phase.** Page-level
  analytics instrumentation (link-click conversion, scroll depth) is
  deferred to a post-D03 marketing deliverable; D03 ships the
  page-level structure that makes analytics easy to add later, but
  does not pull in a tracking library.

---

## 2. Information architecture

### 2.1 URL and site placement

```
docs.agenticspendguard.dev/docs/drop-in/                    ← D03 landing (this spec)
docs.agenticspendguard.dev/docs/drop-in/anythingllm/        ← D33
docs.agenticspendguard.dev/docs/drop-in/lobechat/           ← D34
docs.agenticspendguard.dev/docs/integrations/*              ← Pattern 1 SDK adapters (existing)
docs.agenticspendguard.dev/docs/install/                    ← Pattern 3 install script (D02)
```

The site's Starlight config already groups `Adapter integrations` and
`Deployment` as sidebar sections. D03 adds a new top-level sidebar group
`Drop-in (Pattern 2)` between `Adapter integrations` and `Deployment`
that contains:

- `Overview` (this page — the landing)
- `AnythingLLM recipe` (D33 — placeholder link in D03 SLICE 1)
- `LobeChat recipe` (D34 — placeholder link in D03 SLICE 1)

Slug per the convention: `docs/drop-in/index` for the landing,
`docs/drop-in/anythingllm` and `docs/drop-in/lobechat` for the recipes.
Trailing-slash on; pages are Markdown not MDX unless the interactive
component lands in SLICE 3.

### 2.2 Page anatomy

Top-to-bottom on the rendered page:

1. **Hero** — one-line value proposition, the bash snippet that starts a
   local SpendGuard egress proxy in 30 seconds, and a `Find your tool ↓`
   anchor link.
2. **One-paragraph "How Pattern 2 works"** — a callout explaining the
   architectural pattern in 5 lines so the reader understands what they
   are turning on. Distinguishes Pattern 2 from Pattern 1 and Pattern 3
   in one sentence each.
3. **The 14-tool matrix** — single table with columns: Tool, Setting
   kind, Setting value (env var or config line), Recipe link. Each row
   is collapsible if Slice 3 ships the interactive component; if Slice 3
   is deferred, each row links to a section anchor lower on the page.
4. **Per-tool detail sections** — one H3 per tool (14-15 sections total).
   Each section is at most ~80 lines: prerequisites, the exact env var or
   config block to set, what the user does to verify SpendGuard is now
   in the request path (one curl or one tool-native check), known
   gotchas (auth header forwarding, SSL trust, tools that re-write the
   path), and a link to either the dedicated recipe (D33/D34) or to
   the existing per-tool docs (rows 1, 6, 8, 11 — natives).
5. **What to do next** — three links: provision a real budget (Helm /
   Compose), wire the dashboard, get serious (Pattern 3 install for the
   non-Pattern-2 surface of your stack).
6. **Footer** — one line each: Pattern 1 (SDK adapters) / Pattern 3
   (egress proxy + CA install) / comparison to Cloudflare AI Gateway,
   Databricks Unity, Portkey.

The page is intentionally long-form. Search-engine visibility for queries
like "spendguard aider", "spendguard openhands base url",
"spendguard copilot CLI" requires the per-tool section to exist on the
page with the keyword in the H3. The H1 is "Drop in SpendGuard in 30
seconds (14 tools, one env var)" to capture the matching long-tail.

### 2.3 Visual style

- No emojis in body text. Emojis only as cell content in tables where
  status / verification icons are useful, per the build plan's content
  conventions.
- Code blocks use Starlight's syntax highlighting; `bash` and `yaml` and
  `toml` are the only languages used. Inline env vars wrap in
  backticks (`OPENAI_API_BASE`).
- No `<details>` or `<summary>` collapsibles in Slice 1 / Slice 2. The
  optional Slice 3 interactive component (a tool-selector that highlights
  the relevant row + smooth-scrolls to the H3) is an MDX-only addition
  if it ships.
- One "Open the recipe →" outbound link per row, no in-table icons or
  badges. The matrix has to render legibly on mobile (320 px) — the
  Tool column is the only thing that needs to remain readable;
  Setting value can truncate-with-tooltip on mobile per Starlight's
  default table behaviour.

---

## 3. Key design decisions

### 3.1 Markdown vs MDX

**Decision:** Slice 1 and Slice 2 ship pure Markdown (`.md`) into
`src/content/docs/docs/drop-in/index.md` per the existing convention.
Slice 3, if it ships, converts the page to `.mdx` (`.../drop-in/index.mdx`)
to host an `<DropInPicker />` Astro component. The Astro / Starlight
build supports both; the MDX upgrade is non-breaking because Starlight
treats `.md` and `.mdx` identically for routing. The `.md → .mdx` rename
is the only file move; the URL slug is unchanged.

**Why:** Slice 1 + Slice 2 are content, not interaction. Punting MDX
until Slice 3 keeps the per-tool copy reviewable independently of any
JavaScript / hydration concern. If Slice 3 is descoped, the page still
ships as a self-sufficient Markdown landing.

### 3.2 The "install-attested" column

**Decision:** The matrix carries a `Verified` column with three values:
`Live` (we have run the configuration end-to-end against the egress
proxy on a CI box in the last 30 days), `Spec` (we have read the tool's
docs, the configuration is correct per the maintainer's documentation,
but we have not run it ourselves yet), `Vendor-native` (the tool's
own changelog / docs commit cite a `OPENAI_BASE_URL`-equivalent setting;
no SpendGuard-specific testing). At ship time of Slice 2:

- `Live` = rows 1 (LiteLLM proxy), 6 (Goose), 11 (LobeChat —
  enabled by D34 smoke).
- `Spec` = rows 2 (Aider), 3 (Continue), 4 (Cline), 5 (OpenHands),
  9 (Tabnine), 10 (AnythingLLM — promoted to `Live` by D33 smoke),
  12 (Cody), 13 (Augment), 14 (Dify), 15 (CrewAI Studio).
- `Vendor-native` = rows 7 (Zed AI), 8 (Copilot CLI BYOK).

**Why:** Differentiates "we know it works" from "the tool's docs say
it should work" from "the tool announced support in their changelog
and we have not done anything." Avoids over-claiming on tools that we
have not tested while still listing them so search engines and readers
can find them. The `Live` and `Spec` labels are SpendGuard's claim;
the `Vendor-native` label is the tool's own claim.

### 3.3 Drift control with the README

**Decision:** The README's existing `## 🧰 What works today` table
(README.md:131-141 at the time of this spec) stays as-is. D03 introduces
a NEW table with a different intent (drop-in landing, not
end-to-end verified) and links the README's table from the D03 page's
footer ("For the runtime-verified clients, see the README"). The README
gets a single new row at the bottom of `## 🔌 Adapter integrations`
linking out to D03's URL — the row is added as part of D03 Slice 2 and
covered by `acceptance.md` §3.2.

**Why:** The two tables answer different questions ("what client SDKs
will we vouch for end-to-end?" vs. "what tools can I drop in via one
env var?"). Merging them would force the drop-in page to either
under-claim (mark every Spec row as not-yet-verified, killing the
marketing wedge) or over-claim (mark Aider as verified before we
actually run it). Two tables, two scopes, one cross-link.

### 3.4 Per-tool exact setting strings

**Decision:** Every per-tool detail section ships the **exact** string
the maintainer's docs use, not a SpendGuard-paraphrased version. If
Aider's docs say `export OPENAI_API_BASE=...`, the page uses that
literal command and links to the upstream docs page. If Continue's
docs example uses `apiBase: ...` inside `models.openai.config`, the
page reproduces the YAML structure including the key path, not just
the leaf key. This is the single most important quality bar on the
page: an incorrect env var name on this surface is a marketing
failure that puts SpendGuard in the public worse-than-useless
quadrant.

**Verification:** The slice impl's tests (`tests.md` §3) include a
link-check that fetches each cited upstream docs page and confirms
the env var / config key appears verbatim in the upstream page's HTML.
If the upstream page has changed and no longer contains the cited
string, the CI link-check fails and the slice cannot merge. This is
the hard quality gate on per-tool accuracy.

### 3.5 Hosting the egress proxy URL example

**Decision:** Every per-tool section's example uses
`http://localhost:9000/v1` as the base URL value — matching the
existing README and the egress proxy's default bound port. The page
includes one early callout explaining how to swap this for a remote
SpendGuard URL (a hosted SpendGuard, a Kubernetes service URL, a
sidecar URL inside a pod) without repeating the swap in 14 sections.

**Why:** Localhost is the path of least friction for a 90-second
first-time experience. Production deployment is covered by the
existing `/docs/deployment/*` pages and is one footer link away.

### 3.6 Cross-deliverable test isolation

**Decision:** D03's acceptance gates run independently of D33 / D34 /
D02. The page must build, link-check, and render even if D33 / D34 /
D02 have not shipped. Placeholder stub pages for D33 and D34 (a single
H1 + "Recipe in progress, see D03 row 10/11 for the env var") are
created in D03 Slice 1 and replaced atomically when D33 / D34 ship.

**Why:** Per the build plan's Phase A / Phase B notification cadence,
the user is notified at the end of all spec sets, then again at the
end of all slices. D03 cannot block D33 / D34 cannot block D02. A
broken cross-link is an `acceptance.md` failure; a missing target
file is one — placeholder stubs make both gates green from D03
SLICE 1 onward, regardless of sibling deliverable timing.

---

## 4. Interfaces

D03 is documentation; the relevant "interfaces" are the URLs the page
exposes and the cross-link contracts with sibling deliverables.

| Contract | Interface | Owner | Verification |
|---|---|---|---|
| Page URL | `https://agenticspendguard.dev/docs/drop-in/` (trailing slash) | D03 SLICE 1 | Astro build emits the route; `tests.md` §2.1 |
| Sidebar group | `Drop-in (Pattern 2)` registered in `astro.config.mjs` | D03 SLICE 1 | `tests.md` §2.2 |
| D33 link target | `/docs/drop-in/anythingllm/` exists (real or stub) | D03 SLICE 1 + D33 | `tests.md` §2.3 link-check |
| D34 link target | `/docs/drop-in/lobechat/` exists (real or stub) | D03 SLICE 1 + D34 | `tests.md` §2.3 link-check |
| README row | `## 🔌 Adapter integrations` gains one row pointing at the D03 URL | D03 SLICE 2 | `acceptance.md` §3.2 |
| Upstream cite check | Each per-tool section's cited setting string appears verbatim on the upstream docs page | D03 SLICE 2 | `tests.md` §3.1 |
| `framework-coverage-2026-06.md` Pattern 2 table | Each in-scope tool on the strategy memo must have a row on D03; D03 must not list tools absent from the strategy memo | D03 SLICE 1 | `tests.md` §3.2 cross-check |

---

## 5. Slicing

Two slices, one optional Slice 3 (the interactive component). Total
shipped page size at Slice 2 is ~600 LOC of Markdown, well below the
1000-LOC slice cap.

### Slice 1 — Page skeleton + matrix + per-tool sections + sidebar wiring

**Goal:** The page renders, the sidebar shows the new group, the 14-tool
matrix is populated with `Setting kind` + `Setting value` + `Verified`
columns, every per-tool H3 section exists with its env var / config
block, and the D33 / D34 stub pages exist so cross-links are not broken.
This slice ships the full content as Markdown; copy is functional but
not yet polished.

**Files touched:** see `implementation.md` §4.1.

**Verification:** Astro build green; sidebar renders the new group;
every per-tool anchor resolves; link-check passes against upstream
docs pages.

### Slice 2 — Copy polish + README sync + screenshot regression baseline

**Goal:** Copy is rewritten for the second-person, present-tense,
active-voice voice required for `docs.agenticspendguard.dev`. The
hero, the "How Pattern 2 works" callout, and the per-tool gotchas are
all hand-edited (not template-filled) to read like prose rather than
spec output. The README gains the cross-link row in
`## 🔌 Adapter integrations`. Screenshot regression baselines are
captured for the rendered page at 1280px and 375px viewport widths.

**Files touched:** see `implementation.md` §4.2.

**Verification:** Vale or markdownlint passes (whichever the repo
already uses; none required if neither is configured — see
`tests.md` §5.1); screenshot diff vs. Slice 1 baseline is the
documented change set, no unexplained pixel drift outside the edited
sections.

### Slice 3 — Optional `<DropInPicker />` MDX component

**Goal:** Convert the page to `.mdx`; add an Astro component that
renders a select widget at the top of the page. Choosing a tool
smooth-scrolls to the matching H3 and highlights the matching matrix
row. Pure progressive enhancement: page reads correctly without
JavaScript, the picker is icing.

**Files touched:** see `implementation.md` §4.3.

**Verification:** Page still renders identically with JS disabled;
picker selection emits a hash-based URL update (`#cline`, `#zed-ai`)
that the back/forward buttons handle correctly; Astro build green
with `.mdx` toolchain in place.

Slice 3 is **descoped automatically** if Slice 1 or Slice 2 push the
shipped page over 1500 LOC, or if Slice 2 ships in fewer than 3 days
and the R1 reviewer flags Slice 3 as out of marginal value. The fall-
back is the H3-anchor-link approach already shipped in Slice 1.

---

## 6. Rationale recap

D03 is the wedge: a tool that's not Cloudflare-only, Databricks-only, or
SDK-only, that any developer can adopt in 90 seconds. The bar is not
"we ship a page" but "a developer using one of these 14 tools takes
the action." The two highest-risk failure modes are (a) wrong env var
name on a per-tool section (marketing failure — link-check is the
gate) and (b) drift between the README's verified-clients table and
the D03 drop-in matrix (one ships green-check claims for SDKs, the
other ships drop-in claims for tools; conflating them confuses the
reader). Both are addressed in §3.2-§3.4. Slice 3 is an upside lever,
not a gate.
