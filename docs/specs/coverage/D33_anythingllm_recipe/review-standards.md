# D33 — AnythingLLM Custom Base URL Recipe — `review-standards.md`

> Status: Per-slice gate. Lands with the spec set; defines what every
> R1-R5 reviewer is checking.
> Sibling docs: `design.md`, `implementation.md`, `tests.md`, `acceptance.md`.
> Audience: `superpowers:code-reviewer` skill (canonical reviewer per
> build plan §1.2); Staff+ panel for R5 arbitration.

---

## 1. Overview and rationale

D33 is a hybrid deliverable: a single documentation page plus a Docker
smoke that exercises a real AnythingLLM → SpendGuard → OpenAI
round-trip. The two most failure-prone surfaces are:

1. **Citation drift.** AnythingLLM's UI labels (`Base URL`, `API Key`,
   `Chat Model Name`, `Token context window`) are the primary thing
   the reader transcribes into the AnythingLLM panel. If the recipe's
   field names disagree with the live UI in `mintplexlabs/anythingllm:1.8.4`,
   the recipe is worse than not shipping.
2. **Smoke fragility.** The smoke depends on AnythingLLM's
   `/api/v1/system/update-env` payload schema and on the demo compose
   profile not leaking services into other modes. A regression in
   either is a demo-quality-gate violation per the project memory's
   `feedback_demo_quality_gate`.

Both surfaces have mechanical gates in `tests.md`. The review loop's
job is to verify the mechanical checks ran and to catch the
non-mechanical surfaces: voice, copy-pasteable verification, and the
Desktop / Cloud disclaimers.

Per the build plan §1.2, `superpowers:code-reviewer` is the canonical
reviewer for every slice.

---

## 2. Slice acceptance bar

A slice **passes** and the next slice begins only when all hard gates
are green. Soft gates produce GitHub issues but do not block.

### 2.1 Hard gates (every slice)

| # | Gate | Source of truth |
|---|---|---|
| H1 | Astro build green; `dist/docs/drop-in/anythingllm/index.html` exists | `tests.md` §2.1 |
| H2 | `bash -n deploy/demo/anythingllm_smoke.sh` exits 0 | `tests.md` §2.2 |
| H3 | `docker compose --profile anythingllm_real config` exits 0 | `tests.md` §2.3 |
| H4 | External link-check green on the recipe page | `tests.md` §3.1 |
| H5 | Citation PDF committed under `citations/` | `tests.md` §3.2 |
| H6 | D03 row 10 link target check green | `tests.md` §3.3 |
| H7 | `make demo-up DEMO_MODE=anythingllm_real` exits 0 with `OK: reserve+commit verified` (skipped if `OPENAI_API_KEY` not set, see §2.4 below) | `tests.md` §4.1 |
| H8 | Verify SQL re-run standalone exits 0 | `tests.md` §4.2 |
| H9 | D03 stub no longer ships (no `Recipe in progress` substring) | `tests.md` §5.2 |
| H10 | All `acceptance.md` §2 functional criteria green | `acceptance.md` §2 |
| H11 | Review log committed in the slice's commit set | `review-standards.md` §3.7 |

### 2.2 Slice-1-only hard gates

| # | Gate | Source of truth |
|---|---|---|
| H12 | D03 row 10 `Verified` promoted `Spec → Live` | `tests.md` §5.1 |
| H13 | `make help` lists `DEMO_MODE=anythingllm_real` | `acceptance.md` §S4 |
| H14 | AnythingLLM image pinned to `mintplexlabs/anythingllm:1.8.4`, not floating | `acceptance.md` §F8 |
| H15 | `anythingllm` / `anythingllm-smoke` services gated by `anythingllm_real` profile | `acceptance.md` §F9 |

### 2.3 Slice-2-only hard gates (if Slice 2 ships)

