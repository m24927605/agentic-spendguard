# D03 — `OPENAI_BASE_URL` Drop-in Landing Page — `acceptance.md`

> Status: Scope-lock document. Once accepted, the criteria here are
> immutable for the duration of the slice plan unless the project
> owner explicitly re-opens scope.
> Sibling docs: `design.md` (what we are building), `implementation.md`
> (file-level plan), `tests.md` (gate mechanics), `review-standards.md`
> (per-slice gate).
> Audience: Project owner (sign-off), Technical Writer (target),
> R1-R5 reviewer (verification), `superpowers:code-reviewer` skill.

---

## 1. Acceptance philosophy

This document defines the **scope-locked** answer to "what does
*D03 is done* look like?" Per-slice progress lives in
`review-standards.md` §3. This document is **the bar**: a slice that
passes `review-standards.md` still does not ship "D03" until every
criterion below holds against the merged diff.

Criteria reference `design.md` decision IDs (D-1 through D-6) and
sibling-deliverable contracts (D02, D33, D34) by name so they are
cross-checkable without re-reading.

A deliverable is "done" per the build plan §7 definition:

- All slices merged into main.
- All `acceptance.md` gates green.
- A row exists in `README.md` `## 🔌 Adapter integrations` (acceptance
  §3.2).
- A user-facing page exists under `docs.agenticspendguard.dev/docs/`
  (acceptance §3.1).
- A demo-mode entry exists in `Makefile` if the deliverable is
  demoable — **N/A for D03** (D03 is doc-only; no Makefile entry).

---

## 2. Functional acceptance

Every bullet is a capability the merged deliverable MUST demonstrate
against the built and rendered docs site.

- **F1 (D-1 in `design.md` §1.1, in-scope tools).** The matrix on
  the landing page contains exactly the 14 distinct tools listed in
  `design.md` §1.1 plus the CrewAI Studio indirection row (row 15).
  No additional tools added; no in-scope tools omitted. Verified by
  `tests.md` §3.4 (pattern-2-parity check) and by inspection of the
  matrix in the rendered HTML.

- **F2 (D-2 in `design.md` §1.2, sibling-deliverable coordination).**
  The matrix's row 10 (AnythingLLM) "Recipe" link points at
  `/docs/drop-in/anythingllm/`. The matrix's row 11 (LobeChat)
  "Recipe" link points at `/docs/drop-in/lobechat/`. Both targets
  resolve to a non-empty page on the rendered site — either a stub
  shipped by D03 Slice 1 or the real recipe shipped by D33 / D34.
  Verified by `tests.md` §2.2 (route existence) and §3.1 (internal
  anchors / external links).

- **F3 (D-3 in `design.md` §3.1, Markdown vs MDX).** The page
  renders correctly at Slice 1 and Slice 2 as `.md` Markdown with
  zero JavaScript hydration. If Slice 3 ships, the page also renders
  correctly with JavaScript disabled — the `<DropInPicker />`
  component is progressive enhancement. Verified by `tests.md` §2.1
  (build) at every slice; verified by `tests.md` §7 (manual smoke)
  at R1 with JS toggled off in the browser.

- **F4 (D-4 in `design.md` §3.2, install-attested column).** The
  matrix carries a `Verified` column with one of the three values
  per row defined in `design.md` §3.2 — `Live`, `Spec`, or
  `Vendor-native`. At Slice 2 ship time, the distribution is as
  documented in `design.md` §3.2. Verified by inspection of the
  matrix in the review log.

- **F5 (D-6 in `design.md` §3.4, exact upstream setting strings).**
  Every per-tool detail section's `Setting` block contains the
  literal env var name or config key documented by the maintainer
  on the cited upstream docs page. Verified by `tests.md` §3.3
  (upstream-citation check). Rows that cannot be machine-verified
  (UI-step rows) have a PDF snapshot of the upstream docs page
  committed under
  `docs/specs/coverage/D03_base_url_landing/citations/` and
  reviewer-cross-checked at R1.

- **F6 (D-5 in `design.md` §3.3, drift control with README).** The
  README's existing `## 🧰 What works today` table is unchanged by
  D03. The README's `## 🔌 Adapter integrations` table gains one row
  pointing at the D03 landing URL per `implementation.md` §4.2.
  Verified by inspection of the README diff at slice merge time.

- **F7 (D-7 cross-deliverable test isolation, `design.md` §3.6).**
  D03's slice CI workflow runs green when D02 / D33 / D34 have not
  yet shipped. Verified by running the workflow on a branch where
  D02 / D33 / D34 are not yet present; the build succeeds; the
  stub pages for D33 / D34 cover the link-check requirement;
  no D02-owned file is referenced.

- **F8 (page anatomy from `design.md` §2.2).** The rendered page
  contains, in order: hero, "How Pattern 2 works" callout,
  "Start the proxy locally" snippet, "Find your tool" matrix,
  14-15 per-tool H3 sections, "What next" trio of links, footer
  with Pattern 1 / Pattern 3 / comparison-page links. Verified by
  manual inspection at R1.

---

## 3. Surface acceptance

Surfaces the deliverable must expose externally.

