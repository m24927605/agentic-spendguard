# D30 — Anthropic claude-agent-sdk Egress-Proxy Install Recipe — `acceptance.md`

> Status: Scope-lock document. Once accepted, the criteria here are immutable for the duration of the slice plan unless the project owner explicitly re-opens scope.
> Sibling docs: `design.md`, `implementation.md`, `tests.md`, `review-standards.md`.
> Audience: Project owner (sign-off), Technical Writer (target), R1-R5 reviewer, `superpowers:code-reviewer` skill.

---

## 1. Acceptance philosophy

This document defines the scope-locked answer to "what does D30 is done look like?" Per-slice progress lives in `review-standards.md` §3. This document is the bar: a slice that passes `review-standards.md` still does not ship "D30" until every criterion below holds against the merged diff.

Criteria reference `design.md` decision IDs (D-1 through D-6) and the sibling deliverable D02 by name so they are cross-checkable without re-reading.

A deliverable is "done" per the build plan §7 definition:

- All slices merged into main.
- All `acceptance.md` gates green.
- A row exists in `README.md` `## 🔌 Adapter integrations` (acceptance §3.3).
- A user-facing page exists under `docs.agenticspendguard.dev/integrations/claude-agent-sdk/` (acceptance §3.1).
- A demo-mode entry exists in `deploy/demo/Makefile` (acceptance §3.4) — required for D30 because the deliverable IS partially demoable.

---

## 2. Functional acceptance

Every bullet is a capability the merged deliverable MUST demonstrate against the built doc site and the running compose stack.

- **F1 (D-1, Pattern 3 only).** The rendered doc page contains an above-the-fold callout that says (verbatim or paraphrased with the same meaning) "PreToolUse is tool-scope, not LLM-scope. The egress proxy is the only LLM-scope gate for this SDK." The callout uses an mkdocs admonition / blockquote, not buried prose. Verified by inspection at R1.

- **F2 (D-1, D-6, doc page surface).** The page is at `docs/site/docs/integrations/claude-agent-sdk.md`, registered in `docs/site/mkdocs.yml`'s `nav` under `Integrations`, and renders to `docs/site/site/integrations/claude-agent-sdk/index.html` on `mkdocs build --strict`. Verified by `tests.md` §2.1 + §2.2.

- **F3 (D-2, D-3, smoke parity).** Both `examples/claude-agent-sdk-egress/python/smoke.py` and `examples/claude-agent-sdk-egress/typescript/smoke.mjs` exist; both run the same canonical prompt; both end with `[smoke] PASS`. Verified by `tests.md` §4 (Python) and §5 (TS).

- **F4 (D-4, demo mode).** `DEMO_MODE=agent_real_claude_agent_sdk_egress make demo-up` runs the Python smoke end-to-end against the compose stack and exits 0. The demo container has `HTTPS_PROXY` and `NODE_EXTRA_CA_CERTS` set to the in-container proxy + CA bundle (not the host's D02 install — D30's demo runs self-contained). Verified by `tests.md` §4.1 + §4.2.

- **F5 (D-5, audit chain).** After every smoke run, `deploy/demo/verify_step_claude_agent_sdk_egress.sql` exits 0 — i.e. one `RESERVE_RESPONSE` row and one matching `COMMIT_OUTCOME` row exist for the run, both with `provider='anthropic'` and `model LIKE 'claude-%'`, and both committed token columns > 0. Verified by `tests.md` §6.

- **F6 (D-3, TS path).** With `HTTPS_PROXY` pointed at the proxy and `NODE_EXTRA_CA_CERTS` pointed at the SpendGuard CA, `node smoke.mjs` triggers an `api.anthropic.com/v1/messages` call that the proxy intercepts, gates, forwards, and audits. Verified by `tests.md` §5.

- **F7 (regression).** `decision`, `proxy`, and `agent_real_openai_agents_proxy` demo modes still pass at slice merge time. Verified by `tests.md` §8.

- **F8 (D-6, anti-scope).** The site-v2 page (`docs/site-v2/src/content/docs/docs/integrations/claude-agent-sdk.md`) is NOT shipped by D30. Adding it is a follow-up slice or deliverable and not a D30 acceptance bar.

---

## 3. Surface acceptance

Surfaces the deliverable must expose externally.

- **S1 (page URL).** Page reachable at `https://agenticspendguard.dev/integrations/claude-agent-sdk/` (trailing slash) on the production mkdocs deploy. Verified by HTTP GET post-deploy; pre-deploy build is the slice-CI gate (`tests.md` §2.2).

- **S2 (example directory).** `examples/claude-agent-sdk-egress/` exists in `main` with at least `README.md`, `python/{pyproject.toml,smoke.py,verify_audit.py}` after Slice 1, plus `typescript/{package.json,smoke.mjs,README.md}` after Slice 2.

