# Round-2 followup prompts

After round-1 shipped 6 of 9 (#4, #5, #7, #9-part1, #10, #12), the
remaining 4 issues (#3, #8, #9-part2, #11) needed sharper scope splits
to fit autonomous-session execution. These round-2 prompts encode that.

## Execution order (round 2)

1. **`01-issue-11-per-service-metrics.md`** — start with `ledger`. One
   service per PR. Pattern is mechanical; ride the momentum. Stop at
   any service if context budget runs low.
2. **`02-issue-3-helm-prod-env-mapping.md`** — code-only PRs (no kind
   cluster). One service per PR; smallest mismatches first. The
   `chart.profile=production` fail-gate stays asserted until operator
   validates on real kind cluster post-merge.
3. **`03-issue-8-kms-toolchain-bump.md`** — wider risk: bumps project
   rust 1.88 → 1.91 across all Dockerfiles in PR 8a, then live KMS
   integration in PR 8b.
4. **`04-issue-9-part2-bundling-rpc.md`** — split into 4 sub-PRs (proto
   + ledger handler, sidecar wiring, Python SDK, demo mode). Largest
   total scope.

## Why this order

- #11 is the simplest pattern → quick wins, validates the round-2
  process.
- #3 is code-only and won't break main if not finished.
- #8 has the highest blast radius (toolchain bump touches every
  service) — only attempt after lower-risk work is in.
- #9 part 2 is largest + requires the most coordination across services
  + adapter SDK; comes last so partial completion still leaves earlier
  PRs in a clean shippable state.

## Severity reminder (from honest triage)

These 4 are **production-maturity polish, not pilot blockers**.
Compose-based deploys + LocalEd25519 PEM signing + STOP-rule contracts +
log-based observability cover the first design-partner workflow.
Round-2 work matters when graduating from pilot to first
paying-production customer.