| # | Gate | Source of truth |
|---|---|---|
| H16 | Embedded screenshots match the live UI of the pinned image | R1 reviewer manual check |
| H17 | Visual-regression diff matches the documented change set | `tests.md` §4 of D03 (re-used for D33's page) |

### 2.4 Soft gates (issue, don't block)

- **S1.** Smoke skipped because `OPENAI_API_KEY` secret unavailable in
  CI (fork PR) → reviewer files `d33-smoke-deferred` issue, slice
  merges; the merge-to-main run with the secret is the final gate.
- **S2.** A cited upstream docs URL has redirected 301 → 200 → reviewer
  files `docs-link-drift` issue, slice merges with the redirected URL.
- **S3.** Smoke wall-time > 2 minutes → reviewer files
  `d33-smoke-perf` issue, slice merges. Hard ceiling is 3 minutes per
  `tests.md` §8.
- **S4.** A gotcha block could be expanded with one more bullet →
  reviewer files `enhancement` with `d33-followup` label, slice merges.

---

## 3. The R1-R5 review loop

Per the build plan §1.1. Every slice goes through up to five rounds
with `superpowers:code-reviewer`.

### 3.1 R1 input package

The reviewer is given:

- This `review-standards.md` document.
- The slice doc under `docs/slices/COV_NN_*.md`.
- The slice diff (`git diff main..<slice-branch>`).
- `acceptance.md` for cross-reference.
- The D03 spec set (sibling cross-link target).
- The pinned image's UI as seen at slice time (screenshot or live run
  of `docker run --rm -p 3001:3001 mintplexlabs/anythingllm:1.8.4`).

### 3.2 R1 review checklist

The reviewer walks the slice diff in this order. Each item is ✓
(pass), ✗ (fail, becomes a finding), or N/A.

| # | Check | Slice 1 | Slice 2 |
|---|---|:---:|:---:|
| 1 | All hard gates §2.1 — mechanical checks — run green in CI | ✓ | ✓ |
| 2 | Page contains: hero, Prerequisites, Steps 1-4, Verify, Deployment notes, Gotchas, What next, Maintainer docs link (in order) | ✓ | ✓ |
| 3 | Field names in Step 3 table match the upstream PDF snapshot verbatim | ✓ | (already there) |
| 4 | "Verify end-to-end" Step is one bash invocation runnable from a clean clone | ✓ | ✓ |
| 5 | Desktop / Cloud disclaimers present in Deployment notes | ✓ | (already there) |
| 6 | Gotchas covers trailing `/v1`, Generic-vs-`OpenAI` confusion, API key behaviour, streaming | ✓ | (already there) |
| 7 | Voice / present-tense / active-voice on inspected paragraphs | (soft) | ✓ |
| 8 | Mobile readability at 375 px on the configuration table | ✓ | ✓ |
| 9 | No emoji in body text | ✓ | ✓ |
| 10 | Demo-mode `anythingllm_real` listed in `make help` | ✓ | N/A |
| 11 | D03 row 10 `Verified` reads `Live` | ✓ | (already there) |
| 12 | Compose profile isolation: no `anythingllm` service in other modes | ✓ | N/A |
| 13 | Image tag pinned (not `:latest`) | ✓ | (already there) |
| 14 | Verify SQL uses a time window, not a single-row-most-recent assertion | ✓ | N/A |
| 15 | Smoke prints `OK: reserve+commit verified` on success | ✓ | N/A |
| 16 | Screenshots match the live UI of the pinned image (Slice 2 only) | N/A | ✓ |
| 17 | One concept per section | ✓ | ✓ |

### 3.3 Finding severity

- **P0.** A hard gate from §2.1 / §2.2 / §2.3 is not green. Citation
  drift (H5 / row 3 of §3.2) is P0 by default — a wrong field name is
  a public credibility failure.
- **P1.** Smoke fails on a non-secret-related reason; D03 row 10 not
  promoted; an `acceptance.md` §5 anti-acceptance criterion violated;
  compose profile leaks service into another mode.
- **P2.** Voice / present-tense issue on a single paragraph; cited
  upstream docs page redirected once; a gotcha block omits one known
  caveat (e.g. proxy timeout for slow models).
- **P3.** Cosmetic (table column alignment, code-block language
  mismatch, hyphenation, screenshot crop margin).

### 3.4 Stopping rule

Maximum 5 rounds per slice. If R5 still has unresolved P0 / P1
findings, the slice escalates to the Staff+ panel (§4). Soft gates
(P2 / P3) do not block.

### 3.5 Reviewer assignment

| Slice | Primary reviewer | Tie-breaker (if R5 escalates) |
|---|---|---|
| 1 | `superpowers:code-reviewer` (canonical) | Technical Writer |
| 2 | `superpowers:code-reviewer` | Technical Writer + Senior Developer |

### 3.6 Review log location

Each slice's review log lives at
`docs/specs/coverage/D33_anythingllm_recipe/review-logs/COV_NN_<short>.md`.
The directory is created by Slice 1.

