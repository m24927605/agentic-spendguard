# D03 — `OPENAI_BASE_URL` Drop-in Landing Page — `tests.md`

> Status: Doc-first spec; defines what must run green before any slice
> merges.
> Sibling docs: `design.md`, `implementation.md`, `acceptance.md`,
> `review-standards.md`.
> Audience: R1-R5 reviewer; CI maintainer; Technical Writer implementer
> when authoring the slice.

---

## 1. Test surface

D03 is a documentation deliverable, so the test pyramid does not look
like a code deliverable's. There is no unit-test layer. The pyramid is:

| Layer | What it covers | Runs in |
|---|---|---|
| **L1 — Build** | Astro build succeeds; routes exist; no broken templates | Slice CI + local |
| **L2 — Parse** | Markdown / MDX parses; frontmatter valid; sidebar config valid | Slice CI + local |
| **L3 — Link** | Internal and external links resolve | Slice CI |
| **L4 — Citation** | Each cited tool's env var / config key appears verbatim on the upstream docs page | Slice CI |
| **L5 — Cross-check** | Page tools ⊆ `framework-coverage-2026-06.md` Pattern 2 set | Slice CI |
| **L6 — Visual regression** | Screenshot diff against baseline; intentional changes only | Slice 2 + CI |
| **L7 — Manual smoke** | Reviewer opens the rendered page on desktop and mobile widths | R1-R5 reviewer |

Layers L1-L5 are hard gates on every slice. L6 is hard from Slice 2
onward. L7 is captured in the review log per `review-standards.md` §3.

---

## 2. L1 + L2 — Build and parse

### 2.1 Astro build green

```bash
cd docs/site-v2 && npm run build
```

Exits 0. The Starlight build is the single source of truth for "the
page renders." A failed Starlight build blocks slice merge; a passing
Starlight build does NOT imply the page reads correctly (L4 / L6 / L7
cover that).

### 2.2 Routes exist in the built `dist/`

After `npm run build`:

```bash
test -s docs/site-v2/dist/docs/drop-in/index.html
test -s docs/site-v2/dist/docs/drop-in/anythingllm/index.html
test -s docs/site-v2/dist/docs/drop-in/lobechat/index.html
```

All three checks return 0. A missing file means either the page slug
is wrong in the frontmatter / `astro.config.mjs` or the file was not
saved into the content directory; both fail the slice.

### 2.3 Sidebar group registered

The built HTML contains the new sidebar group label. A simple grep
suffices:

```bash
grep -r "Drop-in (Pattern 2)" docs/site-v2/dist/docs/drop-in/index.html
```

Returns at least one match. The check is intentionally loose; a
stricter check would over-couple this test to Starlight's HTML
structure across upgrades.

### 2.4 Frontmatter valid

Each new `.md` / `.mdx` file has a valid `title` (string) and
`description` (string ≥ 60 chars, ≤ 320 chars to fit the description
metadata convention used elsewhere in `src/content/docs/docs/`).
Verified by inspection in the review log; no programmatic check is
required because Astro's content schema rejects invalid frontmatter
at build time (L1 catches it transitively).

---

## 3. L3 + L4 — Link and citation

### 3.1 Internal anchor link-check

Every `#anchor` reference on the page must resolve to an H2 or H3
heading on the same page. The 14 (or 15, with CrewAI) per-tool
anchors in the matrix in §3 of `implementation.md` are the primary
surface. The Technical Writer runs:

```bash
# A repo-local script; if the repo does not already have one,
# Slice 1 adds a minimal Python script under
# scripts/check-internal-anchors.py that scans the built HTML for
# every <a href="#..."> and confirms a matching id attribute exists.
python scripts/check-internal-anchors.py docs/site-v2/dist/docs/drop-in/index.html
```

Exits 0 with zero missing anchors. The script is shared with the
broader Starlight content surface; if other pages have pre-existing
broken anchors, the script must be scoped to a path filter on its
first introduction so the D03 slice is not blocked by pre-existing
drift.

### 3.2 External link-check (cited tool docs)

Every per-tool section cites the maintainer's upstream docs page in
the `**Maintainer docs:**` line (per `implementation.md` §3.1
template). Slice CI fetches each cited URL with a 10-second timeout
and confirms a 200 response. URLs that return 3xx are followed up to
two hops; any 4xx / 5xx / timeout fails the slice.

