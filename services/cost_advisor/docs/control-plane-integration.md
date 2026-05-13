# Cost Advisor ⇄ control_plane Integration Design

> **Status**: P0 design doc, written 2026-05-13. Owners need to sign off on the
> §2 schema delta + §5 service identity before P1 implementation lands.
>
> **Scope**: how a Cost Advisor finding becomes a contract DSL patch that
> ships via the existing operator approval workflow. Codifies the spec
> §1.1 closed loop + §9 P3.5 wiring; no new product surface.

---

## 0. Decisions captured here (read first)

| # | Decision | Status | Owner |
|---|---|---|---|
| D1 | Reuse `approval_requests` table; add a `proposal_source` discriminator + `proposed_dsl_patch` + `proposing_finding_id` columns. No new "proposed_contract_patches" table. | ✅ landed in migration `0038_approval_requests_proposal_source.sql` | this PR |
| D2 | `cost_findings.finding_id` is a soft FK pointer from `approval_requests.proposing_finding_id`. Cross-database (ledger ⨝ canonical) so unenforced by Postgres; the writing service MUST validate. | ✅ landed in 0038 | this PR |
| D3 | The dashboard does NOT get a new tab. The existing approval list page learns one new URL query parameter `proposal_source=cost_advisor`. | open — needs dashboard owner ack | dashboard |
| D4 | Cost Advisor service identity = `cost-advisor:<workload_instance_id>`. It mTLS-authenticates to control_plane and only receives an `ApprovalCreate` permission scoped to `proposal_source='cost_advisor'`. | open — needs control_plane owner ack | control_plane |
| D5 | Approval → CD pipeline: an `approved` cost_advisor row gates a `bundle_registry/workflows/publish-contract-bundle.yml` invocation that consumes `proposed_dsl_patch` as the DSL delta. No new CD job; just a new trigger condition. | open — needs bundle_registry owner ack | bundle_registry |

---

## 1. Closed loop, end-to-end

```
                            spendguard_canonical                       spendguard_ledger
  ┌───────────────────┐    ┌────────────────────────┐                ┌─────────────────────────────┐
  │ canonical_events  │    │ cost_findings           │                │ approval_requests           │
  │ (audit chain)     │    │ status='open'           │                │ proposal_source='cost_      │
  └───────────────────┘    │ + evidence JSONB        │                │   advisor'                  │
           │                │ + sample_decision_ids  │                │ + proposed_dsl_patch        │
           │                └────────────────────────┘                │ + proposing_finding_id      │
           │ rule SQL                  │                              │ state pending → approved    │
           ▼                            │                              │       → CD trigger          │
  ┌───────────────────┐                │                              └─────────────────────────────┘
  │ services/         │                │ INSERT  (cost_advisor             ▲              │
  │ cost_advisor      │────────────────┴─ service, mTLS) ──────────────────┘              │
  │   rule engine     │                                                                   │ resolve_approval
  │   (P1)            │       ────────────────────────────────────────────────────────────┤   _request SP
  └───────────────────┘                                                                   │
                                                                              ┌───────────┴───────────┐
                                                                              │ operator              │
                                                                              │ (dashboard ?proposal_ │
                                                                              │  source=cost_advisor) │
                                                                              └───────────────────────┘
                                                                                          │ approve
                                                                                          ▼
                                                                  bundle_registry/publish-contract-bundle.yml
                                                                          │
                                                                          ▼
                                                          new contract bundle hash → next sidecar reload
```

Properties this preserves vs. forking a new approval queue:
- Single audit trail (`approval_events` already captures every state transition).
- Single RBAC story (`Permission::ApprovalResolve` continues to gate `/v1/approvals/:id/resolve`).
- Single dashboard idiom — operators already trained on `pending` → `approved/denied`.
- Zero new gRPC service surface (codex r3 was emphatic about this).

---

## 2. Schema delta (already shipped in this PR's migrations)

Migration `services/ledger/migrations/0038_approval_requests_proposal_source.sql`:

```sql
ALTER TABLE approval_requests
    ADD COLUMN proposal_source TEXT NOT NULL DEFAULT 'sidecar_decision'
        CHECK (proposal_source IN ('sidecar_decision', 'cost_advisor'));
ALTER TABLE approval_requests
    ADD COLUMN proposed_dsl_patch JSONB;
ALTER TABLE approval_requests
    ADD COLUMN proposing_finding_id UUID;

-- CHECK: cost_advisor rows MUST carry both patch + finding pointer.
ALTER TABLE approval_requests
    ADD CONSTRAINT approval_requests_cost_advisor_fields_present
    CHECK (
        proposal_source <> 'cost_advisor'
        OR (proposed_dsl_patch IS NOT NULL AND proposing_finding_id IS NOT NULL)
    );

-- Partial index for the dashboard's "open cost_advisor proposals" query.
CREATE INDEX approval_requests_cost_advisor_pending_idx
    ON approval_requests (tenant_id, created_at DESC)
    WHERE proposal_source = 'cost_advisor' AND state = 'pending';
```

