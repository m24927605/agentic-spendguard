# D34 — LobeChat Custom Base URL Recipe — `review-standards.md`

> Status: Per-slice gate. Lands with the spec set; defines what every
> R1-R5 reviewer is checking.
> Sibling docs: `design.md`, `implementation.md`, `tests.md`, `acceptance.md`.
> Audience: `superpowers:code-reviewer` skill (canonical reviewer per
> build plan §1.2); Staff+ panel for R5 arbitration.

---

## 1. Overview and rationale

D34 is a hybrid deliverable: a single documentation page plus a Docker
smoke that exercises a real LobeChat → SpendGuard → OpenAI round-trip.
The two most failure-prone surfaces are:

1. **Citation drift.** LobeChat's `OPENAI_PROXY_URL` env var name is
   the single most important identifier on the page. A typo
   (`OPENAI_BASE_URL`, `OPENAI_API_BASE`, lowercase) silently breaks
   every reader's setup and is indistinguishable from a working recipe
   until they send a chat and see the request go to `api.openai.com`
   directly. The client-mode field label (`API Proxy Address`) is the
   second-most-critical citation.
2. **Server-vs-client mode confusion.** `OPENAI_PROXY_URL` is honoured
   by the LobeChat **server** route only. Client-mode browsers ignore
   it and POST directly to OpenAI. A page that does not loudly
   disclaim this puts client-mode users in a silent-failure state.

Both surfaces have mechanical gates in `tests.md`. The review loop's
job is to verify the mechanical checks ran and to catch the
non-mechanical surfaces: voice, copy-pasteable verification, the
server-vs-client disclaimer, and the Vercel / Cloud notes.

Per the build plan §1.2, `superpowers:code-reviewer` is the canonical
reviewer for every slice.

---

## 2. Slice acceptance bar

A slice **passes** and the next slice begins only when all hard gates
are green. Soft gates produce GitHub issues but do not block.

### 2.1 Hard gates (every slice)

| # | Gate | Source of truth |
|---|---|---|
| H1 | Astro build green; `dist/docs/drop-in/lobechat/index.html` exists | `tests.md` §2.1 |
| H2 | `bash -n deploy/demo/lobechat_smoke.sh` exits 0 | `tests.md` §2.2 |
| H3 | `docker compose --profile lobechat_real config` exits 0 | `tests.md` §2.3 |
| H4 | External link-check green on the recipe page | `tests.md` §3.1 |
| H5 | Citation PDF committed under `citations/` | `tests.md` §3.2 |
| H6 | D03 row 11 link target check green | `tests.md` §3.3 |
| H7 | `make demo-up DEMO_MODE=lobechat_real` exits 0 with `OK: reserve+commit verified` (skipped if `OPENAI_API_KEY` not set, see §2.4 below) | `tests.md` §4.1 |
| H8 | Verify SQL re-run standalone exits 0 | `tests.md` §4.2 |
| H9 | D03 stub no longer ships (no `Recipe in progress` substring) | `tests.md` §5.2 |
| H10 | D33 + D34 compose profiles coexist without collision | `tests.md` §5.3 |
| H11 | All `acceptance.md` §2 functional criteria green | `acceptance.md` §2 |
| H12 | Review log committed in the slice's commit set | `review-standards.md` §3.7 |

### 2.2 Slice-1-only hard gates

| # | Gate | Source of truth |
|---|---|---|
| H13 | D03 row 11 `Verified` reads `Live` (closing the conditional set by D03 §3.2) | `tests.md` §5.1 |
| H14 | `make help` lists `DEMO_MODE=lobechat_real` | `acceptance.md` §S4 |
| H15 | LobeChat image pinned to `lobehub/lobe-chat:1.40.0`, not floating | `acceptance.md` §F8 |
| H16 | `lobechat` / `lobechat-smoke` services gated by `lobechat_real` profile | `acceptance.md` §F9 |
| H17 | Smoke contains NO call to any LobeChat admin / runtime-config API | `acceptance.md` §F11 |
| H18 | Page contains explicit client-mode disclaimer | `acceptance.md` §Q9 |

### 2.3 Slice-2-only hard gates (if Slice 2 ships)

