# Staff+ Panel Arbitration Process

> 🎯 **Trigger**: a slice's round-5 adversarial review still has findings after the implementer's fixes. Per HANDOFF §8.6, this is when Staff+ panel arbitration kicks in.
>
> **Outcome**: a single arbitration ruling that closes round 5 and allows the slice to merge (via `ait apply`) with the ruling linked in the merge commit message.

---

## §0. When to convene

### 0.1 Trigger conditions

- Round 5 review completed
- Findings still present after implementer's round-5 fix attempt
- Implementer requests arbitration (not auto-triggered — implementer judges)

Round 5 findings can be of any severity (Blocker / Major / Minor). Even residual Minor findings → panel still convenes（per `predictor-review-checklist.md` §4 round-pass rule）。

### 0.2 NOT a trigger

- Round 1-4 findings — fix and re-run, don't escalate
- Round 5 zero findings — no panel needed; clean apply
- Implementer disagrees with reviewer Round 1-4 — discuss in adversarial review; don't escalate

---

## §1. Panel composition

### 1.1 Universal panelists (always)

| Panel role | `subagent_type` |
|---|---|
| Architecture | `Software Architect` |
| Backend systems | `Backend Architect` |
| Security / audit-chain | `Security Engineer` |
| Code review rigor | `Code Reviewer` |

### 1.2 Domain expert (varies per slice)

| Slice domain | Domain expert `subagent_type` |
|---|---|
| Postgres migration / schema (SLICE 01) | `Database Optimizer` |
| Proto / Contract DSL (SLICE 02) | `Backend Architect` (additional) |
| Rust services / sidecar / proxy (SLICE 03-05, 09-11) | `Backend Architect` (additional) |
| Stats / SQL aggregation (SLICE 06) | `Data Engineer` |
| Python SDK (SLICE 12) | `Backend Architect` or Python-specific |
| CLI tool (SLICE 13) | `Backend Architect` |
| Benchmark / test design (SLICE 15) | `Performance Benchmarker` or `API Tester` |
| Documentation (any slice doc) | `Technical Writer` |
| Customer template (SLICE 14) | `Backend Architect` + `Technical Writer` |

### 1.3 Panel size

Default = 4 universal + 1 domain expert = 5 panelists. Maintainer can adjust per slice complexity.

---

## §2. Materials preparation

Before convening, implementer prepares the following document set:

### 2.1 Required materials

1. **Original slice spec** (`docs/slices/SLICE_XX_*.md`)
2. **Spec ancestor(s)** (the 1-3 spec files this slice implements)
3. **All findings from rounds 1-5** (output of `ait review finding list --attempt <id>`)
4. **Round-5 implementer response** (what was attempted to fix; why findings remain)
5. **Current implementation diff** (`git diff main...slice/SLICE_XX_<name>`)
6. **Demo regression report** (which `make demo-up DEMO_MODE=...` modes were tested)
7. **`predictor-review-checklist.md`** verbatim
8. **This file** (`staff-panel-arbitration-process.md`)

### 2.2 Optional helpful materials

- Latency benchmark results
- Audit-chain `verify-chain` output
- Cross-spec consistency analysis（if findings touch multiple specs）

### 2.3 Material packaging

Compile into a single ZIP or directory; dispatch each panelist with same materials. Identical context across panelists is required for fair arbitration.

---

## §3. Memo format

Each panelist produces a memo (≤ 1 page markdown) with this structure:

```markdown
# Arbitration Memo — Panelist: [role/subagent_type]
# Slice: SLICE_XX [short name]
# Date: YYYY-MM-DD

## Position
[1 sentence: which findings should be kept / dropped / modified, and overall verdict (merge / block / rework)]

## Evidence
[2-4 bullet points: specific findings reviewed + evidence from materials supporting position]

## Recommendation
[1 paragraph: what implementer should do; what panel should rule]

## Risk assessment
[1-2 sentences: if the panel rules in favor of merge with residuals, what risks accepting; if rules block, what cost accepting]
```

