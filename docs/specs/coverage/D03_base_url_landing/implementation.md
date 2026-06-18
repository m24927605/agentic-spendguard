# D03 — `OPENAI_BASE_URL` Drop-in Landing Page — `implementation.md`

> Status: Doc-first spec. Lands before any slice implementation.
> Sibling docs: `design.md` (scope and IA), `tests.md`, `acceptance.md`,
> `review-standards.md`.
> Audience: Technical Writer implementer; reviewer for layout / file-touch sanity.

---

## 1. Overview

This document is the per-slice implementation plan for D03. It mirrors
`design.md` §5 (slicing) with concrete file paths, content templates,
and the canonical page skeleton the Technical Writer fills in. The
spec is intentionally explicit about file paths and section ordering;
copy quality is the Slice 2 layer, structural soundness is the Slice 1
layer.

The Astro / Starlight site lives at `docs/site-v2/`. Page content lives
under `docs/site-v2/src/content/docs/docs/drop-in/`. The sidebar is
configured in `docs/site-v2/astro.config.mjs`. No new dependencies are
added by Slice 1 or Slice 2; Slice 3 (if shipped) adds an Astro
component only — Astro already supports MDX out of the box via the
Starlight integration in the current `package.json`.

---

## 2. File layout (post-Slice 2)

```
docs/site-v2/
  astro.config.mjs                                          # MODIFIED Slice 1 (sidebar)
  src/
    content/
      docs/
        docs/
          drop-in/
            index.md                                        # NEW Slice 1
            anythingllm.md                                  # NEW Slice 1 (stub; replaced by D33)
            lobechat.md                                     # NEW Slice 1 (stub; replaced by D34)
README.md                                                   # MODIFIED Slice 2 (cross-link row)
docs/specs/coverage/D03_base_url_landing/
  design.md                                                 # this spec set
  implementation.md                                         # this doc
  tests.md
  acceptance.md
  review-standards.md
docs/internal/slices/
  COV_03_base_url_landing_skeleton.md                       # NEW Slice 1 (slice doc)
  COV_04_base_url_landing_copy_polish.md                    # NEW Slice 2 (slice doc)
  COV_05_base_url_landing_picker_mdx.md                     # NEW Slice 3 (optional; slice doc)
```

If Slice 3 ships:

```
docs/site-v2/
  src/
    content/
      docs/
        docs/
          drop-in/
            index.mdx                                       # renamed from index.md
    components/
      DropInPicker.astro                                    # NEW Slice 3
```

---

## 3. Page skeleton (target output of Slice 1)

The following is the canonical structure for
`docs/site-v2/src/content/docs/docs/drop-in/index.md`. Fenced
descriptions in square brackets `[ … ]` are author instructions and
must not appear in the shipped file. Lengths are approximate.

```markdown
---
title: "Drop in SpendGuard in 30 seconds (14 tools, one env var)"
description: >-
  Every tool that accepts an OpenAI-compatible base URL — LiteLLM proxy,
  Aider, Continue, Cline, OpenHands, Goose, Zed AI, GitHub Copilot CLI BYOK,
  Tabnine Enterprise, AnythingLLM, LobeChat, Cody self-hosted, Augment, Dify —
  works with Agentic SpendGuard out of the box. One env var, no SDK install,
  no code change.
---

> [Hero paragraph: one sentence on the value proposition, one sentence on
>  the prerequisite (a running SpendGuard egress proxy at
>  `http://localhost:9000/v1`), and a `Find your tool` anchor.]

## How Pattern 2 works

[Callout block, ~5 lines: explain that each tool already supports a
 custom OpenAI-compatible base URL; SpendGuard runs an OpenAI-compatible
 endpoint; setting the env var routes traffic through SpendGuard before
 it hits the real provider. Contrast in one sentence each with Pattern 1
 (in-process SDK middleware) and Pattern 3 (egress proxy + CA install).]

## Start the proxy locally (30 seconds)

```bash
git clone https://github.com/m24927605/agentic-spendguard.git
cd agentic-spendguard
export OPENAI_API_KEY=sk-...
make demo-up DEMO_MODE=proxy
```

Now point any of the 14 tools below at `http://localhost:9000/v1`.

## Find your tool