| # | Gate | Source of truth |
|---|---|---|
| H19 | Embedded screenshot matches the live UI of the pinned image | R1 reviewer manual check |
| H20 | Visual-regression diff matches the documented change set | `tests.md` §4 of D03 (re-used for D34's page) |

### 2.4 Soft gates (issue, don't block)

- **S1.** Smoke skipped because `OPENAI_API_KEY` secret unavailable in
  CI (fork PR) → reviewer files `d34-smoke-deferred` issue, slice
  merges; the merge-to-main run with the secret is the final gate.
- **S2.** A cited upstream docs URL has redirected 301 → 200 → reviewer
  files `docs-link-drift` issue, slice merges with the redirected URL.
- **S3.** Smoke wall-time > 2 minutes → reviewer files
  `d34-smoke-perf` issue, slice merges. Hard ceiling is 3 minutes per
  `tests.md` §8.
- **S4.** A gotcha block could be expanded with one more bullet
  (e.g. ACCESS_CODE rotation, `OPENAI_MODEL_LIST` syntax) → reviewer
  files `enhancement` with `d34-followup` label, slice merges.

---

## 3. The R1-R5 review loop

Per the build plan §1.1. Every slice goes through up to five rounds
with `superpowers:code-reviewer`.

### 3.1 R1 input package

The reviewer is given:

- This `review-standards.md` document.
- The slice doc under `docs/internal/slices/COV_NN_*.md`.
- The slice diff (`git diff main..<slice-branch>`).
- `acceptance.md` for cross-reference.
- The D03 spec set (sibling cross-link target).
- The D33 spec set (sibling pattern reference).
- The pinned image's UI as seen at slice time (live run of
  `docker run --rm -p 3210:3210 -e ACCESS_CODE=t lobehub/lobe-chat:1.40.0`).

### 3.2 R1 review checklist

The reviewer walks the slice diff in this order. Each item is ✓
(pass), ✗ (fail, becomes a finding), or N/A.

| # | Check | Slice 1 | Slice 2 |
|---|---|:---:|:---:|
| 1 | All hard gates §2.1 — mechanical checks — run green in CI | ✓ | ✓ |
| 2 | Page contains: hero, Prerequisites, Steps 1-5, Deployment notes, Gotchas, What next, Maintainer docs link (in order) | ✓ | ✓ |
| 3 | Env var name `OPENAI_PROXY_URL` cited verbatim (caps + underscores) against upstream PDF | ✓ | (already there) |
| 4 | Client-mode field `API Proxy Address` cited verbatim against upstream PDF | ✓ | (already there) |
| 5 | "Verify end-to-end" Step is one bash invocation runnable from a clean clone | ✓ | ✓ |
| 6 | Server-vs-client mode disclaimer present and unambiguous | ✓ | (already there) |
| 7 | Vercel / Cloud / Client-mode notes present in Deployment notes | ✓ | (already there) |
| 8 | Gotchas covers trailing `/v1`, API key forwarding, ACCESS_CODE header, streaming, client-mode bypass | ✓ | (already there) |
| 9 | Voice / present-tense / active-voice on inspected paragraphs | (soft) | ✓ |
| 10 | Mobile readability at 375 px on the env-var snippet and compose snippet | ✓ | ✓ |
| 11 | No emoji in body text | ✓ | ✓ |
| 12 | Demo-mode `lobechat_real` listed in `make help` | ✓ | N/A |
| 13 | D03 row 11 `Verified` reads `Live` | ✓ | (already there) |
| 14 | Compose profile isolation: no `lobechat` service in other modes | ✓ | N/A |
| 15 | Image tag pinned (not `:latest`) | ✓ | (already there) |
| 16 | Verify SQL uses a time window, not a single-row-most-recent assertion | ✓ | N/A |
| 17 | Smoke prints `OK: reserve+commit verified` on success | ✓ | N/A |
| 18 | Smoke does NOT call any LobeChat admin / config API (env-var-only) | ✓ | N/A |
| 19 | Screenshot matches the live UI of the pinned image (Slice 2 only) | N/A | ✓ |
| 20 | One concept per section | ✓ | ✓ |
| 21 | D33 + D34 compose profiles coexist (no port / volume collision) | ✓ | N/A |

### 3.3 Finding severity

- **P0.** A hard gate from §2.1 / §2.2 / §2.3 is not green. Citation
  drift on `OPENAI_PROXY_URL` (row 3 of §3.2) is P0 by default — the
  env var name is the single decisive identifier on the page. Absent
  or misleading client-mode disclaimer (row 6) is P0 — silent failure
  for client-mode users is the worst-case outcome.
- **P1.** Smoke fails on a non-secret-related reason; D03 row 11 not
  reading `Live`; an `acceptance.md` §5 anti-acceptance criterion
  violated; compose profile leaks service into another mode; smoke
  calls a LobeChat admin API (contradicts F11).
- **P2.** Voice / present-tense issue on a single paragraph; cited
  upstream docs page redirected once; a gotcha block omits one known
  caveat (e.g. `OPENAI_MODEL_LIST` env var); D33 / D34 coexistence test
  flakes once.
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
`docs/specs/coverage/D34_lobechat_recipe/review-logs/COV_NN_<short>.md`.
The directory is created by Slice 1.

### 3.7 Review log content

| Section | Content |
|---|---|
| §1. Round summary | One row per round: round number, findings P0/P1/P2/P3, time-to-fix |
| §2. P0/P1 findings | Per finding: which §3.2 check failed, the diff that resolved it |
| §3. Manual smoke notes | Reviewer's own `make demo-up DEMO_MODE=lobechat_real` run output and wall-time |
| §4. Citation cross-check | Confirmation that env var name + field label match the committed PDF snapshot |
| §5. D33 / D34 coexistence | Confirmation `docker compose --profile anythingllm_real --profile lobechat_real config` parses |
| §6. Soft gates | List of soft-gate issues filed |
| §7. Verdict | `pass` / `merge-with-residuals` / `block-pending-arbitration` |

