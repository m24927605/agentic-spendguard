# D03 — `OPENAI_BASE_URL` Drop-in Landing Page — `review-standards.md`

> Status: Per-slice gate. Lands with the spec set; defines what every
> R1-R5 reviewer is checking.
> Sibling docs: `design.md` (scope), `implementation.md` (file plan),
> `tests.md` (gate mechanics), `acceptance.md` (ship gate).
> Audience: `superpowers:code-reviewer` skill (canonical reviewer per
> build plan §1.2); Staff+ panel for R5 arbitration.

---

## 1. Overview and rationale

This deliverable is documentation, so the review surface is unlike a
code deliverable's. The two most failure-prone surfaces are:

1. **Citation accuracy.** A wrong env var name on a public marketing
   page is a credibility failure. Half the rows on the page citelet
   the upstream maintainer's docs; if a row drifts from the upstream
   reality between spec time and ship time, the page is worse than
   not shipping.
2. **Scope drift between the page and the strategy memo.** The
   strategy memo (`framework-coverage-2026-06.md`) is the source of
   truth for which tools belong in Pattern 2. If the page adds a
   tool the memo does not list, or omits a tool the memo lists, the
   coverage story falls apart.

Both are mechanical checks (`tests.md` §3.3 and §3.4). The review
loop's job is to verify the mechanical checks ran and to catch the
non-mechanical surfaces: voice, mobile readability, and
copy-pasteable verification blocks.

Per the build plan §1.2, `superpowers:code-reviewer` is the canonical
reviewer for every slice. This document is the review-standards input
that skill consumes.

---

## 2. Slice acceptance bar

A slice **passes** and the next slice begins only when all hard gates
are green. Soft gates produce GitHub issues but do not block.

### 2.1 Hard gates (every slice)

| # | Gate | Source of truth |
|---|---|---|
| H1 | Astro build green: `cd docs/site-v2 && npm run build` exits 0 | `tests.md` §2.1 |
| H2 | All in-scope route files exist in `dist/` after build | `tests.md` §2.2 |
| H3 | Sidebar renders the new group | `tests.md` §2.3 |
| H4 | Internal anchors all resolve | `tests.md` §3.1 |
| H5 | External link-check green | `tests.md` §3.2 |
| H6 | Upstream-citation check green (every cited env var / config key matches upstream docs) | `tests.md` §3.3 |
| H7 | Pattern-2-parity check green (page set ⊆ memo set, page set ⊇ memo set modulo CrewAI indirection) | `tests.md` §3.4 |
| H8 | All `acceptance.md` §2 functional criteria green | `acceptance.md` §2 |
| H9 | Review log committed in the slice's commit set | `review-standards.md` §3.7 |

### 2.2 Slice-2-only hard gates

| # | Gate | Source of truth |
|---|---|---|
| H10 | Visual-regression diff matches the intentional change set described in the slice doc | `tests.md` §4 |
| H11 | README cross-link row exists and resolves | `acceptance.md` §3.3 |
| H12 | Voice / present-tense / active-voice pass on inspected pages | `acceptance.md` §Q1 |

### 2.3 Slice-3-only hard gates

| # | Gate | Source of truth |
|---|---|---|
| H13 | Page renders identically with JS disabled (progressive enhancement preserved) | `acceptance.md` §F3 |
| H14 | URL hash updates on selection; back/forward navigation works | `implementation.md` §4.3 |
| H15 | `.mdx` rename does not change the page URL slug | `implementation.md` §4.3 |

### 2.4 Soft gates (issue, don't block)

- **S1.** A per-tool gotcha block could be expanded with one more
  bullet to cover an edge case → reviewer files `enhancement` issue
  with `d03-followup` label, slice merges.
- **S2.** A cited upstream docs page has redirected once (301 →
  200) → reviewer files `docs-link-drift` issue, slice merges with
  the redirected URL.
- **S3.** Visual-regression diff is < 1% but not zero in a section
  the slice did not intend to touch → reviewer files
  `unintended-style-drift` issue, slice merges if the change is
  cosmetic.

---

## 3. The R1-R5 review loop

