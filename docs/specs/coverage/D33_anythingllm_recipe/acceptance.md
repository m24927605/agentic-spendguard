# D33 — AnythingLLM Custom Base URL Recipe — `acceptance.md`

> Status: Scope-lock document. Once accepted, the criteria here are
> immutable for the duration of the slice plan unless the project owner
> explicitly re-opens scope.
> Sibling docs: `design.md`, `implementation.md`, `tests.md`, `review-standards.md`.
> Audience: Project owner (sign-off), Technical Writer (target),
> R1-R5 reviewer (verification), `superpowers:code-reviewer` skill.

---

## 1. Acceptance philosophy

This document defines the scope-locked answer to "what does D33 done
look like?" Per-slice progress lives in `review-standards.md` §3. This
document is **the bar**: a slice that passes `review-standards.md`
still does not ship "D33" until every criterion below holds against the
merged diff.

Per the build plan §7, a deliverable is done when all slices are
merged, all `acceptance.md` gates are green, a row exists in the
README adapter integrations table, a user-facing page exists, and a
demo-mode entry exists in `Makefile`. For D33 the demo-mode entry is
`anythingllm_real`.

---

## 2. Functional acceptance

Every bullet is a capability the merged deliverable MUST demonstrate.

- **F1 (page replaces D03 stub).** `docs/site-v2/src/content/docs/docs/drop-in/anythingllm.md`
  exists, is non-empty, and does NOT contain the substring
  `Recipe in progress`. Verified by `tests.md` §5.2.

- **F2 (page content per `design.md` §1).** The rendered page contains,
  in order: hero paragraph, Prerequisites, Steps 1-4, Verify end-to-end
  block, Deployment notes (Docker / Desktop / Cloud), Gotchas, What next,
  Maintainer docs link. Verified by reviewer at R1.

- **F3 (citation accuracy, `design.md` §1.1 + `tests.md` §3.2).** The
  field names cited on the page — **Base URL**, **API Key**, **Chat Model
  Name** — appear verbatim on the AnythingLLM upstream docs page captured
  under `citations/anythingllm-custom-openai-base-url.pdf`. Verified by
  R1 reviewer cross-check against the snapshot.

- **F4 (smoke green, `design.md` §2.2 + `tests.md` §4).** `make demo-up
  DEMO_MODE=anythingllm_real` exits 0 with stdout containing
  `[anythingllm-smoke] OK: reserve+commit verified` on a clean clone
  with a valid `OPENAI_API_KEY`.

- **F5 (audit chain assertion, `tests.md` §4.2).** The verify SQL
  asserts ≥ 1 `reserve` row AND ≥ 1 `commit_estimated` row in
  `ledger_transactions` for the demo tenant within the smoke's 10-minute
  window. Verified by SQL standalone re-run.

- **F6 (D03 row 10 promoted to `Live`, `design.md` §2.1 + `tests.md` §5.1).**
  D03 landing page row 10 (AnythingLLM) `Verified` column reads `Live`
  in the merged diff. Promotion is conditional on F4 passing in the
  same slice; if F4 fails, F6 reverts to `Spec`.

- **F7 (Desktop / Cloud disclaimers, `design.md` §1.2).** The
  Deployment notes section explicitly states that Desktop users cannot
  run the smoke and Cloud users cannot point at `localhost`. Linking
  Cloud users to the Helm deployment page satisfies F7.

- **F8 (compose pinning, `design.md` §2.4).** The AnythingLLM image in
  `deploy/demo/compose.yaml` is pinned to `mintplexlabs/anythingllm:1.8.4`,
  not `:latest` or a floating tag. Verified by grep of the compose file.

- **F9 (image isolation).** The `anythingllm` and `anythingllm-smoke`
  services are gated behind the `anythingllm_real` compose profile and
  are NOT pulled / built on any other demo mode. Verified by:
  `cd deploy/demo && docker compose config | grep -c anythingllm`
  returns 0 when no `--profile` is set.

---

## 3. Surface acceptance

Surfaces the deliverable must expose externally.

- **S1 (page URL).** Page is reachable at
  `https://agenticspendguard.dev/docs/drop-in/anythingllm/` (trailing
  slash) on the production deploy. Verified by HTTP GET post-deploy.

- **S2 (sidebar entry).** The Starlight sidebar shows
  `Drop-in (Pattern 2) → AnythingLLM recipe` (registered by D03 SLICE 1;
  D33 does not modify `astro.config.mjs`). Verified by inspection on
  any rendered page.

- **S3 (README adapter integrations row).** If the README's
  `## 🔌 Adapter integrations` table needs a new row for the
  AnythingLLM recipe specifically (separate from the D03 drop-in row),
  it is added in D33 Slice 1. Otherwise the D03 row covers D33 via the
  sub-link from the drop-in landing. The decision is captured in the
  R1 review log.