---

## 4. R5 Staff+ panel arbitration

Triggered when R5 still has P0 / P1 findings unresolved.

### 4.1 Panel composition

| Role | `subagent_type` | Rationale |
|---|---|---|
| Architecture | Software Architect | Page structure / compose profile design |
| Content systems | Technical Writer | Voice / accuracy / reader experience |
| AI / framework ecosystem | AI Engineer | LobeChat behaviour, server-vs-client nuances, streaming |
| Backend systems | Backend Architect | Smoke + verify SQL durability |
| Pragmatic implementation | Senior Developer | Ship-able pragmatic judgment |

Summarizer defaults to Technical Writer (this is a documentation
deliverable; the writer is best placed to reconcile reader-facing
trade-offs). AI Engineer summarizes only if the panel ruling hinges
on LobeChat-internal behaviour (server vs client mode, streaming
semantics) rather than page content or smoke durability.

### 4.2 Inputs to the panel

- The slice diff.
- The R1-R5 review logs.
- The unresolved P0 / P1 findings with implementer responses.
- The `tests.md` CI output for the slice.
- The committed citation PDF.
- This `review-standards.md` document.
- The D33 panel memo (if any) — for cross-deliverable consistency.

### 4.3 Output

A ≤ 1-page panel memo committed to
`docs/specs/coverage/D34_lobechat_recipe/review-logs/COV_NN_panel.md`.
The memo states the ruling, the rationale, and the residuals tracked
as GitHub issues.

---

## 5. Cross-deliverable review hooks

D34's review loop **does not** re-evaluate D03's acceptance. D34 ships
changes to D03's row 11 `Verified` column (confirming `Live`) and to
the `lobechat.md` content (replacing D03's SLICE 1 stub); these are the
only D03 touches D34 makes. If a D34 slice modifies any other D03
surface, the modification is out-of-scope and must be reverted before
merge.

D34's review loop **does** check that D33 and D34 do not collide in
the compose stack (`tests.md` §5.3 / H10 / §3.2 row 21). A regression
where D34 reuses a D33 port (3001) or volume name (`anythingllm-storage`)
is caught at the D34 review stage.

If a later D03 slice (post-D34 merge) edits row 11 in a way that
contradicts the D34 recipe (e.g. swaps the cited env var name), the
contradiction is caught at the D03 review stage and resolved in that
same D03 slice — not by re-opening D34.

If LobeChat ships a version that breaks the smoke (e.g. renames
`OPENAI_PROXY_URL` to `OPENAI_BASE_URL`, or changes the `/api/chat/openai`
request schema), the fix is a new D34 follow-up slice that bumps the
pinned image tag AND updates the page + smoke + citation PDF in
lock-step. The R1 reviewer's manual UI check (§3.1) is the gate that
catches upstream env-var rename.

---

## 6. Anti-review-scope

The reviewer is **not** asked to:

- Run LobeChat Vercel or LobeChat Cloud. Vercel is an inherited
  configuration; Cloud is hosted SaaS. The recipe documents both but
  the smoke covers self-host Docker only.
- Audit LobeChat's source code for security or quality. The pinned
  image is a third-party dependency; SpendGuard's security surface is
  the egress proxy, not LobeChat.
- Test client-mode (browser-keys) end-to-end. Client mode is
  explicitly anti-scope per `design.md` §1.2; the page documents the
  per-session UI override but the smoke does not exercise it.
- Test LobeChat's plugin / agent / TTS / image / vision surfaces.
  These are explicitly anti-scope per `design.md` §1.2.
- Re-derive the smoke's audit assertions. The verify SQL pattern is
  inherited from `verify_step_anythingllm_real.sql` (which was inherited
  from `verify_step_litellm_real.sql`) and was reviewed at those slices'
  R1; D34 reuses the pattern as-is.

The reviewer **is** asked to:

- Confirm the env var name `OPENAI_PROXY_URL` matches the LobeChat
  upstream docs captured in `citations/lobechat-environment-variables.pdf`
  with exact capitalisation and underscores (§2.1 H5, §3.2 row 3).
- Confirm the client-mode field label `API Proxy Address` matches the
  upstream PDF verbatim (§3.2 row 4).
- Run `make demo-up DEMO_MODE=lobechat_real` once and record the
  wall-time in the review log (§3.7 §3).
- Confirm the compose profile isolation (§3.2 row 14) by running
  `docker compose --profile <other-mode> config` and confirming the
  `lobechat` service does not appear.
- Confirm D33 + D34 compose coexistence (§3.2 row 21) by running
  `docker compose --profile anythingllm_real --profile lobechat_real config`.
- Confirm D03 row 11 reads `Live` post-merge (§3.2 row 13).
- Confirm the client-mode disclaimer is present and unambiguous
  (§3.2 row 6).
- Confirm voice / readability / one-concept-per-section invariants
  (§3.2 rows 9, 10, 20).