- **S3 (README cross-link row).** `README.md` `## 🔌 Adapter integrations` table gains one row pointing at the D30 page URL. Verified by inspection of the README diff at slice merge.

- **S4 (Makefile demo arm).** `grep -n agent_real_claude_agent_sdk_egress deploy/demo/Makefile` returns at least two hits (the `demo-up` arm and the dispatch arm). Verified by inspection.

- **S5 (CI workflow, Slice 2).** `.github/workflows/d30-claude-agent-sdk-smoke.yml` exists, runs on PRs touching the in-scope paths (`tests.md` §9), and is green at Slice 2 merge.

---

## 4. Quality acceptance

Standards the doc + smoke must meet that are not covered by mechanical tests.

- **Q1 (voice).** Doc page uses second person, present tense, active voice throughout. Verified by reviewer scan; mixed person/tense/voice fails R1 with a `voice` finding.

- **Q2 (no emoji in body).** No emoji in body text. Admonition icons supplied by mkdocs are fine; SpendGuard-author-added emojis are not. Verified by `grep` for non-ASCII codepoints in headings and paragraphs.

- **Q3 (working code).** Every fenced code block in the doc page is copy-paste runnable as shown (modulo the obvious `sk-ant-...` placeholder). Verified by spot-check at R1; a reviewer pastes at least two blocks into a clean shell and confirms they execute as documented.

- **Q4 (citation accuracy).** Per `tests.md` §3.2, the three cited upstream URLs each contain the cited literal string at slice time. A drift between spec time and slice time is a hard fail.

- **Q5 (one concept per section).** Each H2 covers one concept (Why, Prerequisites, Python recipe, TypeScript recipe, Verifying audit, PreToolUse explainer, Troubleshooting, Next steps). No section mixes two concepts. Verified by inspection.

- **Q6 (PreToolUse explainer accuracy).** The "What `PreToolUse` is — and is not" section accurately describes the SDK hook (it fires on tool invocations like Bash/Edit/Read, not on `messages` exchanges) and cites the upstream docs. A misstatement here is the highest-risk finding because it undermines the page's core thesis.

---

## 5. Anti-acceptance

The deliverable is NOT shipped — even if every gate above is green — if any of the following holds:

- **A1.** The doc page recommends using `PreToolUse` as a budget gate. (Equivalent to F1 failing; called out separately because it is the single most-damaging failure mode.)

- **A2.** The Python or TS smoke "passes" without the audit-chain SQL actually finding the rows. (`verify_step_claude_agent_sdk_egress.sql` must exit 0; a smoke that prints `PASS` then has no rows in `audit_outbox` is a P0 fail.)

- **A3.** The demo mode runs against a mocked Anthropic endpoint. The smoke MUST hit `api.anthropic.com` through the egress proxy; mocks defeat the entire D30 thesis.

- **A4.** D02 is not yet shipped at D30 ship time. D30 depends on D02's `spendguard install` for the on-host path (the in-container demo path is self-contained, but the user-facing recipe assumes D02). Ship order: D02 → D30.

- **A5.** A row is added to `README.md` `## 🧰 What works today` claiming claude-agent-sdk is "verified" without the CI workflow being green in `main` at that moment. D30 may only modify `## 🔌 Adapter integrations`, not `## 🧰 What works today` — the latter is the verified-runtime table owned elsewhere.

- **A6.** The doc page or smoke promotes Claude Code Pro / Max subscription mode. Subscription metering is D13's surface; D30 is BYOK only.

---

## 6. Slice gating

Every slice must pass `review-standards.md` §3 (the R1-R5 gate) before the next slice begins. A slice that fails R5 escalates to the Staff+ panel per the build plan §1.3.

The deliverable is shipped when:

- Slice 1 (doc + Python smoke + demo mode + verify SQL) is merged.
- Slice 2 (TS smoke + CI workflow + doc cross-link) is merged.
- All `acceptance.md` §2-§5 gates are green against the post-merge `main` branch.
- The post-deploy live URL returns 200 with the expected content.
- `DEMO_MODE=agent_real_claude_agent_sdk_egress make demo-up` returns exit 0 against a fresh clone of `main`.

---

## 7. Memory write-back

Per the build plan §8: when D30 is shipped, a memory entry under `~/.claude/projects/-Users-michael-chen-products-agentic-spendguard/memory/` named `project_coverage_D30_shipped.md` is created with:

- Merge commit SHA of Slice 1 and Slice 2.
- R1-R5 round count per slice.
- Arbitration yes / no per slice.
- The live URL.
- A one-line summary of what shipped (e.g. "doc page + Python + TS smoke + DEMO_MODE=agent_real_claude_agent_sdk_egress + 1 new verify SQL").
- The model + canonical prompt used in the verified Python + TS smokes at ship time (so any future Anthropic API rename is detected against this baseline).

This memory entry is the durable record that D30 has shipped and the claude-agent-sdk Pattern 3 surface is live.
