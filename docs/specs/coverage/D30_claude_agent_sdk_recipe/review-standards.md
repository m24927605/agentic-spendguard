# D30 — Anthropic claude-agent-sdk Egress-Proxy Install Recipe — `review-standards.md`

> Status: Per-slice gate. Lands with the spec set; defines what every R1-R5 reviewer is checking.
> Sibling docs: `design.md`, `implementation.md`, `tests.md`, `acceptance.md`.
> Audience: `superpowers:code-reviewer` skill (canonical reviewer per build plan §1.2); Staff+ panel for R5 arbitration.

---

## 1. Overview and rationale

D30 is a hybrid: a marketing-grade doc page + a real end-to-end smoke that exercises the egress proxy + audit chain through a real Anthropic call. The two most failure-prone surfaces are:

1. **The PreToolUse mis-claim.** The single most damaging finding on the page would be a sentence that even implicitly suggests `PreToolUse` can act as a budget gate. The page's whole reason for existing is the opposite. A reviewer who lets this through has let SpendGuard publish a false architectural claim about Anthropic's own SDK.
2. **The smoke that passes without verifying.** A smoke that prints `PASS` but does not actually find the reserve + commit rows in `audit_outbox` would be worse than no smoke — it would create false confidence. The verify SQL is the gate, not the smoke's own exit code.

Both are mechanical checks (`tests.md` §3 + §6). The review loop's job is to verify the mechanical checks ran AND to catch the non-mechanical surfaces: voice, code-block runnability, and admonition placement.

Per the build plan §1.2, `superpowers:code-reviewer` is the canonical reviewer for every slice. This document is the review-standards input the skill consumes.

---

## 2. Slice acceptance bar

A slice **passes** and the next slice begins only when all hard gates are green. Soft gates produce GitHub issues but do not block.

### 2.1 Hard gates (every slice)

| # | Gate | Source of truth |
|---|---|---|
| H1 | mkdocs build green: `mkdocs build --strict` exits 0 | `tests.md` §2.1 |
| H2 | Page route file exists in the built `site/` tree | `tests.md` §2.2 |
| H3 | Example metadata files (`pyproject.toml`, `package.json` if Slice 2) parse | `tests.md` §2.3 |
| H4 | Internal + external link-check green | `tests.md` §3.1 |
| H5 | Upstream citation strings present on cited pages | `tests.md` §3.2 |
| H6 | All `acceptance.md` §2 functional criteria green for the slice's scope | `acceptance.md` §2 |
| H7 | Regression: `decision`, `proxy`, `agent_real_openai_agents_proxy` demo modes still pass | `tests.md` §8 |
| H8 | Review log committed in the slice's commit set | §3.6 |

### 2.2 Slice-1-only hard gates

| # | Gate | Source of truth |
|---|---|---|
| H9 | `DEMO_MODE=agent_real_claude_agent_sdk_egress make demo-up` exits 0 | `tests.md` §4.1 |
| H10 | `verify_step_claude_agent_sdk_egress.sql` exits 0 after the demo run | `tests.md` §6 |
| H11 | `[smoke] PASS` line printed by Python smoke | `tests.md` §4 |
| H12 | The `## What `PreToolUse` is — and is not` section accurately describes the SDK hook (tool-scope, not LLM-scope) | `acceptance.md` §Q6 |

### 2.3 Slice-2-only hard gates

| # | Gate | Source of truth |
|---|---|---|
| H13 | `node smoke.mjs` exits 0 with audit-chain verify | `tests.md` §5 |
| H14 | CI workflow `.github/workflows/d30-claude-agent-sdk-smoke.yml` exists and green | `tests.md` §9 |
| H15 | README `## 🔌 Adapter integrations` row added (Slice 2 only) | `acceptance.md` §3.3 |

### 2.4 Soft gates (issue, don't block)

- **S1.** A gotcha bullet could be expanded with one more edge case → reviewer files `enhancement` issue with `d30-followup` label, slice merges.
- **S2.** A cited upstream docs page has redirected once (301 → 200) → reviewer files `docs-link-drift` issue, slice merges with the redirected URL.
- **S3.** The Python or TS smoke output has a slightly noisier-than-needed log line → cosmetic; file issue, do not block.

---

## 3. The R1-R5 review loop