| # | Tool | Setting kind | Setting value | Verified | Recipe |
|---|------|--------------|---------------|----------|--------|
| 1 | [LiteLLM (proxy mode)](#litellm-proxy-mode) | Env | `OPENAI_API_BASE=http://localhost:9000/v1` | Live | [Open ↗](../integrations/litellm/) |
| 2 | [Aider](#aider) | Env | `OPENAI_API_BASE=http://localhost:9000/v1` | Spec | [Section ↓](#aider) |
| 3 | [Continue](#continue) | YAML `apiBase` | `apiBase: http://localhost:9000/v1` | Spec | [Section ↓](#continue) |
| 4 | [Cline / Roo Code](#cline--roo-code-byok) | UI | Custom OpenAI provider, base URL field | Spec | [Section ↓](#cline--roo-code-byok) |
| 5 | [OpenHands (BYOK)](#openhands-byok) | UI | LLM custom endpoint | Spec | [Section ↓](#openhands-byok) |
| 6 | [Goose](#goose) | Env | `OPENAI_HOST=http://localhost:9000` | Live | [Section ↓](#goose) |
| 7 | [Zed AI](#zed-ai) | TOML | `api_url = "http://localhost:9000/v1"` | Vendor-native | [Section ↓](#zed-ai) |
| 8 | [GitHub Copilot CLI (BYOK)](#github-copilot-cli-byok) | Env | `COPILOT_PROVIDER_BASE_URL=http://localhost:9000/v1` | Vendor-native | [Section ↓](#github-copilot-cli-byok) |
| 9 | [Tabnine Enterprise](#tabnine-enterprise) | Admin UI | BYO LLM endpoint | Spec | [Section ↓](#tabnine-enterprise) |
| 10 | [AnythingLLM](#anythingllm) | Admin UI | Custom OpenAI-compatible base URL | Spec | [Open ↗](anythingllm.md) |
| 11 | [LobeChat](#lobechat) | UI | Custom base URL | Live | [Open ↗](lobechat.md) |
| 12 | [Cody self-hosted Enterprise](#cody-self-hosted-enterprise) | Config | Sourcegraph relay endpoint | Spec | [Section ↓](#cody-self-hosted-enterprise) |
| 13 | [Augment (BYOK)](#augment-byok) | UI | LLM custom endpoint | Spec | [Section ↓](#augment-byok) |
| 14 | [Dify](#dify) | Plugin | Custom Model Provider plugin | Spec | [Section ↓](#dify) |
| 15 | [CrewAI Studio (via LiteLLM)](#crewai-studio-via-litellm) | Indirect | Use row 1 | Spec | [Section ↓](#crewai-studio-via-litellm) |

> Already running SpendGuard in production? Swap `http://localhost:9000/v1`
> for your SpendGuard egress proxy URL (Kubernetes Service URL, sidecar
> URL inside the pod, or hosted SpendGuard URL). The relative path is
> always `/v1`. See [Helm deployment](../deployment/helm/) for the
> production hostname pattern.

---

### LiteLLM (proxy mode)

[Per-tool template — see §3.1 below for the canonical structure each H3
 follows. Each per-tool section is roughly 30 lines.]

### Aider
…
### Continue
…
### Cline / Roo Code (BYOK)
…
### OpenHands (BYOK)
…
### Goose
…
### Zed AI
…
### GitHub Copilot CLI (BYOK)
…
### Tabnine Enterprise
…
### AnythingLLM
…
### LobeChat
…
### Cody self-hosted Enterprise
…
### Augment (BYOK)
…
### Dify
…
### CrewAI Studio (via LiteLLM)
…

---

## What next

- [Provision a real budget on Kubernetes](../deployment/helm/)
- [Open the SpendGuard dashboard](../operations/dashboard/)
- [Cover the rest of your stack with the egress proxy install (Pattern 3)](../install/)
- [Wire a framework SDK (Pattern 1)](../integrations/openai-agents/)
- [How SpendGuard compares to Cloudflare AI Gateway, Databricks Unity, Portkey](../posts/agent-spend-governance-gap/)
```

### 3.1 Per-tool H3 template

Every per-tool section follows this structure exactly. Variability is
in the content of the `### Set it` and `### Gotchas` blocks; the
heading sequence is fixed so the page reads consistently and so the
screenshot-regression baseline matches Slice over Slice.

```markdown
### <Tool name>

> [One-sentence statement of what this tool is and the context in which
>  a developer would be reading this section.]

**Maintainer docs:** [<upstream docs page>](<upstream URL>)

**Setting**

```<bash|yaml|toml>
<exact env var or config block, copy-paste runnable>
```

**Verify it works**

```bash
<one curl or one tool-native check that confirms the next call goes
 through SpendGuard — e.g. `aider --version --verbose` showing the
 OPENAI_API_BASE the tool resolved; or `curl -X POST
 http://localhost:9000/v1/chat/completions` returning a SpendGuard
 audit row in the dashboard.>
```

**Gotchas**

- [Any non-obvious behaviour: auth header forwarding, SSL trust,
  tools that re-write `/v1/chat/completions` to `/chat/completions`,
  tools that require setting `OPENAI_API_KEY` even when the SpendGuard
  proxy ignores it, tools whose UI requires the trailing `/v1` and
  tools whose UI rejects it. One-line bullets, no prose paragraphs.]

```

The `Verify it works` block is the one piece every reader will copy;
it must be self-sufficient. If a tool does not expose a way to
introspect its resolved base URL, the verification falls back to "make
one call from the tool, confirm the SpendGuard dashboard shows the
audit row." Each per-tool section's `Verify` block must complete in
under 10 seconds of wall time on a developer laptop.

### 3.2 Stub pages

`docs/site-v2/src/content/docs/docs/drop-in/anythingllm.md` (Slice 1):

```markdown
---
title: "AnythingLLM custom OpenAI-compatible base URL"
description: >-
  Drop SpendGuard into AnythingLLM by setting the LLM provider base URL
  to a running SpendGuard egress proxy. Recipe in progress; see the
  drop-in landing for the env var.
---

The full walkthrough lands with D33. In the meantime, the env var and
the verification step are listed on the
[drop-in landing page](./#anythingllm).
```

`docs/site-v2/src/content/docs/docs/drop-in/lobechat.md` (Slice 1)
mirrors the AnythingLLM stub with the tool name swapped and the anchor
adjusted to `#lobechat`. When D33 / D34 ship, these files are replaced
in their entirety; D03 owns the placeholders, not the long-form
recipes.

### 3.3 Sidebar wiring

`docs/site-v2/astro.config.mjs` — add a new sidebar group between
`Adapter integrations` (line 60 region in the current file) and
`Operations`:

```javascript
{
  label: 'Drop-in (Pattern 2)',
  items: [
    { label: 'Drop in 14 tools (overview)', slug: 'docs/drop-in/index' },
    { label: 'AnythingLLM recipe', slug: 'docs/drop-in/anythingllm' },
    { label: 'LobeChat recipe', slug: 'docs/drop-in/lobechat' },
  ],
},
```

No other section of `astro.config.mjs` changes.

---

## 4. Per-slice file plan

### 4.1 Slice 1 — Skeleton (`COV_03_base_url_landing_skeleton`)

**Adds:**

- `docs/site-v2/src/content/docs/docs/drop-in/index.md` — full page
  per §3, with all 14 (15 with CrewAI) per-tool H3 sections populated
  with the env var / config block, upstream docs link, and one-line
  gotcha bullets where applicable. Copy is functional but not yet
  polished (Slice 2 owns that).
- `docs/site-v2/src/content/docs/docs/drop-in/anythingllm.md` — stub
  per §3.2.
- `docs/site-v2/src/content/docs/docs/drop-in/lobechat.md` — stub
  per §3.2.
- `docs/internal/slices/COV_03_base_url_landing_skeleton.md` — slice doc per
  build plan §1.5.

**Modifies:**

- `docs/site-v2/astro.config.mjs` — adds the `Drop-in (Pattern 2)`
  sidebar group per §3.3.

**Does NOT touch:**

- `README.md` (Slice 2 owns the cross-link row).
- Any existing page outside `docs/site-v2/src/content/docs/docs/drop-in/`.
- Any spec / proto / migration file.

**Build / verification (per `tests.md`):**

- `cd docs/site-v2 && npm run build` exits 0.
- `docs/site-v2/dist/docs/drop-in/index.html` exists with content.
- Sidebar renders the new group in the built HTML.
- Per-tool H3 anchors all resolve (link checker passes).
- Upstream docs page link-check passes per §3 of `tests.md`.

### 4.2 Slice 2 — Copy polish + README sync + screenshot baseline (`COV_04_base_url_landing_copy_polish`)

**Modifies:**

- `docs/site-v2/src/content/docs/docs/drop-in/index.md` — copy
  pass on hero, the "How Pattern 2 works" callout, the per-tool
  gotchas. No structural change. Per `design.md` §3 voice:
  second person, present tense, active voice. The structural
  templates in §3.1 (heading sequence, block ordering) are not
  changed; only the prose is edited.
- `README.md` — adds one row to the `## 🔌 Adapter integrations`
  table at the bottom of the existing list:

  ```markdown
  | **Drop-in (14 tools)** | `docs/site-v2/src/content/docs/docs/drop-in/index.md` (Pattern 2: env-var redirect) | Every OpenAI-compatible base URL tool | [Drop-in landing](https://agenticspendguard.dev/docs/drop-in/) |
  ```

  The exact column shape mirrors the existing rows in that table; if
  the column count differs at slice-time, the Technical Writer adapts
  the row to match the live table format.

**Adds:**

- `docs/internal/slices/COV_04_base_url_landing_copy_polish.md` — slice doc.
- `docs/site-v2/.screenshots/drop-in-1280.png` — baseline at 1280px.
- `docs/site-v2/.screenshots/drop-in-375.png` — baseline at 375px.

(Screenshot file path uses `.screenshots/` to match any existing
visual-regression convention in the repo; if none exists, this
directory is added under `docs/site-v2/.gitignore` for the source
files and the comparison happens in CI only. See `tests.md` §4.)

**Build / verification:**

- `cd docs/site-v2 && npm run build` exits 0.
- Markdownlint (if configured) or built-in MDX parser produces zero
  errors / warnings on the changed file.
- The README cross-link row resolves to a 200 on the live URL once
  the page is deployed (CI link-check runs against the built `dist/`
  in pre-deploy; deploy gating is `acceptance.md` §3.4).

### 4.3 Slice 3 — Optional `<DropInPicker />` MDX picker (`COV_05_base_url_landing_picker_mdx`)

**Adds:**

- `docs/site-v2/src/components/DropInPicker.astro` — Astro component:
  a `<select>` element listing the 14 tools, with a small inline
  script that updates the URL hash to the matching anchor on
  selection and smooth-scrolls to the H3.
- `docs/internal/slices/COV_05_base_url_landing_picker_mdx.md` — slice doc.

**Renames:**

- `docs/site-v2/src/content/docs/docs/drop-in/index.md` →
  `docs/site-v2/src/content/docs/docs/drop-in/index.mdx`. Adds at the
  top, right after the frontmatter:

  ```mdx
  import DropInPicker from '../../../../components/DropInPicker.astro';

  <DropInPicker />
  ```

**Does NOT touch:**

- `astro.config.mjs` (Starlight handles `.md` and `.mdx` identically;
  no config change required).
- `package.json` (MDX support is already in the Starlight integration).

**Build / verification:**

- `cd docs/site-v2 && npm run build` exits 0 with the `.mdx` file in
  place.
- With JavaScript disabled in the browser, the page renders the
  fallback (no picker visible; H3 anchor links continue to work).
- With JavaScript enabled, selecting a tool in the picker updates the
  URL hash and smooth-scrolls; using browser back/forward navigates
  through the hash history correctly.

Slice 3 is skipped automatically per `design.md` §5 if Slice 1 / 2
exceed the 1500-LOC budget or the R1 reviewer flags it as not worth
the JS surface.

---

## 5. Reviewer-facing reading order

A reviewer picking up the slice diff reads in this order:

1. `design.md` §1 (scope) and §3 (key decisions).
2. This file's §3 (page skeleton) — confirms shape.
3. The actual page diff in `docs/site-v2/src/content/docs/docs/drop-in/index.md`.
4. `tests.md` §3 (the upstream-citation link-check is the highest-risk gate).
5. `acceptance.md` §2 / §3 — the ship gates.

The Technical Writer's PR description should call out the upstream
docs URLs they cited in §3 per-tool sections, so the reviewer's
link-check can be re-run by hand if CI is unavailable.
