# Followup #12 — Per-drill runbook deep dives (S23)

GitHub issue: https://github.com/m24927605/agentic-flow-cost-evaluation/issues/12

## Goal

Turn the 4 placeholder drill scenarios in `docs/site/docs/operations/slos.md`
into real runbooks that on-call can actually rehearse. Wire every alert in
`deploy/observability/prometheus-rules.yaml` to a `runbook_url` annotation
pointing to the matching drill doc.

Sequencing: do this **last** in the followup chain — depends on issue #11
(per-service metrics shipped) so the alerts actually fire and rehearsals
have data to react to.

## Files to read first

- `docs/site/docs/operations/slos.md` — current state: L1-L9 SLO targets
  table + 4 drill scenario *names* (placeholder one-liners)
- `deploy/observability/prometheus-rules.yaml` — 8 alert groups; each
  alert has `expr` + `for` + `labels` but the `annotations.runbook_url`
  is empty / generic
- `docs/site/docs/operations/multi-pod.md` — existing high-quality
  runbook style; copy the structure (symptoms, first-check, mitigation,
  escalation)
- `docs/site/docs/operations/data-classification.md` — second example of
  the established runbook style

## The 4 drill scenarios

The names below are taken from `slos.md`'s placeholder list. Each
becomes its own page under `docs/site/docs/operations/drills/`. Confirm
the actual names against the current `slos.md` and adjust if the
placeholders changed.

1. **Lease lost mid-batch** — outbox_forwarder or ttl_sweeper loses its
   leader lease while a batch is in flight. Drill validates round-9
   `is_leader_now()` gating and the worker's recovery on next cycle.

2. **Audit chain forwarder backlog** — outbox_forwarder paused / slow,
   audit_outbox queue grows. Drill validates the alert fires at the
   documented threshold and the runbook's drain procedure works.

3. **Strict signature quarantine spike** — canonical_ingest sees
   InvalidSignature surge (e.g. operator rotated a key without updating
   the trust store, or a producer is misconfigured). Drill validates
   the round-1 P2#3 / round-9 admit-counter behavior in non-strict
   modes vs the strict-mode quarantine rate.

4. **Approval TTL wave** — many approvals expire at once, sweeper
   needs to handle the burst. Drill validates round-5 system-actor
   injection (migration 0030) + round-9 atomic TTL guard
   (migration 0033) + the sweeper's batch sizing behaves under load.

## Acceptance criteria

- 4 new files under `docs/site/docs/operations/drills/`:
  - `lease-lost-mid-batch.md`
  - `audit-chain-forwarder-backlog.md`
  - `strict-signature-quarantine-spike.md`
  - `approval-ttl-wave.md`
- Each follows the multi-pod.md / data-classification.md style:
  1. **Symptoms** — what does the on-call see (alert names from
     prometheus-rules.yaml + dashboard view)
  2. **First check** — kubectl / psql / curl one-liners that confirm
     the diagnosis
  3. **Mitigation** — short-term unblock (no code changes)
  4. **Escalation** — when to wake whom (cross-link the owner table
     in slos.md)
  5. **Rehearsal** — concrete steps to trigger the scenario on the
     demo cluster (kind / compose) and verify the alert + mitigation
     work end-to-end without prod traffic
- `prometheus-rules.yaml`: every alert in every group gets
  `annotations.runbook_url:
  https://docs.spendguard.example/operations/drills/<drill-doc-name>`.
  If a single alert maps to multiple drills, point to the most relevant
  one and cross-link in the doc
- `slos.md` index links each drill doc + each existing standalone
  runbook (multi-pod, data-classification). Owner table cross-references
  the right team per scenario
- Reviewer can run **one** of the 4 drill rehearsals on the
  compose-based demo cluster and confirm the alert fires + the
  mitigation steps unblock the system
- Lint: `mkdocs build` (or whatever docs site CI uses) passes with no
  broken links across all 4 new docs + slos.md updates

## Pattern references

- `multi-pod.md` shows the right tone (operator-direct, copy-paste
  command snippets, no marketing prose)
- `data-classification.md` shows how to mix schema-table reference with
  operator playbook in the same doc
- PR #2 commit `64026ab` (S23) is the original SLO doc commit; the
  drill scenario names there are the source of truth

## Verification

```bash
# Docs build
cd docs/site && mkdocs build  # or whatever the site builder is
# expect: no broken links

# Manual rehearsal of one drill (lease-lost-mid-batch is the simplest)
make demo-up DEMO_MODE=invoice
docker pause spendguard-ttl-sweeper  # simulate lease loss
sleep 30
# follow the rehearsal steps in the new doc; confirm alert fires
# (if metrics from issue #11 are wired) and the mitigation works
docker unpause spendguard-ttl-sweeper
make demo-down
```

## Commit + close

```
docs(s23): per-drill runbook deep dives + alert runbook_url wiring (followup #12)

Four drill scenarios from slos.md become real runbooks under
docs/site/docs/operations/drills/. Every prometheus alert in
prometheus-rules.yaml now has a runbook_url annotation pointing
to the matching drill doc.

Manual rehearsal of lease-lost-mid-batch on the compose demo
cluster confirmed the alert fires + mitigation works.
```

After merge: `gh issue close 12 --comment "Shipped in <commit-sha>"`.
