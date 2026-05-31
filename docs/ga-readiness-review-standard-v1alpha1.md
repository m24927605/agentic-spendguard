# GA Readiness Review Standard v1alpha1

> **Status**: draft
> **Reviewer**: codex CLI through AIT adversarial mode

---

## §0. Required Invocation

Every GA slice review uses:

```bash
ait run \
  --adapter codex \
  --review-mode adversarial \
  --base main \
  --branch ga/GA_NN_<name> \
  --slice-doc docs/slices/GA_NN_<name>.md \
  --review-budget deep
```

If the local AIT wrapper rejects a flag, record the failure and dispatch a separate codex CLI adversarial reviewer with the same base, branch, and slice doc context. Do not switch to the claude-code adapter.

---

## §1. Review Scope

The reviewer checks:

- implementation matches the slice doc
- acceptance gates actually ran
- evidence is reproducible
- no hidden local state is required
- no security baseline regressed
- no review finding is deferred without a named prerequisite
- docs do not cite nonexistent conventions or fabricated slice decisions

---

## §2. Severity Handling

Severity helps prioritize the fix order but does not change the closure rule. Blockers, Majors, and Minors all require a real in-slice fix unless Staff+ arbitration accepts an explicit out-of-scope decision after R5.

---

## §3. Round Budget

Maximum review rounds: 5.

After each round:

1. Copy findings into the slice implementation notes.
2. Fix every finding.
3. Commit the fix with a Co-Authored-By trailer.
4. Re-run relevant acceptance gates.
5. Re-run adversarial review.

After R5 with findings, stop the codex loop and run Staff+ arbitration.

---

## §4. Staff+ Arbitration

Panel roles:

- Software Architect
- Release Engineering Architect or Backend Architect
- SRE/Operations Architect
- Security Engineer
- Performance/Database Architect or domain expert

Each panelist receives the last review findings, implementation branch, and slice doc. The panel votes:

- fix in-slice
- accept as out-of-scope
- choose option A/B/C

The majority decision is final. If tied, Software Architect breaks the tie.

---

## §5. Reviewer Anti-Patterns

Reject a slice if it:

- uses shim-only evidence while claiming real-stack readiness
- documents a manual gate but provides no command
- emits dashboards or alerts for metrics that do not exist
- stores secret material in examples
- rewrites shipped migrations instead of adding forward migrations
- hides failures behind "best effort" release scripts
- leaves a long-running process or dev server open at final response
- skips memory update after merge
