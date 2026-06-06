# D34 — LobeChat Custom Base URL Recipe — `acceptance.md`

> Status: Scope-lock document. Once accepted, the criteria here are
> immutable for the duration of the slice plan unless the project owner
> explicitly re-opens scope.
> Sibling docs: `design.md`, `implementation.md`, `tests.md`, `review-standards.md`.
> Audience: Project owner (sign-off), Technical Writer (target),
> R1-R5 reviewer (verification), `superpowers:code-reviewer` skill.

---

## 1. Acceptance philosophy

This document defines the scope-locked answer to "what does D34 done
look like?" Per-slice progress lives in `review-standards.md` §3. This
document is **the bar**: a slice that passes `review-standards.md`
still does not ship "D34" until every criterion below holds against the
merged diff.

Per the build plan §7, a deliverable is done when all slices are
merged, all `acceptance.md` gates are green, a row exists in the
README adapter integrations table, a user-facing page exists, and a
demo-mode entry exists in `Makefile`. For D34 the demo-mode entry is
`lobechat_real`.

---

## 2. Functional acceptance

Every bullet is a capability the merged deliverable MUST demonstrate.

- **F1 (page replaces D03 stub).** `docs/site-v2/src/content/docs/docs/drop-in/lobechat.md`
  exists, is non-empty, and does NOT contain the substring
  `Recipe in progress`. Verified by `tests.md` §5.2.

- **F2 (page content per `design.md` §1).** The rendered page contains,
  in order: hero paragraph, Prerequisites, Steps 1-5 (env var, confirm,
  send chat, verify end-to-end, compose snippet), Deployment notes
  (Docker / Vercel / Cloud / Client mode), Gotchas, What next,
  Maintainer docs link. Verified by reviewer at R1.

- **F3 (citation accuracy, `design.md` §1.1 + `tests.md` §3.2).** The
  identifiers cited on the page — env var **`OPENAI_PROXY_URL`** and
  client-mode field **API Proxy Address** — appear verbatim on the
  LobeChat upstream docs page captured under
  `citations/lobechat-environment-variables.pdf`. Verified by R1
  reviewer cross-check against the snapshot.

- **F4 (smoke green, `design.md` §2.2 + `tests.md` §4).** `make demo-up
  DEMO_MODE=lobechat_real` exits 0 with stdout containing
  `[lobechat-smoke] OK: reserve+commit verified` on a clean clone
  with a valid `OPENAI_API_KEY`.

- **F5 (audit chain assertion, `tests.md` §4.2).** The verify SQL
  asserts ≥ 1 `reserve` row AND ≥ 1 `commit_estimated` row in
  `ledger_transactions` for the demo tenant within the smoke's
  10-minute window. Verified by SQL standalone re-run.

- **F6 (D03 row 11 reads `Live`, `design.md` §2.1 + `tests.md` §5.1).**
  D03 landing page row 11 (LobeChat) `Verified` column reads `Live`
  in the merged diff. Closes the conditional set by D03 `design.md`
  §3.2.

- **F7 (Vercel / Cloud / Client-mode disclaimers, `design.md` §1.2).**
  The Deployment notes section explicitly states that Vercel inherits
  the env var, Cloud cannot point at `localhost`, and Client mode
  bypasses `OPENAI_PROXY_URL` (use the per-session UI override
  instead). Linking Cloud users to the Helm deployment page satisfies
  F7's Cloud requirement.

- **F8 (compose pinning, `design.md` §2.4).** The LobeChat image in
  `deploy/demo/compose.yaml` is pinned to `lobehub/lobe-chat:1.40.0`,
  not `:latest` or a floating tag. Verified by grep of the compose
  file.

- **F9 (image isolation).** The `lobechat` and `lobechat-smoke`
  services are gated behind the `lobechat_real` compose profile and
  are NOT pulled / built on any other demo mode. Verified by:
  `cd deploy/demo && docker compose config | grep -c lobechat`
  returns 0 when no `--profile` is set.

- **F10 (server mode locked, `design.md` §2.5).** The smoke runs
  LobeChat in `NEXT_PUBLIC_SERVICE_MODE=server`. Client-mode is
  documented as out-of-scope for the smoke. Verified by inspection
  of the compose env block.

- **F11 (env-var-only configuration, `design.md` §2.2).** The smoke
  does NOT call any LobeChat admin / update-env / runtime-config API.
  The single source of provider configuration is the `OPENAI_PROXY_URL`
  env var set at container boot. Verified by absence of any
  `/api/config` or `/api/admin` curl in `lobechat_smoke.sh`.

---

## 3. Surface acceptance

Surfaces the deliverable must expose externally.

- **S1 (page URL).** Page is reachable at
  `https://agenticspendguard.dev/docs/drop-in/lobechat/` (trailing
  slash) on the production deploy. Verified by HTTP GET post-deploy.

- **S2 (sidebar entry).** The Starlight sidebar shows
  `Drop-in (Pattern 2) → LobeChat recipe` (registered by D03 SLICE 1;
  D34 does not modify `astro.config.mjs`). Verified by inspection on
  any rendered page.

- **S3 (README adapter integrations row).** If the README's
  `## 🔌 Adapter integrations` table needs a new row for the LobeChat
  recipe specifically (separate from the D03 drop-in row), it is added
  in D34 Slice 1. Otherwise the D03 row covers D34 via the sub-link
  from the drop-in landing. The decision is captured in the R1 review
  log.