- **S4 (demo mode in Makefile help).** `make help` lists
  `DEMO_MODE=anythingllm_real` in the demo-mode enumeration of
  `deploy/demo/Makefile`'s help block. Verified by inspection.

- **S5 (CI workflow).** `.github/workflows/docs-drop-in.yml` runs the
  D33 jobs from `tests.md` §8 on every PR that touches the in-scope
  paths; the jobs are green at slice merge time.

---

## 4. Quality acceptance

Standards the page + smoke must meet beyond mechanical tests.

- **Q1 (voice).** Second person, present tense, active voice
  throughout the page. Verified by reviewer scan.

- **Q2 (no emoji in body).** Body text contains no emoji. The matrix
  on D03 may carry status icons per D03's standard; D33 has no matrix
  and so no emoji at all.

- **Q3 (mobile readability at 375 px).** The four-field configuration
  table remains legible at 375 px. Verified at R1 manual smoke.

- **Q4 (one concept per section).** Each H2 covers one concept:
  prerequisites, step, verification, deployment notes, gotchas. No
  section mixes two concepts. Verified by inspection.

- **Q5 (copy-pasteable smoke).** The Step 4 "Verify end-to-end" block
  is one bash invocation that runs against a clean clone of the
  SpendGuard repo. No assumed terminal state beyond `OPENAI_API_KEY`.
  Verified by R1 reviewer running it.

- **Q6 (citation accuracy).** Per F3, every cited upstream field name
  is exact; paraphrasing is a hard fail.

- **Q7 (no false claim).** The page does NOT claim that streaming,
  embeddings, voice, or agent skills are end-to-end verified. The
  smoke covers chat only; the page is explicit about that scope.

- **Q8 (smoke determinism).** The verify SQL uses a 10-minute window
  (not a single-row most-recent assertion) so a re-run with stale rows
  in the ledger does not false-positive. The window is documented in
  the SQL comment header.

---

## 5. Anti-acceptance

The deliverable is **not** shipped — even if every gate above is green —
if any of the following holds:

- **A1.** The page cites a field name (e.g. "Base Path", "OpenAI Key")
  that does not match the literal upstream field name. Equivalent to F3
  failing; called out separately because misnamed fields are the most
  public-facing failure mode for a recipe page.

- **A2.** The smoke claims `reserve+commit verified` while the verify
  SQL did not actually run or returned zero rows. Smoke output and SQL
  result must be coupled (smoke runs SQL inline, exit code propagates).

- **A3.** The page is published live before the citation PDF exists
  under `citations/`. The snapshot is the audit trail for the
  "manual review" L4 gate; no snapshot, no ship.

- **A4.** The D03 row 10 `Verified` column reads `Live` while the D33
  smoke last ran red in CI. The promotion is conditional on a green run
  in the slice's CI; a red run reverts the column to `Spec` before merge.

- **A5.** The `anythingllm` / `anythingllm-smoke` services boot on any
  demo mode other than `anythingllm_real`. F9 must hold; a leaked
  service inflates demo-up time and cost for unrelated modes.

- **A6.** The AnythingLLM image tag is `:latest` or any non-pinned
  reference. F8 must hold; a floating tag breaks the demo-regression
  reproducibility budget the rest of the repo relies on.

---

## 6. Slice gating

Every slice must pass `review-standards.md` §3 (the R1-R5 gate) before
the next slice begins. A slice that fails R5 escalates to the Staff+
panel per the build plan §1.3.

The deliverable is **shipped** when:

- Slice 1 (recipe page + smoke) is merged.
- Slice 2 (screenshots / UX polish) is either merged or formally
  descoped per `design.md` §2.1 (descope rule: Slice 1 ships under 400
  LOC and R1 reviewer flags Slice 2 as low marginal value).
- All `acceptance.md` §2-§5 gates are green against the post-merge
  `main` branch.
- The post-deploy live URL returns 200 with the expected content.
- A clean clone runs `make demo-up DEMO_MODE=anythingllm_real` green
  with `OPENAI_API_KEY` set.

---

## 7. Memory write-back

Per the build plan §8: when D33 is shipped, a memory entry under
`~/.claude/projects/-Users-michael-chen-products-agentic-spendguard/memory/`
named `project_coverage_D33_shipped.md` is created with:

- Merge commit SHA of Slice 1 (and Slice 2 if shipped).
- R1-R5 round count per slice.
- Arbitration yes / no per slice.
- The live URL.
- The pinned AnythingLLM image tag at ship time.
- The smoke wall-time on the slice CI green run.

This memory entry is the durable record that D33 has shipped, that
AnythingLLM is on the `Live`-verified row of the D03 matrix, and that
the demo-mode `anythingllm_real` is available to operators.