Per the build plan §1.1. Every slice goes through up to five rounds
with `superpowers:code-reviewer`. The skill produces a structured
report of findings; the implementer fixes them; the skill re-runs.

### 3.1 R1 input package

The reviewer is given:

- This `review-standards.md` document (the rubric).
- The slice doc under `docs/internal/slices/COV_NN_*.md`.
- The slice's diff (`git diff main..<slice-branch>`).
- `acceptance.md` for cross-reference.
- The link to the strategy memo
  (`docs/strategy/framework-coverage-2026-06.md`).

### 3.2 R1 review checklist

The reviewer walks the slice diff in this order. Each item is
either ✓ (pass), ✗ (fail, becomes a finding), or N/A (does not
apply to this slice).

| # | Check | Slice 1 | Slice 2 | Slice 3 |
|---|---|:---:|:---:|:---:|
| 1 | All hard gates §2.1 — mechanical checks — run green in CI | ✓ | ✓ | ✓ |
| 2 | Per-tool sections contain the full template (Maintainer docs, Setting, Verify, Gotchas) | ✓ | ✓ | N/A |
| 3 | Matrix row count matches the per-tool section count | ✓ | ✓ | N/A |
| 4 | `Verified` column values match `design.md` §3.2 distribution | ✓ | ✓ | N/A |
| 5 | Citation snapshots exist under `citations/` for manual-review rows | ✓ | ✓ | N/A |
| 6 | Voice / present-tense / active-voice on inspected paragraphs | (soft) | ✓ | (soft) |
| 7 | Mobile readability at 375 px on inspected sections | ✓ | ✓ | ✓ |
| 8 | "What next" footer trio of links resolves | ✓ | ✓ | ✓ |
| 9 | No emoji in body text (matrix + footer status icons allowed) | ✓ | ✓ | ✓ |
| 10 | README diff scope-correct (Slice 2 only) | N/A | ✓ | N/A |
| 11 | Stub pages for D33 / D34 are present and non-empty (Slice 1) | ✓ | (already there) | (already there) |
| 12 | Astro component is progressive enhancement (Slice 3) | N/A | N/A | ✓ |
| 13 | URL hash + back / forward semantics (Slice 3) | N/A | N/A | ✓ |
| 14 | One concept per section (`acceptance.md` §Q4) | ✓ | ✓ | ✓ |
| 15 | Working code in `Verify it works` blocks | ✓ | ✓ | N/A |

### 3.3 Finding severity

Per the build plan's review standard. P0 / P1 findings block; P2 /
P3 may merge with linked GitHub issues.

- **P0.** A hard gate from §2.1 / §2.2 / §2.3 is not green.
  Citation-accuracy failures (H6) are P0 by default; a wrong env
  var name on the page is a public credibility failure.
- **P1.** A row in the matrix is mis-categorised in the `Verified`
  column (e.g. labelled `Live` without a CI run on record); a
  per-tool section's `Verify` block does not produce the expected
  result on a developer laptop; an anti-acceptance criterion from
  `acceptance.md` §5 is violated.
- **P2.** A voice / present-tense / active-voice issue on a single
  paragraph; a cited upstream docs page has redirected; a gotcha
  block omits one known caveat.
- **P3.** Cosmetic (table column alignment, code-block language
  mismatch, hyphenation, oxford-comma drift).

### 3.4 Stopping rule (`feedback_codex_iteration_pattern`)

Per the build plan's review cadence: maximum 5 rounds per slice. If
R5 still has unresolved P0 / P1 findings, the slice is escalated to
the Staff+ panel (§4 below). Soft gates (P2 / P3) do not block.

R1 is automatic — the reviewer runs on every slice. R2-R5 run only
if R1 produced P0 / P1 findings. The implementer fixes findings
between rounds; the reviewer re-runs against the updated diff.

### 3.5 Reviewer assignment

| Slice | Primary reviewer | Tie-breaker (if R5 escalates) |
|---|---|---|
| 1 | `superpowers:code-reviewer` (canonical) | Software Architect |
| 2 | `superpowers:code-reviewer` | Software Architect |
| 3 | `superpowers:code-reviewer` | Frontend Developer + Software Architect |