```bash
# Reuses the existing link-check tooling if present in the repo
# (e.g. lychee, markdown-link-check). If none exists, Slice 1 vendors
# lychee via a one-off CI step; no new dependency in package.json.
lychee docs/site-v2/src/content/docs/docs/drop-in/index.md \
       docs/site-v2/src/content/docs/docs/drop-in/anythingllm.md \
       docs/site-v2/src/content/docs/docs/drop-in/lobechat.md
```

Exits 0 with zero broken links. Flake budget is two retries with
exponential backoff before failing; this is the standard link-check
convention used in the rest of the repo's CI.

### 3.3 L4 — Upstream citation verbatim check

This is the **highest-risk gate** for D03 (`design.md` §3.4). For
each per-tool section, the cited env var name or config key must
appear verbatim in the rendered HTML of the upstream docs page. A
Python script (added by Slice 1 under
`scripts/check-upstream-citations.py`) does the following per row:

1. Reads the per-tool section's `**Setting**` code block.
2. Extracts the env var name (regex `[A-Z_][A-Z0-9_]+(?==)`) or
   YAML / TOML key (regex `^[a-zA-Z_][a-zA-Z0-9_]*(?=:)` / `(?= =)`).
3. Fetches the cited upstream docs URL.
4. Greps the rendered HTML for the literal string.
5. Records pass / fail per row.

| Row | Upstream URL (citation target) | Literal string |
|---|---|---|
| 1 | https://docs.litellm.ai/docs/proxy/config_settings | `OPENAI_API_BASE` |
| 2 | https://aider.chat/docs/llms/openai-compat.html | `OPENAI_API_BASE` |
| 3 | https://docs.continue.dev/customization/models | `apiBase` |
| 4 | https://docs.cline.bot/getting-started/byok | (UI step text — manual review) |
| 5 | https://docs.all-hands.dev/usage/llms/custom-llm-configs | (UI step text — manual review) |
| 6 | https://block.github.io/goose/docs/getting-started/installation | `OPENAI_HOST` |
| 7 | https://zed.dev/docs/ai/configuration | `api_url` |
| 8 | https://docs.github.com/en/copilot/github-copilot-cli/using-byok | `COPILOT_PROVIDER_BASE_URL` |
| 9 | https://docs.tabnine.com/main/software-configuration/connect-custom-llm | (admin UI step — manual review) |
| 10 | https://docs.anythingllm.com/llm-configuration/custom-openai-base-url | (admin UI step — manual review) |
| 11 | https://lobehub.com/docs/self-hosting/usage/byok | (UI step — manual review) |
| 12 | https://sourcegraph.com/docs/cody/clients/install-vscode (Enterprise self-hosted relay docs) | (config — manual review) |
| 13 | https://docs.augmentcode.com/setup/byok | (UI step — manual review) |
| 14 | https://docs.dify.ai/plugins/model-provider | (plugin manifest — manual review) |
| 15 | (redirects to row 1; no separate citation) | — |

URLs marked "manual review" do not lend themselves to a one-string
verbatim check because the upstream docs surface the setting through
a screenshot or a UI walkthrough, not an env var literal. For these
rows, the Technical Writer captures the upstream docs page as a
PDF snapshot under `docs/specs/coverage/D03_base_url_landing/citations/`
and the reviewer compares it against the per-tool section by hand
during R1. The slice cannot merge without the snapshots in the
review log.

If an upstream docs page has changed between spec time and slice
time and the citation string no longer matches, the slice fails. The
Technical Writer must either (a) update the per-tool section to
reflect the new upstream setting, then re-run the check, or (b)
escalate to the R5 panel if the upstream change is breaking enough
that the row should be removed from the matrix entirely.

### 3.4 L5 — Cross-check against the strategy memo

Slice CI runs:

```bash
python scripts/check-pattern2-parity.py \
    --memo docs/strategy/framework-coverage-2026-06.md \
    --page docs/site-v2/src/content/docs/docs/drop-in/index.md
```

The script parses the Pattern 2 table in the strategy memo (the table
under heading `### Pattern 2 — Proxy redirect via OpenAI-compatible
base URL`, columns "Tool" + "Setting") and the matrix in the
landing page. Pass iff:

- Every tool in the memo's Pattern 2 table appears as a row on the
  landing page.
- Every row on the landing page (except row 15, CrewAI Studio,
  which is an explicit indirection) appears in the memo's table.