- **S4 (demo mode in Makefile help).** `make help` lists
  `DEMO_MODE=lobechat_real` in the demo-mode enumeration of
  `deploy/demo/Makefile`'s help block. Verified by inspection.

- **S5 (CI workflow).** `.github/workflows/docs-drop-in.yml` runs the
  D34 jobs from `tests.md` §8 on every PR that touches the in-scope
  paths; the jobs are green at slice merge time.

- **S6 (D33 / D34 coexistence).** The `anythingllm_real` and
  `lobechat_real` compose profiles can be enabled simultaneously
  without port / volume collision (`tests.md` §5.3). Verified by CI
  job `d33-d34-coexistence`.

---

## 4. Quality acceptance

Standards the page + smoke must meet beyond mechanical tests.

- **Q1 (voice).** Second person, present tense, active voice
  throughout the page. Verified by reviewer scan.

- **Q2 (no emoji in body).** Body text contains no emoji.

- **Q3 (mobile readability at 375 px).** The env-var snippet in Step 1
  and the compose snippet in Step 5 remain readable at 375 px. Code
  blocks may horizontally scroll on mobile; the surrounding prose must
  remain non-truncated. Verified at R1 manual smoke.

- **Q4 (one concept per section).** Each H2 covers one concept:
  prerequisites, step, verification, deployment notes, gotchas. No
  section mixes two concepts. Verified by inspection.

- **Q5 (copy-pasteable smoke).** The Step 4 "Verify end-to-end" block
  is one bash invocation that runs against a clean clone of the
  SpendGuard repo. No assumed terminal state beyond `OPENAI_API_KEY`.
  Verified by R1 reviewer running it.

- **Q6 (citation accuracy).** Per F3, every cited upstream identifier
  is exact; paraphrasing is a hard fail. `OPENAI_PROXY_URL` (with
  underscores, all caps) and `API Proxy Address` (with the exact
  capitalisation LobeChat uses) are the two hardest cases.

- **Q7 (no false claim).** The page does NOT claim that LobeChat
  plugins, agents, TTS, image generation, or vision are end-to-end
  verified. The smoke covers the OpenAI chat path only; the page is
  explicit about that scope.

- **Q8 (smoke determinism).** The verify SQL uses a 10-minute window
  (not a single-row most-recent assertion) so a re-run with stale rows
  in the ledger does not false-positive. The window is documented in
  the SQL comment header.

- **Q9 (client-mode honest disclaimer).** The page explicitly states
  that `OPENAI_PROXY_URL` is ignored in client mode. A reader running
  LobeChat in client mode without reading this would have a silent
  failure; the disclaimer is the only thing preventing that.

---

## 5. Anti-acceptance

The deliverable is **not** shipped — even if every gate above is green —
if any of the following holds:

- **A1.** The page cites a different env var name (e.g. `OPENAI_BASE_URL`,
  `OPENAI_API_BASE`) than the upstream `OPENAI_PROXY_URL`. Equivalent
  to F3 failing; called out separately because the env var name is
  the single decisive identifier on the entire page.

- **A2.** The smoke claims `reserve+commit verified` while the verify
  SQL did not actually run or returned zero rows. Smoke output and SQL
  result must be coupled (smoke runs SQL inline, exit code propagates).

- **A3.** The page is published live before the citation PDF exists
  under `citations/`. The snapshot is the audit trail for the
  "manual review" L4 gate; no snapshot, no ship.

- **A4.** The D03 row 11 `Verified` column reads `Live` while the D34
  smoke last ran red in CI. The promotion is conditional on a green run
  in the slice's CI; a red run reverts the column to `Spec` before
  merge.

- **A5.** The `lobechat` / `lobechat-smoke` services boot on any demo
  mode other than `lobechat_real`. F9 must hold; a leaked service
  inflates demo-up time and cost for unrelated modes.

- **A6.** The LobeChat image tag is `:latest` or any non-pinned
  reference. F8 must hold; a floating tag breaks the demo-regression
  reproducibility budget the rest of the repo relies on.

- **A7.** The page implies that `OPENAI_PROXY_URL` is honoured by the
  client-mode browser. Per F11 / Q9, the env var is read by the
  LobeChat server only; misleading the reader here is a worse-than-
  not-shipping failure mode for users on client deployments.

- **A8.** The smoke includes a call to any LobeChat admin API for
  runtime configuration (the recipe and the smoke must demonstrate
  that the env var alone is sufficient — calling an admin API would
  contradict F11 and Q9 and make the page misleading).

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
- A clean clone runs `make demo-up DEMO_MODE=lobechat_real` green with
  `OPENAI_API_KEY` set.

---

## 7. Memory write-back

Per the build plan §8: when D34 is shipped, a memory entry under
`~/.claude/projects/-Users-michael-chen-products-agentic-spendguard/memory/`
named `project_coverage_D34_shipped.md` is created with:

- Merge commit SHA of Slice 1 (and Slice 2 if shipped).
- R1-R5 round count per slice.
- Arbitration yes / no per slice.
- The live URL.
- The pinned LobeChat image tag at ship time.
- The smoke wall-time on the slice CI green run.

This memory entry is the durable record that D34 has shipped, that
LobeChat is on the `Live`-verified row of the D03 matrix, and that the
demo-mode `lobechat_real` is available to operators.
