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

### 5.2.1 Trigger + RLS implementation (codex r7 P2)

The hand-wavy "BEFORE INSERT trigger that rejects other values for this role" in §5.1 needs a concrete mechanism. Postgres supports two:

**Mechanism A — Row-level security (preferred):**

```sql
ALTER TABLE approval_requests ENABLE ROW LEVEL SECURITY;
ALTER TABLE approval_requests FORCE ROW LEVEL SECURITY;

CREATE POLICY cost_advisor_insert_self_only
    ON approval_requests
    FOR INSERT
    TO cost_advisor_application_role
    WITH CHECK (proposal_source = 'cost_advisor');

-- Other roles bypass the policy (control_plane / sidecar legacy
-- callers continue to INSERT with proposal_source='sidecar_decision').
-- The default-permissive policy is omitted here so the role-scoped
-- policy is the only INSERT path for cost_advisor.
```

Caveat: RLS policies are evaluated against the CURRENT role. The
service must connect with login role `cost_advisor_login` that has
membership in `cost_advisor_application_role` AND issue
`SET ROLE cost_advisor_application_role` at session start. Without
SET ROLE, `current_user` stays as the login role and the policy does
not match. Document this in the service's connection-init code.

**Mechanism B — BEFORE INSERT trigger (fallback if RLS unavailable):**

```sql
CREATE OR REPLACE FUNCTION enforce_cost_advisor_proposal_source()
    RETURNS TRIGGER LANGUAGE plpgsql AS $$
BEGIN
    IF pg_has_role(session_user, 'cost_advisor_application_role', 'USAGE')
       AND NEW.proposal_source <> 'cost_advisor' THEN
        RAISE EXCEPTION
            'role % may only INSERT proposals with proposal_source=cost_advisor (got %)',
            session_user, NEW.proposal_source
            USING ERRCODE = '42501';   -- insufficient_privilege
    END IF;
    RETURN NEW;
END; $$;
CREATE TRIGGER cost_advisor_proposal_source_guard
    BEFORE INSERT ON approval_requests
    FOR EACH ROW
    EXECUTE FUNCTION enforce_cost_advisor_proposal_source();
```

`pg_has_role(session_user, 'cost_advisor_application_role', 'USAGE')` correctly identifies whether the connecting role has membership, regardless of whether `SET ROLE` was issued — more forgiving than the RLS path. The trigger only rejects when the role IS cost_advisor AND it tries to write a non-cost_advisor proposal; other roles pass through unaffected.

Decision: ship **Mechanism A (RLS)** if the runtime can guarantee `SET ROLE` at session start; otherwise ship Mechanism B (trigger). Lands in P1.

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
| Q5 | security + cost_advisor | Cross-database soft FK (`approval_requests.proposing_finding_id → spendguard_canonical.cost_findings.finding_id`): **codex r5 P1-5 rejected my v1 answer**. Validate-before-INSERT has a TOCTOU window — between (validate finding exists, INSERT approval_requests) the retention sweeper can DELETE the finding, leaving a dangling `proposing_finding_id` whose dereference in the dashboard / CD pipeline either silently fails or applies an orphan proposal. The fix is in §9 below (added per r5). |

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

## 9. Cross-DB referential safety (added per codex r5 P1-5)

The original §0 D2 + §5 design declared `approval_requests.proposing_finding_id → cost_findings.finding_id` a "soft FK enforced by validate-before-INSERT". Codex r5 P1-5 caught two real holes:

1. **TOCTOU race**: between (cost_advisor reads `cost_findings`, validates the finding exists) and (cost_advisor INSERTs `approval_requests` with `proposing_finding_id`), the retention sweeper or an operator can DELETE the finding (`cost_findings` is a derived artifact with no immutability protection). The INSERT then commits with a dangling pointer.
2. **Retention-driven dangling**: even after a valid INSERT, the retention sweeper can DELETE the originating finding while the proposal is still `pending` (e.g. operator hasn't reviewed in 90 days). The dashboard "view finding details" deep-link from the proposal page then 404s.

### 9.1 Fix: reference-counted retention on cost_findings

Add a `referenced_by_pending_proposal` boolean (or counter) column to `cost_findings`. retention_sweeper refuses to DELETE rows where this is TRUE/>0. The flag is maintained by:

- INSERT into `approval_requests` with `proposal_source='cost_advisor'` → also UPDATE `cost_findings.referenced_by_pending_proposal=TRUE` for that finding_id. Both writes go in one cross-DB orchestration step in `cost_advisor` service (atomic via 2-phase commit OR by serializing: first canonical UPDATE, then ledger INSERT, with rollback compensation if the second fails).
- approval_requests state→terminal (approved/denied/expired/cancelled) → UPDATE `cost_findings.referenced_by_pending_proposal=FALSE`.

A periodic reconciler (~hourly) sweeps `cost_findings` and corrects drift. Codex r6 P1 caught that the v1 design only handled "terminal referenced approvals"; the reconciler must additionally handle every drift state:

| Drift | Detection | Repair |
|---|---|---|
| Flagged TRUE, referencing approval is now terminal | LEFT JOIN cost_findings.referenced_by_pending_proposal=TRUE ⨝ approval_requests.proposing_finding_id WHERE approval_requests.state IN ('approved','denied','expired','cancelled') | flip flag to FALSE |
| Flagged TRUE, but NO approval row references the finding (orphan flag from a half-committed orchestration step) | LEFT JOIN cost_findings TRUE rows ⨝ approval_requests; approval row IS NULL | flip flag to FALSE after a grace window (10 min) so we don't race an in-flight INSERT |
| Flagged FALSE, but a pending approval row still references the finding (rare: the proposal-write path crashed between approval INSERT and flag UPDATE) | LEFT JOIN approval_requests state='pending' AND proposal_source='cost_advisor' ⨝ cost_findings WHERE referenced_by_pending_proposal=FALSE | flip flag to TRUE so the retention sweeper stops chasing the finding |
| approval row exists, finding is GONE (the only truly-bad state) | approval_requests state='pending' AND proposal_source='cost_advisor' WHERE proposing_finding_id NOT IN (SELECT finding_id FROM cost_findings) | log + page operator; orphan proposals require manual decision (approve based on the cached evidence in approval_requests.decision_context, or cancel). This state SHOULD be unreachable if the reference flag works; the reconciler treats it as an invariant breach. |

The reconciler runs as a P1 background job in cost_advisor. The 10-minute grace window for orphan flags is the explicit tolerance for half-committed orchestration: if the orchestrator crashes after canonical UPDATE but before ledger INSERT, the proposal-write path retries within 10 minutes (sub-minute on a healthy worker) and the flag stays TRUE; if it never retries, the reconciler clears the flag. Out of P0 scope; the safety hole is closed at the design level via this section + the open-item routing in §6 Q5.

### 9.2 Schema delta

To land in P1 (not this P0; this is design intent only):

```sql
-- In spendguard_canonical
ALTER TABLE cost_findings
    ADD COLUMN referenced_by_pending_proposal BOOLEAN NOT NULL DEFAULT FALSE;
CREATE INDEX cost_findings_referenced_pending_idx
    ON cost_findings (tenant_id)
    WHERE referenced_by_pending_proposal = TRUE;
```

`retention_sweeper`'s future `CostFindingsPurge` sweep kind:

```sql
DELETE FROM cost_findings
 WHERE tenant_id = $1
   AND referenced_by_pending_proposal = FALSE
   AND (
        (status = 'open'      AND detected_at < now() - $2::interval)
     OR (status IN ('dismissed','fixed','superseded')
         AND detected_at < now() - $3::interval)
   );
```

The reconciler runs as a P1 background job in cost_advisor. Out of P0 scope; the safety hole is closed at the design level via this section + the open-item routing in §6 Q5.

### 9.3 Why not a hard FK / single DB

We do NOT move `cost_findings` into `spendguard_ledger` to enforce a real FK because:
- Audit chain + cost analytics have different scaling profiles. `canonical_events` + `cost_findings` are append-heavy + retention-managed; `spendguard_ledger` is the financial-truth DB with stricter SLOs.
- Separating preserves the spec invariant "cost_advisor proposals are recommendations, not ledger events" — they reach `audit_outbox` only after operator approval flips a row to `approved` AND the bundle ships, never at proposal-create time.

The cross-DB design stays; the safety hole is closed by the reference flag + reconciler instead.

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