Memo MUST be ≤ 1 page (~ 500 words). Brevity forces clarity.

---

## §4. Summarizer ruling format

After all panelists submit, a designated summarizer (`subagent_type: Software Architect` by default; maintainer can override) produces the final ruling.

### 4.1 Summarizer dispatch

```
Agent({
  subagent_type: "Software Architect",
  description: "Arbitration ruling summarizer for SLICE_XX",
  prompt: "Read the 5 panelist memos and produce a single arbitration ruling.
            Ruling must: (a) cite each panelist's position;
                        (b) reconcile disagreements with reasoned argument;
                        (c) issue final verdict (merge / block / rework);
                        (d) specify any action items for implementer.
            Output: a single markdown document, ≤ 2 pages, structure below."
})
```

### 4.2 Ruling structure

```markdown
# Arbitration Ruling — SLICE_XX [short name]
# Date: YYYY-MM-DD
# Convened due to: round-5 findings residual after fix attempt

## Panel composition
[List of panelists + their roles]

## Panelist positions summary
[Each panelist's verdict in 1 sentence; cite memo]

## Key disagreements
[Where panelists differ; cite evidence]

## Reconciled judgment
[Summarizer's reasoned synthesis]

## Final verdict
[ ] Merge with documented residuals (residuals tracked as GH issues per HANDOFF feedback `codex_iteration_pattern`)
[ ] Block until implementer addresses [specific findings]
[ ] Rework spec ancestor (escalate to spec-level discussion; halt slice)

## Action items
- [Implementer task 1]
- [Implementer task 2]
- ...

## Linked artifacts
- Round 1-5 findings: ...
- Implementation diff: ...
- Demo regression report: ...
```

---

## §5. Implementer follow-through

### 5.1 Verdict: Merge

- Implementer applies any remaining action items from ruling §4.2 "Action items"
- Re-runs adversarial review one final time（round 6） to confirm action items closed
- If clean → `ait apply --to slice/SLICE_XX_<name> --mode branch`
- Merge commit message includes link to ruling document（per §6）

### 5.2 Verdict: Block

- Implementer addresses specific findings listed in ruling
- Re-runs adversarial review from round 1 (fresh start)
- Round count resets

### 5.3 Verdict: Rework spec ancestor

- Slice halts
- Implementer escalates to maintainer
- Spec-level discussion / revision required before slice can resume
- After spec revision, new slice attempt begins fresh

---

## §6. Closure & merge commit recording

Merge commit message format:

```
<conventional commit subject for SLICE_XX>

<body describing slice deliverables>

Round 5 finding residuals arbitrated by Staff+ panel.
Ruling: docs/arbitrations/SLICE_XX-ruling-YYYY-MM-DD.md
Panelists: [list]

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
```

The ruling document itself is committed to `docs/arbitrations/` directory (created on first arbitration occurrence; per-slice files永久 retained as historical record).

---

## §7. Multi-slice arbitration cap

If 3 or more slices in the same spec set hit Staff+ panel arbitration → maintainer pause cascade and re-review the underlying spec. Possible signals:

- Spec ancestor has subtle invariant that hot reviewers keep catching
- `predictor-review-checklist.md` universal checks need rebalance
- Implementer / reviewer adversarial calibration off

Pause cadence: 24h cooling-off + maintainer-led retrospective.

---

## §8. Edge cases

### 8.1 Panelists disagree fundamentally

- Summarizer's reasoned synthesis is final
- If summarizer cannot reconcile → escalate to maintainer for tie-break

### 8.2 Panelist unavailable / non-responsive

- Maintainer assigns alternate within same role bucket
- Panel size minimum 4 (don't go below 80% capacity)

### 8.3 Implementer wants to challenge ruling

- Single appeal allowed: implementer submits 1-page rebuttal
- Panel reconvenes (same composition) and issues amended ruling
- Amended ruling is final

---

*Process version: staff-panel-arbitration-process v1alpha1 | Triggered by round-5 fail per HANDOFF §8.6 | Companion: `predictor-review-checklist.md` for universal checks | Maintainer override at any step*