### 3.7 Review log content

| Section | Content |
|---|---|
| §1. Round summary | One row per round: round number, findings P0/P1/P2/P3, time-to-fix |
| §2. P0/P1 findings | Per finding: which §3.2 check failed, the diff that resolved it |
| §3. Manual smoke notes | Reviewer's own `make demo-up DEMO_MODE=anythingllm_real` run output and wall-time |
| §4. Citation cross-check | Confirmation that page field names match the committed PDF snapshot |
| §5. Soft gates | List of soft-gate issues filed |
| §6. Verdict | `pass` / `merge-with-residuals` / `block-pending-arbitration` |

---

## 4. R5 Staff+ panel arbitration

Triggered when R5 still has P0 / P1 findings unresolved.

### 4.1 Panel composition

| Role | `subagent_type` | Rationale |
|---|---|---|
| Architecture | Software Architect | Page structure / compose profile design |
| Content systems | Technical Writer | Voice / accuracy / reader experience |
| AI / framework ecosystem | AI Engineer | AnythingLLM behaviour, streaming nuances |
| Backend systems | Backend Architect | Smoke + verify SQL durability |
| Pragmatic implementation | Senior Developer | Ship-able pragmatic judgment |

Summarizer defaults to Technical Writer (this is a documentation
deliverable; the writer is best placed to reconcile reader-facing
trade-offs). Backend Architect summarizes only if the panel ruling
hinges on smoke / SQL correctness rather than page content.

### 4.2 Inputs to the panel

- The slice diff.
- The R1-R5 review logs.
- The unresolved P0 / P1 findings with implementer responses.
- The `tests.md` CI output for the slice.
- The committed citation PDF.
- This `review-standards.md` document.

### 4.3 Output

A ≤ 1-page panel memo committed to
`docs/specs/coverage/D33_anythingllm_recipe/review-logs/COV_NN_panel.md`.
The memo states the ruling, the rationale, and the residuals tracked
as GitHub issues.

---

## 5. Cross-deliverable review hooks

D33's review loop **does not** re-evaluate D03's acceptance. D33
ships changes to D03's row 10 `Verified` column and to the
`anythingllm.md` content (replacing D03's SLICE 1 stub); these are
the only D03 touches D33 makes. If a D33 slice modifies any other
D03 surface, the modification is out-of-scope and must be reverted
before merge.

If a later D03 slice (post-D33 merge) edits row 10 in a way that
contradicts the D33 recipe (e.g. swaps the cited base URL pattern),
the contradiction is caught at the D03 review stage and resolved in
that same D03 slice — not by re-opening D33.

If AnythingLLM ships a version that breaks the smoke (e.g. renames
`/api/v1/system/update-env` payload keys), the fix is a new D33
follow-up slice that bumps the pinned image tag AND updates the page +
smoke in lock-step. The R1 reviewer's manual UI check (§3.1) is the
gate that catches upstream UI rename.

---

## 6. Anti-review-scope

The reviewer is **not** asked to:

- Run AnythingLLM Desktop or Cloud. Desktop is GUI-only; Cloud is
  hosted. The recipe documents both but the smoke covers Docker only.
- Audit AnythingLLM's source code for security or quality. The
  pinned image is a third-party dependency; SpendGuard's security
  surface is the egress proxy, not AnythingLLM.
- Re-derive the smoke's audit assertions. The verify SQL pattern is
  inherited from `verify_step_litellm_real.sql` and was reviewed at
  that slice's R1; D33 reuses the pattern as-is.
- Test AnythingLLM's embedding / voice / agent-skill surfaces. These
  are explicitly anti-scope per `design.md` §1.2.

The reviewer **is** asked to:

- Confirm every cited field name matches the AnythingLLM UI captured
  in `citations/anythingllm-custom-openai-base-url.pdf` (§2.1 H5,
  §3.2 row 3).
- Run `make demo-up DEMO_MODE=anythingllm_real` once and record the
  wall-time in the review log (§3.7 §3).
- Confirm the compose profile isolation (§3.2 row 12) by running
  `docker compose --profile <other-mode> config` and confirming the
  `anythingllm` service does not appear.
- Confirm D03 row 10 reads `Live` post-merge (§3.2 row 11).
- Confirm voice / readability / one-concept-per-section invariants
  (§3.2 rows 7, 8, 17).