Per the build plan §1.1. Every slice goes through up to five rounds with `superpowers:code-reviewer`. The skill produces a structured report of findings; the implementer fixes them; the skill re-runs.

### 3.1 R1 input package

The reviewer is given:

- This `review-standards.md` document (the rubric).
- The slice doc under `docs/internal/slices/COV_NN_*.md`.
- The slice's diff (`git diff main..<slice-branch>`).
- `acceptance.md` for cross-reference.
- The link to the strategy memo (`docs/strategy/framework-coverage-2026-06.md`).
- The output of `make demo-up DEMO_MODE=agent_real_claude_agent_sdk_egress` from the slice CI (or a manual run).

### 3.2 R1 review checklist

The reviewer walks the slice diff in this order. Each item is ✓ (pass), ✗ (fail, becomes a finding), or N/A.

| # | Check | Slice 1 | Slice 2 |
|---|---|:---:|:---:|
| 1 | All hard gates §2.1 — mechanical checks — run green in CI | ✓ | ✓ |
| 2 | Doc page contains an above-the-fold PreToolUse warning (admonition / blockquote, not buried prose) | ✓ | (already there) |
| 3 | Recipe — Python section's code block can be copy-pasted to a clean shell and runs | ✓ | (already there) |
| 4 | Recipe — TypeScript section's code block can be copy-pasted and runs | (preview-only) | ✓ |
| 5 | "What `PreToolUse` is — and is not" section is technically accurate per the SDK docs | ✓ | (already there) |
| 6 | Troubleshooting bullets each map to a reproducible error | ✓ | (already there) |
| 7 | Voice / present-tense / active-voice on all paragraphs | ✓ | ✓ |
| 8 | Mobile readability at 375 px on the doc page | ✓ | (no change) |
| 9 | No emoji in body text (mkdocs admonition icons fine) | ✓ | ✓ |
| 10 | Stub recipe section for TS clearly labels "Slice 2" or "preview" in Slice 1 | ✓ | (replaced by real recipe) |
| 11 | README diff scope-correct (Slice 2 only) | N/A | ✓ |
| 12 | One concept per section (`acceptance.md` §Q5) | ✓ | ✓ |
| 13 | Working code in fenced code blocks (`acceptance.md` §Q3) | ✓ | ✓ |
| 14 | Smoke ends only after `verify_audit.py` returns 0 (`acceptance.md` §A2) | ✓ | ✓ |
| 15 | Demo regression on the 3 baseline modes (`tests.md` §8) | ✓ | ✓ |

### 3.3 Finding severity

Per the build plan's review standard. P0 / P1 findings block; P2 / P3 may merge with linked GitHub issues.

- **P0.**
  - A hard gate from §2.1 / §2.2 / §2.3 is not green.
  - Doc page implies `PreToolUse` is the LLM-scope gate (F1 violated; A1 fires).
  - Smoke prints `PASS` but `verify_step_claude_agent_sdk_egress.sql` did not actually run / did not actually pass (A2 fires).
  - Smoke uses a mocked Anthropic endpoint instead of `api.anthropic.com` (A3 fires).
- **P1.**
  - A code block in the doc page does not run as documented on a clean shell.
  - A row in the audit chain has wrong `provider` or `model` shape.
  - An anti-acceptance criterion from `acceptance.md` §5 is otherwise violated.
- **P2.**
  - A voice / present-tense / active-voice issue on a single paragraph.
  - A cited upstream docs page has redirected.
  - A troubleshooting bullet is technically correct but underspecified.
- **P3.**
  - Cosmetic (code-block language mismatch, table column alignment, oxford-comma drift).

### 3.4 Stopping rule

Per `feedback_codex_iteration_pattern` and the build plan's review cadence: maximum 5 rounds per slice. If R5 still has unresolved P0 / P1 findings, the slice escalates to the Staff+ panel (§4 below). Soft gates (P2 / P3) do not block.

R1 is automatic — the reviewer runs on every slice. R2-R5 run only if R1 produced P0 / P1 findings. The implementer fixes findings between rounds; the reviewer re-runs against the updated diff.

### 3.5 Reviewer assignment

| Slice | Primary reviewer | Tie-breaker (if R5 escalates) |
|---|---|---|
| 1 | `superpowers:code-reviewer` (canonical) | AI Engineer (SDK semantic accuracy) |
| 2 | `superpowers:code-reviewer` | AI Engineer + Frontend Developer (TS toolchain) |