The Staff+ panel composition for R5 follows the build plan §1.3.

### 3.6 Review log location

Each slice's review log lives at
`docs/specs/coverage/D03_base_url_landing/review-logs/COV_NN_<short>.md`.
The directory is created by Slice 1 as part of the slice CI scaffolding
even if its first commit is the only artifact in it.

### 3.7 Review log content

Each review log contains:

| Section | Content |
|---|---|
| §1. Round summary | One row per round: round number, findings P0/P1/P2/P3, time-to-fix |
| §2. P0/P1 findings | Per finding: which §3.2 check failed, the diff that resolved it |
| §3. Manual smoke notes | Spot-check of two per-tool sections at desktop and mobile |
| §4. Soft gates | List of soft-gate issues filed |
| §5. Verdict | `pass` / `merge-with-residuals` / `block-pending-arbitration` |

---

## 4. R5 Staff+ panel arbitration

Triggered when R5 still has P0 / P1 findings unresolved. The panel
follows `docs/internal/review-standards/staff-panel-arbitration-process.md` §2
(referenced in the build plan).

### 4.1 Panel composition

| Role | `subagent_type` | Rationale |
|---|---|---|
| Architecture | Software Architect | Page structure / IA decisions |
| Content systems | Technical Writer | Voice / accuracy / reader experience |
| AI / framework ecosystem | AI Engineer | Tool-specific configuration nuances |
| Pragmatic implementation | Senior Developer | Buildable / ship-able pragmatic judgment |
| Security / external surface | Security Engineer | (cite an upstream URL on a public page: any tracking / fingerprinting concern?) |

Summarizer defaults to Software Architect. The summarizer reconciles
the five panelists' memos into one ruling: `merge-with-residuals`,
`block`, or `rework`.

### 4.2 Inputs to the panel

- The slice diff.
- The R1-R5 review logs (§3.6).
- The unresolved P0 / P1 findings, each with the implementer's
  response.
- The `tests.md` CI output for the slice.
- This `review-standards.md` document.

### 4.3 Output

A ≤ 1-page panel memo committed to
`docs/specs/coverage/D03_base_url_landing/review-logs/COV_NN_panel.md`.
The memo states the ruling, the rationale, and the residuals that
must be tracked as GitHub issues.

---

## 5. Cross-deliverable review hooks

D03's review loop **does not** re-evaluate D02 / D33 / D34's
acceptance. If D03 Slice 1 ships stub pages that D33 / D34 later
replace, the D03 review log records the stub state at slice time;
later D33 / D34 reviews validate their own page content against
their own spec sets.

If D33 or D34 ships a recipe that the D03 page's matrix row 10 /
row 11 contradicts (e.g. D33 ships a recipe that uses a different
env var than D03 row 10 lists), the contradiction is caught at the
D33 / D34 review stage and the resolution is a D33 / D34 PR that
also updates D03's row in the same slice. This convention preserves
D03's atomic acceptance gate without artificially blocking D33 /
D34.

---

## 6. Anti-review-scope

The reviewer is **not** asked to:

- Run any service / sidecar / ledger code. D03 is doc-only.
- Re-derive the strategy memo's Pattern 2 set. The set is locked at
  spec time; later additions go through a memo update, not a D03
  re-review.
- Verify marketing claims about competitor products (the page
  footer cites Cloudflare / Portkey / Databricks comparisons; those
  citations live on a separate marketing page reviewed under its
  own spec set).
- Audit the SpendGuard egress proxy's behaviour. The page documents
  how to point at the proxy; the proxy's correctness is owned by
  the egress-proxy deliverable.

The reviewer **is** asked to:

- Confirm every cited env var or config key matches the upstream
  maintainer's documentation at slice time (§2.1 H6).
- Spot-check that two per-tool sections' `Verify it works` blocks
  run as documented on a developer laptop (§3.2 row 15).
- Confirm the page's matrix and the strategy memo's Pattern 2 set
  remain in lock-step (§2.1 H7).
- Confirm the page's voice / readability / one-concept-per-section
  invariants hold (§3.2 rows 6, 7, 9, 14).