- **S1 (page URL, F1+F2).** Page is reachable at
  `https://agenticspendguard.dev/docs/drop-in/` (trailing slash) on
  the production deploy. Verified by HTTP GET after deploy; the
  pre-deploy `dist/` build is the slice-CI gate (`tests.md` §2.2);
  the post-deploy live URL is the project owner's sign-off gate.

- **S2 (sidebar entry, F2).** The Starlight sidebar on every page
  of the docs site shows a `Drop-in (Pattern 2)` group with three
  entries: overview, AnythingLLM recipe, LobeChat recipe.
  Verified by inspection on any rendered page.

- **S3 (README cross-link row, F6).** `README.md` shows one new row
  in the `## 🔌 Adapter integrations` table pointing at the
  production D03 URL. Verified by inspection of the README on
  GitHub after slice 2 merge.

- **S4 (CI workflow, F7).** `.github/workflows/docs-drop-in.yml`
  exists, runs on PRs that touch the in-scope paths
  (`tests.md` §7), and is green at slice merge.

---

## 4. Quality acceptance

Standards the page must meet that are not covered by mechanical
tests.

- **Q1 (voice).** Second person, present tense, active voice
  throughout. Verified by reviewer scan; a slice with mixed person
  / tense / voice in the body fails R1 with a `voice` finding.

- **Q2 (no emoji in body).** Body text contains no emoji. The
  matrix and the comparison footer are allowed emoji per
  `design.md` §2.3 for status icons; no other emoji allowed.
  Verified by `grep` for non-ASCII codepoints in the
  `## ` / `### ` headings and the body paragraphs (excluding
  the matrix and the footer).

- **Q3 (mobile readability at 375 px).** The matrix's `Tool`
  column remains legible at 375 px; per-tool code blocks scroll
  horizontally rather than overflow. Verified by `tests.md` §4
  (visual regression at 375 px) and §5 (manual smoke).

- **Q4 (one concept per section).** Each per-tool H3 covers one
  tool only; installation / configuration / verification are
  separated into the template's blocks (`Maintainer docs`,
  `Setting`, `Verify it works`, `Gotchas`). No section mixes
  two tools' settings. Verified by inspection.

- **Q5 (working code in `Verify it works`).** Every per-tool
  section's `Verify it works` block is copy-pasteable and runs in
  under 10 seconds against a local SpendGuard egress proxy on a
  developer laptop. Verified by spot-check on at least two rows
  during R1 manual smoke (`tests.md` §5).

- **Q6 (citation accuracy).** Per F5, every cited string is exact;
  any paraphrasing of an upstream env var name is a hard fail.

- **Q7 (no broken claim).** No claim is made on the page that is
  not either (a) verified by a SpendGuard-run end-to-end test
  (only rows labelled `Live` in the matrix qualify), or (b)
  cited from the upstream maintainer's docs. Verified by inspection.

---

## 5. Anti-acceptance

The deliverable is **not** shipped — even if every gate above is
green — if any of the following holds:

- **A1.** A per-tool section's `Setting` block contains an env var
  name that no longer matches the upstream maintainer's docs.
  (Equivalent to F5 failing; called out separately because this
  is the single most public-facing failure mode.)

- **A2.** The page lists a tool that is not in the strategy memo's
  Pattern 2 table. (Equivalent to F1 failing.)

- **A3.** The README's `## 🧰 What works today` table was modified
  by D03. The two tables MUST stay independent
  (`design.md` §3.3).

- **A4.** A row in the matrix is marked `Live` in the `Verified`
  column without a corresponding green run in the
  SpendGuard repo's CI in the last 30 days at slice merge time.
  `Live` rows must be backed by a real run; if no run is on
  record, the row drops to `Spec`.

- **A5.** The page is published live before the citation snapshots
  for the "manual review" rows
  (`tests.md` §3.3 rows 4, 5, 9, 10, 11, 12, 13, 14) exist under
  `docs/specs/coverage/D03_base_url_landing/citations/`. The
  snapshots provide the audit trail for rows we cannot
  machine-verify.

---

## 6. Slice gating

Every slice must pass `review-standards.md` §3 (the R1-R5 gate)
before the next slice begins. A slice that fails R5 escalates to
the Staff+ panel per the build plan §1.3.

The deliverable is **shipped** when:

- Slice 1 (skeleton) is merged.
- Slice 2 (copy polish + README sync + screenshot baseline) is merged.
- Slice 3 (optional MDX picker) is either merged or formally descoped
  per `design.md` §5.
- All `acceptance.md` §2-§5 gates are green against the post-merge
  `main` branch.
- The post-deploy live URL returns 200 with the expected content.

---

## 7. Memory write-back

Per the build plan §8: when D03 is shipped, a memory entry under
`~/.claude/projects/-Users-michael-chen-products-agentic-spendguard/memory/`
named `project_coverage_D03_shipped.md` is created with:

- Merge commit SHA of Slice 2 (and Slice 3 if shipped).
- R1-R5 round count per slice.
- Arbitration yes / no per slice.
- The live URL.
- A one-line summary of `Verified` column distribution at ship time
  (e.g. "Live=3, Spec=10, Vendor-native=2").

This memory entry is the durable record that D03 has shipped and the
"Drop-in (Pattern 2)" surface is live.