The strengthened `approval_requests_block_immutable_updates()` trigger
freezes the three new columns at INSERT — an operator cannot
silently substitute a different patch between review and approve.

Also `tenant_data_policy` gains:
- `cost_findings_retention_days_open INT DEFAULT 90`
- `cost_findings_retention_days_resolved INT DEFAULT 30`

These drive the retention_sweeper's future `CostFindingsPurge` sweep
kind (lands in P1; see `services/retention_sweeper/`).

### What the `requested_effect` / `decision_context` columns hold for cost_advisor rows

These columns are NOT NULL on `approval_requests` (per 0026). For
cost_advisor proposals the writer fills them with:

```jsonc
// requested_effect: the contract semantics that change if approved.
//   Same shape as the original BudgetClaim list, but listing the
//   *outcome*: after this patch, the named budget's max ratio
//   becomes X / TTL becomes Y / etc.
{
  "patch_summary": "tighten idle_reservation_ratio cap from 30% to 18%",
  "affected_budget_ids": ["..."]
}
// decision_context: full forensic snapshot.
{
  "source": "cost_advisor",
  "rule_id":          "idle_reservation_rate_v1",
  "rule_version":     1,
  "finding_id":       "<uuid>",
  "evidence_summary": { /* salient metrics */ },
  "current_value":    { "idle_reservation_ratio_cap": 0.30 },
  "proposed_value":   { "idle_reservation_ratio_cap": 0.18 },
  "patch_diff_url":   null   // future: dashboard renders the diff
}
```

This lets the existing detail endpoint render the proposal without any
new column additions beyond those already in 0038.

---

## 3. Proposal lifecycle (state machine)

| Event | Actor | Effect |
|---|---|---|
| 1. Rule detects waste | `cost_advisor` runtime | UPSERT into `spendguard_canonical.cost_findings` with `status='open'` |
| 2. Rule produces a proposed patch | `cost_advisor` runtime | INSERT into `spendguard_ledger.approval_requests` with `proposal_source='cost_advisor'`, `proposed_dsl_patch=<rfc6902>`, `proposing_finding_id=<finding>`, `state='pending'`. Idempotency: composite UNIQUE `(tenant_id, decision_id)` already exists; cost_advisor generates a deterministic `decision_id` for each proposal as `uuid_v5(finding_id || rule_version)` so a re-fired finding does not double-insert. |
| 3. Operator views proposal | dashboard → control_plane `GET /v1/approvals?proposal_source=cost_advisor&state=pending` | reads, no DB write |
| 4. Operator approves | control_plane `POST /v1/approvals/:id/resolve` with `{ target_state: "approved", reason: "..." }` | calls existing `resolve_approval_request` SP — same path the sidecar_decision flow uses |
| 5. Approved → CD pipeline | bundle_registry CI job | observes the new `state='approved' AND proposal_source='cost_advisor'` row (poll or NOTIFY); fetches `proposed_dsl_patch` + the current contract bundle; emits a new bundle with the patch applied; signs; ships to endpoint_catalog |
| 6. Sidecar reload | sidecar | manifest verifier pulls the new bundle on its existing schedule; new contract applies on next decision |
| 7. cost_advisor closes the loop | cost_advisor runtime (P2+) | observes that the patch took effect (new `audit.decision` rows show the rule no longer triggers) and transitions the originating `cost_findings.status` from `'open'` to `'fixed'` |

### Idempotency rules

- **Step 2 (INSERT proposal)**: the writer uses `INSERT ... ON CONFLICT
  (tenant_id, decision_id) DO NOTHING RETURNING approval_id`. Repeated
  rule runs on the same `(tenant_id, finding_fingerprint)` produce the
  same `decision_id` so the existing UNIQUE keeps things singleton.
- **Step 7 (close loop)**: gated on at least 24h of `audit.decision` rows
  since the bundle reloaded so we don't prematurely "fix" a flaky
  detection.

### Cancellation paths

- Operator denies in step 4 → the originating `cost_findings.status` →
  `dismissed` (cost_advisor runtime watches `approval_events` for
  `to_state IN ('denied', 'cancelled')`).
