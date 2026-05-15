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
| D2 | `cost_findings.finding_id` is a real FK target from `approval_requests.proposing_finding_id`. **CA-P1.6**: `cost_findings` was relocated from `spendguard_canonical` to `spendguard_ledger` (migrations 0040/0041/0042) so the FK is Postgres-enforced. Soft-FK + reconciler design from §9 is HISTORICAL — superseded by the real FK. | ✅ landed in 0038 + 0042 | this PR |
| D3 | The dashboard does NOT get a new tab. The existing approval list page learns one new URL query parameter `proposal_source=cost_advisor`. | open — needs dashboard owner ack | dashboard |
| D4 | Cost Advisor service identity = `cost-advisor:<workload_instance_id>`. It mTLS-authenticates to control_plane and only receives an `ApprovalCreate` permission scoped to `proposal_source='cost_advisor'`. | open — needs control_plane owner ack | control_plane |
| D5 | Approval → CD pipeline: an `approved` cost_advisor row gates a `bundle_registry/workflows/publish-contract-bundle.yml` invocation that consumes `proposed_dsl_patch` as the DSL delta. No new CD job; just a new trigger condition. | open — needs bundle_registry owner ack | bundle_registry |

---

## 1. Closed loop, end-to-end

```
       spendguard_canonical                        spendguard_ledger
  ┌───────────────────┐                ┌──────────────────────────────────────────┐
  │ canonical_events  │                │ cost_findings (CA-P1.6: relocated here)  │
  │ (audit chain)     │                │ + cost_findings_id_keys (FK target)      │
  └───────────────────┘                │ + cost_baselines                         │
           │                            └──────────────────────────────────────────┘
           │ rule SQL (READ-only,                          │ FK
           │  canonical pool)                              ▼
           │                            ┌──────────────────────────────────────────┐
           │                            │ approval_requests                        │
           │                            │ + proposal_source='cost_advisor'         │
           │                            │ + proposed_dsl_patch                     │
           │                            │ + proposing_finding_id (real FK)         │
           │                            │ state pending → approved → CD trigger    │
           │                            └──────────────────────────────────────────┘
           ▼                                          ▲              │
  ┌───────────────────┐                              │              │
  │ services/         │  WRITE (ledger pool, mTLS)   │              │
  │ cost_advisor      │──────────────────────────────┘              │ resolve_
  │ rule engine (P1)  │                                             │ approval_
  └───────────────────┘                                             │ request SP
                                                       ┌────────────┴────────────┐
                                                       │ operator                │
                                                       │ (dashboard ?proposal_   │
                                                       │  source=cost_advisor)   │
                                                       └─────────────────────────┘
                                                                    │ approve
                                                                    ▼
                                            bundle_registry/publish-contract-bundle.yml
                                                                    │
                                                                    ▼
                                            new contract bundle hash → next sidecar reload
```

**CA-P1.6 note**: `cost_findings`/`cost_findings_id_keys`/`cost_baselines` now live in `spendguard_ledger`. The cost_advisor runtime reads canonical_events from the canonical pool (rule SQL) and writes findings + reads baselines on the ledger pool. The `approval_requests.proposing_finding_id` FK is real (Postgres-enforced) — see §9.

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
| 1. Rule detects waste | `cost_advisor` runtime | UPSERT into `spendguard_ledger.cost_findings` (via `cost_findings_upsert()` SP) with `status='open'`. CA-P1.6: was canonical, now ledger. |
| 2. Rule produces a proposed patch | `cost_advisor` runtime | INSERT into `spendguard_ledger.approval_requests` with `proposal_source='cost_advisor'`, `proposed_dsl_patch=<rfc6902>`, `proposing_finding_id=<finding>`, `state='pending'`. Idempotency: composite UNIQUE `(tenant_id, decision_id)` already exists; cost_advisor generates a deterministic `decision_id` for each proposal as `uuid_v5(finding_id || rule_version)` so a re-fired finding does not double-insert. |
| 3. Operator views proposal | dashboard → control_plane `GET /v1/approvals?proposal_source=cost_advisor&state=pending` | reads, no DB write |
| 4. Operator approves | control_plane `POST /v1/approvals/:id/resolve` with `{ target_state: "approved", reason: "..." }` | calls existing `resolve_approval_request` SP — same path the sidecar_decision flow uses |
| 5. Approved → bundle re-publish | `services/bundle_registry/` daemon (CA-P3.5 shipped) | LISTENs on `approval_requests_state_change` channel; on NOTIFY for `state='approved' AND proposal_source='cost_advisor'`, fetches `proposed_dsl_patch`, applies it to the active contract bundle YAML, re-bundles deterministically, writes the new `.tgz` + signature placeholder + updates the runtime.env `SPENDGUARD_SIDECAR_CONTRACT_BUNDLE_HASH_HEX` line atomically. Startup recovery scan handles approvals committed during downtime (LISTEN/NOTIFY is not durable across restarts). |
| 6. Sidecar reload | sidecar (v0.1: restart-required; P3.6+: hot-reload) | **v0.1**: the sidecar loads bundles ONCE at startup; the operator's CD pipeline must restart the sidecar for it to read the new contract. **P3.6+**: hot-reload via bundle watcher (see `services/sidecar/src/decision/transaction.rs` "POC has no hot-reload" comment). |
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
row. **CA-P1.6**: cost_findings now lives in `spendguard_ledger`, so
the dashboard reads it from the ledger pool (the same pool it already
uses to read `approval_requests`). The previous canonical-DB read path
for findings is no longer needed.