If the strategy memo is updated to add or remove a tool, the
mismatch is surfaced at CI time and the implementer either updates
the page (preferred) or opens a discussion to update the memo before
re-running. Drift between memo and page is a hard fail.

---

## 4. L6 — Visual regression

Slice 2 establishes the baseline; subsequent slice runs compare
against it. Two viewport widths are captured:

```bash
# Run inside a headless Chromium via Playwright (vendored in CI;
# no new package.json dependency for the page itself).
npx playwright screenshot \
    --viewport-size 1280,4000 \
    http://localhost:4321/docs/drop-in/ \
    docs/site-v2/.screenshots/drop-in-1280.png

npx playwright screenshot \
    --viewport-size 375,8000 \
    http://localhost:4321/docs/drop-in/ \
    docs/site-v2/.screenshots/drop-in-375.png
```

The full-page height is intentionally exaggerated so the entire
matrix + per-tool sections are captured (Playwright clamps to the
actual page height). The CI compares the new screenshot with the
baseline using a pixel-diff threshold of 1% — anything above triggers
a manual review. The review log §3 of `review-standards.md` records
the diff verdict; an intentional change is fine, an unexplained one
blocks merge.

The baseline lives in the repo; the comparison output (diff images)
lives in CI artifacts only and is not committed.

If a slice intentionally rewrites a per-tool gotcha block (Slice 2 is
expected to do this), the diff is expected to be > 1% in that block's
region; the reviewer confirms the diff matches the intentional change
described in the slice doc.

---

## 5. L7 — Manual smoke

Captured in the R1 review log per `review-standards.md` §3. Includes:

- Desktop visual scan at 1280 px and 1920 px viewport widths.
- Mobile visual scan at 375 px.
- Two random per-tool sections clicked through to confirm the
  `Verify it works` block can be copy-pasted and runs without
  modification against a local SpendGuard egress proxy.
- The matrix's "Recipe" column tested on three rows: at least one
  link to a sibling page (D33 / D34 stub or final), at least one
  in-page anchor, at least one external link (row 1 to the LiteLLM
  recipe page, which already exists).

### 5.1 Markdownlint / Vale

If the repo has a configured markdown linter (none required at the
time of this spec), the slice runs it on the changed files and
captures the output in the review log. If neither is configured, the
reviewer scans for repo-wide style consistency by inspection.

---

## 6. Demo regression — none

D03 does not modify any service, sidecar, ledger, proxy, or contract.
There is no demo regression surface for this deliverable. The slice
CI does not run `make demo-up`; this is intentional and saves ~3
minutes per slice run.

If a reviewer suspects the page's `Start the proxy locally`
instructions are stale, they re-run `make demo-up DEMO_MODE=proxy`
once at R1 and record the result in the review log. After R1, the
demo-up step is not re-run unless the underlying egress proxy
deliverable changes.

---

## 7. CI workflow

A single new workflow file `.github/workflows/docs-drop-in.yml` is
added by Slice 1 with the following jobs (parallel where independent):

| Job | Steps | Hard gate |
|---|---|---|
| `build` | Checkout; install Node 22; `cd docs/site-v2 && npm ci && npm run build`; assert `dist/docs/drop-in/index.html` non-empty | Yes |
| `internal-anchors` | Depends on `build`; `python scripts/check-internal-anchors.py docs/site-v2/dist/docs/drop-in/index.html` | Yes |
| `external-links` | Depends on `build`; `lychee` on the three drop-in pages | Yes |
| `upstream-citations` | `python scripts/check-upstream-citations.py docs/site-v2/src/content/docs/docs/drop-in/index.md` | Yes |
| `pattern2-parity` | `python scripts/check-pattern2-parity.py` | Yes |
| `visual-regression` | Slice 2 + later; depends on `build`; Playwright screenshot + pixel-diff vs baseline | Yes (Slice 2+) |

Concurrency: the workflow runs in parallel with the rest of the
repo's CI (Rust build / Helm validate / etc.); D03 has no shared
state with those workflows.

Trigger: `pull_request` against `main`, paths-filter on:

- `docs/site-v2/**`
- `docs/strategy/framework-coverage-2026-06.md`
- `docs/specs/coverage/D03_base_url_landing/**`
- `scripts/check-internal-anchors.py`
- `scripts/check-upstream-citations.py`
- `scripts/check-pattern2-parity.py`
- `.github/workflows/docs-drop-in.yml`

A change outside these paths does NOT trigger the D03 workflow.