- TTL expires before step 4 → status → `dismissed` with reason
  `proposal_ttl_expired`.

---

## 4. Dashboard filter — no new tab, one new query parameter

The existing dashboard read path is the control_plane REST API. Two
extensions to ship in P3.5 (per spec §9):

### 4.1 `GET /v1/approvals` learns one new filter

```http
GET /v1/approvals?tenant_id=<uuid>&state=pending&proposal_source=cost_advisor
```

`proposal_source` is optional; when absent the response includes both
sources (backward compat). When set, the SQL becomes:

```sql
SELECT approval_id, tenant_id, decision_id, state, proposal_source,
       proposing_finding_id, ttl_expires_at, created_at
  FROM approval_requests
 WHERE tenant_id = $1 AND state = $2 AND proposal_source = $3
 ORDER BY created_at DESC
 LIMIT $4
```

Uses the new partial index. SELECTed columns grow by two
(`proposal_source`, `proposing_finding_id`); response struct extended
correspondingly.

### 4.2 `GET /v1/approvals/:id` returns the patch + finding pointer

`ApprovalDetail` gains:
- `proposal_source: String`
- `proposed_dsl_patch: Option<serde_json::Value>`
- `proposing_finding_id: Option<Uuid>`

Dashboard UI: on the detail page, when `proposal_source == cost_advisor`,
render the RFC-6902 patch as a syntax-highlighted JSON diff (mostly a
front-end concern; backend just hands over the JSON). The
`proposing_finding_id` is rendered as a deep link to the cost_findings
row (dashboard needs read access to spendguard_canonical for this —
already configured for the audit-export endpoint per
`SPENDGUARD_DASHBOARD_CANONICAL_DATABASE_URL`).

### 4.3 No new POST endpoint

Approval mutations continue to flow through `POST /v1/approvals/:id/resolve`.
The control_plane handler doesn't care about `proposal_source` for
resolve — the same `resolve_approval_request` SP applies. (Future
hardening: if we want different RBAC for cost_advisor vs. sidecar_decision
resolves, we'd add a `Permission::ApprovalResolveCostAdvisor` subkind.
Out of P0 scope.)

---

## 5. Service identity + auth (mTLS)

`cost_advisor` runtime needs **write access** to:
- `spendguard_canonical.cost_findings` — INSERT + UPDATE (lifecycle).
- `spendguard_ledger.approval_requests` — INSERT only, scoped to
  `proposal_source='cost_advisor'`.

### 5.1 Database role

Mirrors the existing pattern (`canonical_ingest_application_role` in
`services/canonical_ingest/migrations/0005_immutability_triggers.sql`,
`outbox_forwarder` etc.):

```sql
-- Will be added in P1 alongside the runtime; included here so reviewers
-- can sign off on the surface now.
CREATE ROLE cost_advisor_application_role NOINHERIT;

-- canonical DB grants:
GRANT INSERT, UPDATE, DELETE ON cost_findings
                              TO cost_advisor_application_role;
GRANT INSERT, UPDATE ON cost_findings_fingerprint_keys
                              TO cost_advisor_application_role;
GRANT SELECT ON canonical_events, ledger_units (read-only join targets)
                              TO cost_advisor_application_role;

-- ledger DB grants:
GRANT SELECT ON commits, ledger_entries, ledger_transactions, reservations
                              TO cost_advisor_application_role;
GRANT INSERT ON approval_requests
                              TO cost_advisor_application_role;
-- column-level restriction: the role can only INSERT rows whose
-- proposal_source = 'cost_advisor'. Enforced by a row-security
-- policy + a BEFORE INSERT trigger that rejects other values for this
-- role.
```

`DELETE` on `cost_findings` is allowed (it's a derived artifact, not
audit). `UPDATE` on `cost_findings` for lifecycle transitions
(`open → dismissed | fixed | superseded`); the `cost_findings_touch`
trigger handles `updated_at`.

### 5.2 mTLS workload identity

The compose / Helm chart adds a new TLS cert pair `cost-advisor.crt` /
`.key` issued by the same PKI root as the other services. Service
identity string: `cost-advisor:<workload_instance_id>`. Same pattern as
`sidecar:demo-sidecar-1`, `outbox-forwarder:demo-outbox-forwarder`, etc.

### 5.3 control_plane gate

control_plane today authenticates the operator and gates writes by
`Permission::ApprovalResolve`. The cost_advisor service is NOT an
operator — it doesn't use the control_plane REST API to write. It
writes to `approval_requests` directly via its mTLS-authenticated
DB connection. control_plane learns about the new row when the
operator views the dashboard.