The Staff+ panel composition for R5 follows the build plan §1.3, but with Summarizer overridden to **AI Engineer** (instead of the default Software Architect) — the dominant risk surface on D30 is "does the page misrepresent the SDK's hook model", which is an AI-ecosystem question, not an architecture one.

### 3.6 Review log location

Each slice's review log lives at `docs/specs/coverage/D30_claude_agent_sdk_recipe/review-logs/COV_NN_<short>.md`. The directory is created by Slice 1 as part of the slice CI scaffolding even if its first commit is the only artifact in it.

### 3.7 Review log content

Each review log contains:

| Section | Content |
|---|---|
| §1. Round summary | One row per round: round number, findings P0/P1/P2/P3, time-to-fix |
| §2. P0/P1 findings | Per finding: which §3.2 check failed, the diff that resolved it |
| §3. Manual smoke notes | Spot-check of the Python smoke (and TS smoke for Slice 2) at desktop and mobile |
| §4. Soft gates | List of soft-gate issues filed |
| §5. Demo regression check | Exit codes for the three baseline demo modes (`tests.md` §8) |
| §6. Verdict | `pass` / `merge-with-residuals` / `block-pending-arbitration` |

---

## 4. R5 Staff+ panel arbitration

Triggered when R5 still has P0 / P1 findings unresolved. The panel follows `docs/internal/review-standards/staff-panel-arbitration-process.md` §2.

### 4.1 Panel composition

| Role | `subagent_type` | Rationale |
|---|---|---|
| Architecture | Software Architect | Page structure + smoke wiring decisions |
| Content systems | Technical Writer | Voice / accuracy / reader experience |
| AI / framework ecosystem | AI Engineer | claude-agent-sdk semantic accuracy (the dominant risk) |
| Backend systems | Backend Architect | Audit-chain assertion correctness |
| Security / external surface | Security Engineer | CA trust chain + the public claim "egress proxy is the only gate" |

Summarizer is **AI Engineer** for D30 (override of the build plan default, per §3.5).

### 4.2 Inputs to the panel

- The slice diff.
- The R1-R5 review logs (§3.6).
- The unresolved P0 / P1 findings, each with the implementer's response.
- The `tests.md` CI output for the slice.
- This `review-standards.md` document.
- The output of the Python (and TS for Slice 2) smoke runs.

### 4.3 Output

A ≤ 1-page panel memo committed to `docs/specs/coverage/D30_claude_agent_sdk_recipe/review-logs/COV_NN_panel.md`. The memo states the ruling, the rationale, and the residuals that must be tracked as GitHub issues.

---

## 5. Cross-deliverable review hooks

D30's review loop **does not** re-evaluate D02's acceptance. If D02 ships a change to the on-host CA install path that breaks the doc page's recipe, the contradiction is caught at the D02 review stage and the resolution is a D02 PR that also updates D30's recipe in the same slice.

If D13 (subscription metering) later ships and overlaps with D30's BYOK assumption, the doc page's "PreToolUse vs egress proxy" admonition is unchanged but a new sub-section may be added explaining how the two modes differ. That edit is owned by D13's review, not by a D30 re-review.

---

## 6. Anti-review-scope

The reviewer is **not** asked to:

- Verify D02's CA install correctness. That is D02's deliverable.
- Audit the egress proxy's Anthropic routing. That is owned by the egress-proxy deliverable and the `services/egress_proxy/tests/multi_provider.rs` suite.
- Re-derive the claude-agent-sdk's hook semantics. The strategy memo + Anthropic's published docs are the source of truth; D30 cites them, it does not re-litigate them.
- Run any SpendGuard sidecar / ledger test outside the demo regression set in §2.1 H7.

The reviewer **is** asked to:

- Confirm the PreToolUse warning is above the fold, accurate, and visually distinct (§2.2 H12).
- Confirm both smokes terminate only after the verify SQL passes (§2.1 H6 + §3.3 P0).
- Confirm the doc page's Python and TS code blocks can be copy-pasted to a clean shell and run (§3.2 row 13).
- Confirm the three baseline demo modes (`decision`, `proxy`, `agent_real_openai_agents_proxy`) still pass at slice merge (§2.1 H7).