### 4.3 No new POST endpoint

Approval mutations continue to flow through `POST /v1/approvals/:id/resolve`.
The control_plane handler doesn't care about `proposal_source` for
resolve — the same `resolve_approval_request` SP applies. (Future
hardening: if we want different RBAC for cost_advisor vs. sidecar_decision
resolves, we'd add a `Permission::ApprovalResolveCostAdvisor` subkind.
Out of P0 scope.)

---

## 5. Service identity + auth (mTLS)

`cost_advisor` runtime needs **write access** via SECURITY DEFINER
SPs (never direct table DML):

- `spendguard_ledger.cost_findings` — written via `cost_findings_upsert()` SP (CA-P1.6 — sole legal writer).
- `spendguard_ledger.cost_baselines` — INSERT + UPDATE for the nightly baseline refresher (P2).
- `spendguard_ledger.approval_requests` — written via `cost_advisor_create_proposal()` SP (CA-P3). The SP hard-codes `state='pending'` + NULL resolution fields so the caller cannot bypass `resolve_approval_request`. NO direct INSERT grant.

And **read access** to:
- `spendguard_canonical.canonical_events` — SELECT only (rule SQL).
- `spendguard_ledger.approval_requests` — SELECT for status checks.

**Migration chain** (codex CA-P3 r3 P2):
1. **0043 (this PR)** — creates `cost_advisor_create_proposal()` SECURITY DEFINER SP + `REVOKE ALL FROM PUBLIC`. No role exists yet; only the function owner (postgres / migration runner) can invoke.
2. **Future P1-role migration** — `CREATE ROLE cost_advisor_application_role` + `GRANT EXECUTE` on the SPs listed above. Until then the cost_advisor service connects as the migration runner.

### 5.1 Database role

Mirrors the existing pattern (`canonical_ingest_application_role` in
`services/canonical_ingest/migrations/0005_immutability_triggers.sql`,
`outbox_forwarder` etc.):

```sql
-- DEFERRED TO P1 ROLE MIGRATION (NOT shipped in 0043).
-- 0043 creates the SP + REVOKEs from PUBLIC; the future P1 role
-- migration creates the role + grants EXECUTE. Until P1 ships,
-- the cost_advisor service connects as the migration runner.
--
-- CA-P1.6: cost_findings now lives in spendguard_ledger (was canonical).
-- Single role, single DB for the writer surface.
CREATE ROLE cost_advisor_application_role NOINHERIT;

-- canonical DB grants (READ-only — rule SQL reads canonical_events):
GRANT SELECT ON canonical_events
                              TO cost_advisor_application_role;

-- ledger DB grants:
-- Direct INSERT/UPDATE/DELETE on cost_findings + the mirrors is NOT
-- granted. The SP is the SOLE legal writer (spec §11.5 A1) and the
-- role only gets EXECUTE on it. Writes that bypass the SP would skip
-- the mirror maintenance and could violate the FK chain.
GRANT EXECUTE ON FUNCTION cost_findings_upsert
                              TO cost_advisor_application_role;
-- Lifecycle transitions (status='open' → 'fixed' / 'dismissed') run
-- via narrower SPs not yet shipped; for P0/P1 the runtime only INSERTs
-- via the upsert SP.
GRANT SELECT ON cost_findings, cost_findings_fingerprint_keys,
                cost_findings_id_keys, cost_baselines
                              TO cost_advisor_application_role;
GRANT SELECT, INSERT, UPDATE ON cost_baselines
                              TO cost_advisor_application_role;
GRANT SELECT ON commits, ledger_entries, ledger_transactions, reservations
                              TO cost_advisor_application_role;

-- approval_requests: NO direct INSERT/UPDATE/DELETE. The
-- cost_advisor_create_proposal SECURITY DEFINER SP (migration 0043)
-- is the SOLE legal writer for cost_advisor proposals. It
-- hard-codes state='pending' + NULL resolution fields so a
-- compromised writer cannot bypass the resolve_approval_request
-- transition (which fires approval_events + the pg_notify trigger
-- on AFTER UPDATE OF state). Codex CA-P3 r1 P1 + r2 P1 closed this
-- bypass.
GRANT EXECUTE ON FUNCTION cost_advisor_create_proposal(
    UUID, UUID, JSONB, UUID, TIMESTAMPTZ
)                             TO cost_advisor_application_role;
GRANT SELECT ON approval_requests
                              TO cost_advisor_application_role;  -- read-only for status checks
```