If we later want cost_advisor to flow proposals through control_plane
(rather than directly into Postgres), the natural extension is a new
`POST /v1/approvals/proposals` endpoint that requires
`Permission::ProposalCreate` and only admits `proposal_source='cost_advisor'`.
Out of P0 scope; flagged here so the door is open.

### 5.4 Audit chain implications

INSERT into `approval_requests` writes nothing into `audit_outbox`
today — the existing `post_approval_required_decision_sp` SP (migration
0037) only fires for sidecar_decision flow. cost_advisor proposals
are NOT a ledger event; they're a derived recommendation. The audit
trail for cost_advisor proposals lives entirely in `approval_events` +
`cost_findings.evidence`. This is a deliberate split: ledger audit
chain is for monetary effects; cost_advisor audit chain is for
recommendations that may or may not become monetary effects.

Future spec question (O5 for §11): when a cost_advisor proposal is
approved AND the resulting contract bundle ships, do we tie the
approval back into the next sidecar reload's bundle change event? My
recommendation: yes, via `bundle_registry`'s existing audit hooks (it
already signs + publishes contract bundles to the endpoint catalog;
that publish event is the natural anchor).

---

## 6. Open items requiring owner ack

| # | Owner | Question |
|---|---|---|
| Q1 | dashboard | Will the existing `/v1/approvals` list view render the new query param's results without UI change, or does the existing UI break on the extra response fields? Need to verify on the demo dashboard before P1 wraps. |
| Q2 | control_plane | Does the existing `Permission::ApprovalResolve` apply uniformly to cost_advisor + sidecar_decision rows, or do operators have different review obligations for the two sources? My read: same permission. Confirm. |
| Q3 | bundle_registry | What is the exact trigger from "approval_requests.state→approved" to a new contract bundle build? Today the workflow is manually invoked; the cost_advisor loop wants a polling worker or a NOTIFY listener. P3.5 task. |
| Q4 | bundle_registry | RFC-6902 patches in `proposed_dsl_patch`: which paths are addressable? Need a schema for the contract DSL's JSON Pointer surface so cost_advisor can validate patches at proposal time, not at CD time. |
| Q5 | security | Cross-database soft FK (`approval_requests.proposing_finding_id → spendguard_canonical.cost_findings.finding_id`): is the cost_advisor service's validate-before-INSERT sufficient, or do we need a periodic reconciler that flags orphan rows? My read: validate-before-INSERT is enough for v0.1; revisit at scale. |

---

## 7. Implementation order for P3.5

1. Database role + column-level INSERT policy (cost_advisor runtime).
   *Already-planned-for-P1 work.*
2. control_plane: extend `list_approvals` + `get_approval` response
   structs with the three new columns. Add `proposal_source` query
   filter. ~half day.
3. Dashboard: add the URL query parameter passthrough + a section in
   the detail page that renders `proposed_dsl_patch`. ~1 day depending
   on UI framework.
4. bundle_registry: pollerless trigger via Postgres LISTEN/NOTIFY on
   `approval_requests` `AFTER UPDATE`. The notification carries the
   approval_id; the CI worker fetches `proposed_dsl_patch` + the
   current contract bundle, builds the new bundle, signs, publishes.
   ~1 day if NOTIFY infra exists; ~3 days if not (then we use a poll
   loop instead).
5. cost_advisor: the close-the-loop watcher in step 7 of the lifecycle.
   Lands in P2 because it depends on the baseline refresher being
   live. Not blocking P3.5 itself.

Total P3.5 (control_plane + dashboard + bundle_registry trigger): ~3
days. Matches spec §9 estimate.

---

## 8. Footnotes / dropped alternatives

- **"Add a separate `proposed_contract_patches` table"**: rejected.
  Doubles the surface area for approvals (operators would learn two
  views), forks RBAC, requires a new SP for state transitions.
  Codex r3 was explicit that the win is reusing approval_requests.
- **"control_plane wraps cost_advisor inserts in a new REST endpoint"**:
  deferred to v0.2. P0.5 / P1 keep cost_advisor's DB write direct
  because the wrapping endpoint adds latency without adding value at
  this scale.
- **"Use audit_outbox for cost_advisor proposals so they ride the
  existing audit chain to canonical_events"**: rejected as a category
  error. audit_outbox is for ledger writes that have monetary
  consequences. cost_advisor proposals are recommendations until
  approved + a contract bundle ships; the consequence happens at
  bundle-reload time, not at proposal-create time.
