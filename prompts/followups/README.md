# Phase 5 GA hardening — followup fix prompts

Self-contained prompts for an AI agent to pick up sequentially. Each file
references the corresponding GitHub issue, lists files to read first,
spells out acceptance criteria, points at established patterns in the
merged PR #2 commits, and gives explicit verification steps.

## Suggested execution order

The numbering reflects sequencing — earlier items unblock later ones, or
are smaller / more independent and produce reusable patterns.

| # | Issue | Title | Why this position |
|---|---|---|---|
| 01 | #4 | Approval notification outbox writes | Smallest scope, closes a real gap (S15 outbox dispatcher has nothing to forward today). One migration. |
| 02 | #9 | Approval bundling SP | Closes the S15+S16 end-to-end loop (`REQUIRE_APPROVAL → approve → resume real ledger op`). Independent of #4 but conceptually adjacent — review them together. |
| 03 | #5 | K8sLease backend | Independent. Removes the chart-template fail-gate from PR #2 round 5. Real `kube`-rs integration. |
| 04 | #7 | OpenAI + Anthropic real HTTP | Independent. Replaces stubs in `services/usage_poller`. Self-contained. |
| 05 | #8 | Live AWS KMS signing | Independent. PR #2 round 7+8 SP relaxations already accept arbitrary key_id/sig — only the signing side needs work. |
| 06 | #11 | Per-service /metrics endpoints | Cross-cutting. **Must land before #10 and #12** so new services follow the metrics pattern from the start, and so drills have data. |
| 07 | #10 | Retention sweeper service | Depends on #11 (metrics pattern) being in place. New crate following ttl_sweeper's shape. |
| 08 | #3 | Helm production env mapping | **Last to land**. Operator-facing — by this point the services that need env wiring (#10 retention sweeper, #11 metrics ports, possibly #5 K8s RBAC) have settled. Removes the round-6 fail-gate. |
| 09 | #12 | Per-drill runbook deep dives | **Final**. Depends on #11 (alerts actually fire) and ideally #3 (production deploy works) for the rehearsal commands to be realistic. |

## How to use these prompts

Each prompt file is intended to be the **only context** the agent needs to
pick up the work. Hand it to the agent verbatim:

```
Please apply the changes described in
prompts/followups/01-issue-4-notification-outbox.md.
Read all the files listed under "Files to read first" before making
changes. Don't deviate from the acceptance criteria.
```

The agent should:
1. Branch off `main` (`git checkout -b fix/followup-N-<short-name>`)
2. Read the listed files first
3. Implement against the acceptance criteria
4. Run the verification commands
5. Commit using the message template at the bottom of the prompt
6. Open a PR linking the GitHub issue
7. After merge, close the issue with the merge commit SHA

## Important notes

- These prompts assume the agent has read access to the merged main branch
  state. They reference specific commits from PR #2 (`a4dea4b` round 1,
  `8810c14` round 9, etc.) — those exist in main now.
- The compose-based demo path (`deploy/demo`) is the working integration
  reference. When in doubt about wiring, mirror what compose does.
- PR #2's 13-round Codex review iteration log lives in PR #2's body and in
  `MEMORY.md` — useful background but not required to start any single
  followup.
- Three followups (#3, #4, #5) were filed earlier as part of PR #2 ship;
  #7-#12 were filed afterwards from the PR body's "honest gaps" list.
  All nine are linked from main's `MEMORY.md`.