`UPDATE` / `DELETE` on `cost_findings` is gated behind the
`cost_findings_upsert()` SP (and future lifecycle SPs covering
`open → dismissed | fixed | superseded`); direct table access is
intentionally NOT granted so writers cannot bypass the
mirror-maintenance + FK-chain invariants. The `cost_findings_touch`
trigger handles `updated_at`.

### 5.2 mTLS workload identity

The compose / Helm chart adds a new TLS cert pair `cost-advisor.crt` /
`.key` issued by the same PKI root as the other services. Service
identity string: `cost-advisor:<workload_instance_id>`. Same pattern as
`sidecar:demo-sidecar-1`, `outbox-forwarder:demo-outbox-forwarder`, etc.

### 5.2.1 Write-path enforcement — SECURITY DEFINER SP (CA-P3 final)

> **Status**: replaced RLS / BEFORE-INSERT-trigger approach (v1/v2/v3
> drafts below preserved as historical) with the
> `cost_advisor_create_proposal` SECURITY DEFINER SP shipped in
> migration 0043. Codex CA-P3 r1 P1 + r2 P1 closed this loop.

The cost_advisor write path uses **only** the SECURITY DEFINER SP:

```sql
-- Migration 0043:
GRANT EXECUTE ON FUNCTION cost_advisor_create_proposal(
    UUID, UUID, JSONB, UUID, TIMESTAMPTZ
)                             TO cost_advisor_application_role;

-- No direct INSERT/UPDATE/DELETE grants on approval_requests for
-- cost_advisor. The SP is hardened:
--   * SECURITY DEFINER (runs as the migration owner, has INSERT perms)
--   * search_path = pg_catalog, pg_temp (defeats temp-shadow attacks)
--   * REVOKE ALL FROM PUBLIC (only granted roles can invoke)
--   * Hard-codes state='pending' + NULL resolution fields so the
--     caller can't write a fake-approved row, skipping
--     resolve_approval_request and the pg_notify trigger.
```

Why this beats the RLS/trigger approach: with role-based gates, the
gate exists at the GRANT layer (revocable, fallible). With the SP,
the gate is built into the only callable surface — any future role
that gets EXECUTE on this SP inherits all the safety properties
without further configuration.

#### Historical drafts (RLS / BEFORE INSERT trigger)

The v1 doc proposed RLS or a BEFORE INSERT trigger to constrain
which `proposal_source` value a cost_advisor role could INSERT.
That approach worked but had two operational issues:
  1. RLS required `SET ROLE` at every session start.
  2. The BEFORE INSERT trigger needed `pg_has_role(session_user,
     'cost_advisor_application_role', 'MEMBER')` checks that broke
     if role inheritance changed.

The SP approach replaces both: no role-membership introspection
needed; the SP body is the gate.

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
| Q5 | RESOLVED via CA-P1.6 (2026-05-14) | Cross-database soft FK was identified by codex r5 P1-5 as having TOCTOU + retention-dangling holes. **Resolution**: greenfield no-backcompat decision allowed moving `cost_findings` from `spendguard_canonical` to `spendguard_ledger` (migrations 0040/0041/0042). The FK is now real and Postgres-enforced via the `cost_findings_id_keys` mirror (cost_findings PK is partitioned so can't be the FK target directly). The reconciler/reference-flag design from §9 is HISTORICAL — superseded. |

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

## 9. Cross-DB referential safety — RESOLVED via CA-P1.6 (real FK)

> **Status (2026-05-14)**: this section is HISTORICAL. The reconciler design described below was never implemented. Codex r5 P1-5 + r6 P1 identified holes in the soft-FK + reconciler approach; the v0.1 greenfield-no-backcompat property let us pick a simpler fix: move `cost_findings` into `spendguard_ledger` and use a real Postgres FK.

### 9.0 Current design (CA-P1.6, what actually ships)

`cost_findings`, `cost_findings_fingerprint_keys`, `cost_findings_id_keys`, and `cost_baselines` all live in `spendguard_ledger` (migrations 0040, 0041, 0042). The FK is:

```sql
-- Composite tenant-scoped FK (codex CA-P1.6 r1 P2) so cross-tenant
-- pointers are rejected at write time.
ALTER TABLE approval_requests
    ADD CONSTRAINT approval_requests_proposing_finding_id_fkey
    FOREIGN KEY (tenant_id, proposing_finding_id)
    REFERENCES cost_findings_id_keys (tenant_id, finding_id)
    ON DELETE RESTRICT
    NOT VALID;
ALTER TABLE approval_requests
    VALIDATE CONSTRAINT approval_requests_proposing_finding_id_fkey;

-- Back-FK so retention DELETE on cost_findings cascades through the
-- mirror — and is blocked by the RESTRICT step above if any
-- approval_requests references the finding.
-- (Declared on cost_findings_id_keys itself in migration 0042.)
-- cost_findings_id_keys (tenant_id, detected_at, finding_id)
--     REFERENCES cost_findings (tenant_id, detected_at, finding_id)
--     ON DELETE CASCADE
```

Why a mirror (`cost_findings_id_keys`) instead of a direct FK on `cost_findings.finding_id`: cost_findings is partitioned on `(tenant_id, detected_at)`, so its PK must include the partition key — `(tenant_id, detected_at, finding_id)`. Postgres FKs can only target a UNIQUE-on-FK-columns target; a single-column FK on `finding_id` therefore needs a non-partitioned mirror with `PRIMARY KEY (finding_id)`. The mirror is maintained in lockstep by `cost_findings_upsert()` (inserted/updated/reinstated paths all touch it).

**ON DELETE RESTRICT semantics** (the FK default): retention_sweeper can DELETE unreferenced findings (no approval ever proposed) but is blocked from deleting findings that any approval_request points at. That preserves the audit anchor — every proposal that ever existed has its originating evidence preserved alongside it — without any reconciler.

The stale-mirror self-heal path in `cost_findings_upsert()` (reinstated outcome) issues a `DELETE FROM cost_findings_id_keys WHERE finding_id = <dead-id>`. If any approval_requests row references that finding_id, the DELETE is rejected by the FK, the SP aborts, and the invariant breach is surfaced loudly instead of being silently overwritten.

### 9.1 What was wrong with the v1 soft-FK design (historical, for traceability)

Codex r5 P1-5 caught two holes:

1. **TOCTOU race**: between (cost_advisor reads `cost_findings`, validates the finding exists) and (cost_advisor INSERTs `approval_requests` with `proposing_finding_id`), the retention sweeper or an operator could DELETE the finding (cost_findings is a derived artifact with no immutability protection). The INSERT would then commit with a dangling pointer.
2. **Retention-driven dangling**: even after a valid INSERT, the retention sweeper could DELETE the originating finding while the proposal was still `pending` (e.g. operator hadn't reviewed in 90 days). The dashboard "view finding details" deep-link would 404.

### 9.2 The reconciler design that was rejected (historical)

The original §9 proposed a `referenced_by_pending_proposal` flag column maintained by cross-DB orchestration, plus an hourly reconciler with 4 drift states and a 10-minute grace window. Codex r6 P1 specifically flagged that this still had race windows. We retained the design intent until CA-P1.6 made it unnecessary by relocating the table.

### 9.3 Why we DID move cost_findings into spendguard_ledger (reversed from v1)

The v1 doc argued AGAINST moving cost_findings into ledger:
> "Audit chain + cost analytics have different scaling profiles."
> "Separating preserves the spec invariant 'cost_advisor proposals are recommendations, not ledger events'."

Both are wrong with the benefit of CA-P1.6 perspective:
- **Scaling profile**: cost_findings is small (one row per detected waste pattern per tenant per day; bounded by `tenant_data_policy.cost_findings_retention_days_*`). It does NOT have canonical_events' append-rate or retention surface. It's also write-heavy from a single writer (cost_advisor) — same profile as ledger_transactions. Living in ledger costs nothing.
- **Invariant**: "proposals are recommendations not ledger events" is preserved by the proposal_source discriminator + the approval-resolve workflow, not by physical DB separation. cost_findings rows are NOT audit events; they're derived analytics. The audit chain (canonical_events) still lives where it always did.

CA-P1.6 was a strict simplification: it deleted ~60 lines of reconciler design, ~80 lines of cross-DB orchestration plumbing, and replaced them with a 90-line migration. Net code reduction. Net invariant strengthening.

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
