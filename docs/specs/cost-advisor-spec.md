# Cost Advisor — Design Spec

> **Status**: **v4** (2026-05-13, post-CA-P0 implementation + codex r5-r8 adversarial audit on the P0 branch). v0 → v1 → v2 → v3 → v4 changelog at §14. Codex audit log at §15.
>
> **v4 thesis**: implementation reality on the CA-P0 branch (commit `42cb787`) revealed that v3's scope assumptions were partially wrong. Codex rounds 5-8 closed the gap. v4 reconciles spec text with what actually shipped + what the P1 path actually requires. §0 below summarizes the corrections; the original v3 narrative is preserved in §1-§13 for traceability. **Where v3 narrative conflicts with §0, §0 is authoritative.**
>
> **Codex audit history**: r1 4.5/10 → r2 5.2/10 → r3 5.0/10 (rescope) → r4 GREEN_LIGHT_FOR_P0 → r5 RED (7 P1) → r6 RED (3 P1) → r7 RED (1 P1) → r8 **GREEN** (P1 readiness gate cleared). Full log §15.
>
> **Codename**: `cost-advisor` (final brand TBD; see §10).
> **Owner**: Agentic SpendGuard
> **Closes**: post-event suggestion gap noted in `benchmarks/real-stack-e2e/REAL_LANGCHAIN_E2E.md` (the "事後建議優化" row flagged ❌)

---

## 0. v4 corrections summary (authoritative)

This section reconciles the v3 narrative with what the CA-P0 implementation + codex r5-r8 verified. Where any section below the line conflicts with §0, §0 wins.

### 0.1 Scope cut: v0.1 ships ZERO rules (was: 4 rules)

v3 §5.1 listed 4 rules for v0.1. Reality:

- `idle_reservation_rate_v1`: blocked. Column is `reservations.current_state`, not `latest_state`; allowed values do NOT include `ttl_expired` (TTL expiry is in `audit_outbox` release event `reason='TTL_EXPIRED'`); no `ttl_seconds` column. Requires new `reservations_with_ttl_status_v1` view (workstream P0.6, issue #49).
- `failed_retry_burn_v1`, `runaway_loop_v1`, `tool_call_repeated_v1`: all blocked on `canonical_events.payload_json` lacking `prompt_hash`, `agent_id`, `run_id`, `tool_name`, `tool_args_hash`, `model_family` (5 of 6 fields are 0% populated). Requires sidecar enrichment (workstream P0.5, issue #48). `tool_*` requires SDK extension (deferred to v0.2).

**Net**: v0.1 has zero fireable rules until P0.5 + P0.6 land. P1 (issue #50) then ships `idle_reservation_rate_v1` as the first fireable rule. P1.5 (issue #51) ships the other 3 at run-scope (after P0.5).

### 0.2 `canonical_events.payload_json` shape (was: decoded `{kind, ...}`)

v3 §3 + §6 examples implied `payload_json->>'kind'` and `payload_json->>'committed_micros_usd'` work directly. Reality (per `services/canonical_ingest/src/persistence/append.rs`):

```json
{
  "specversion":     "1.0",
  "type":            "spendguard.audit.outcome",
  "source":          "sidecar://...",
  "id":              "<uuid>",
  "time_seconds":    1234567890,
  "datacontenttype": "application/json",
  "data_b64":        "<base64-encoded JSON>",
  "tenantid":        "<uuid>",
  "runid":           "<uuid or empty until P0.5>",
  "decisionid":      "<uuid>",
  "producer_id":     "sidecar:...",
  "producer_sequence": 42,
  "signing_key_id":  "..."
}
```

Rule SQL must decode `data_b64` via `convert_from(decode(payload_json->>'data_b64', 'base64'), 'UTF8')::jsonb` to reach inner fields. Tier-2 baseline SQL example in v3 §6 is wrong as written.

Per-call cost data (`committed_micros_usd`, `estimated_amount_atomic`) lives in `ledger.commits` and `ledger_entries` in the `spendguard_ledger` database — NOT in `canonical_events.payload_json`. Rules that quantify waste in USD MUST join `canonical_events.decision_id ⨝ ledger_transactions.decision_id ⨝ commits` + `ledger_units` + `pricing_snapshots` for unit/currency normalization.

### 0.3 §4.1 `cost_findings` uses mirror + upsert SP, not direct UNIQUE INDEX

v3 §4.1 (line 237 of the v3 file) showed `CREATE UNIQUE INDEX cost_findings_fingerprint ON cost_findings (tenant_id, fingerprint)`. Reality (post-codex r6 P1-3): Postgres requires UNIQUE constraints on partitioned tables to include the partition key. The implementation uses a non-partitioned mirror table:

```sql
CREATE TABLE cost_findings_fingerprint_keys (
    tenant_id   UUID NOT NULL,
    fingerprint CHAR(64) NOT NULL,
    finding_id  UUID NOT NULL,
    detected_at TIMESTAMPTZ NOT NULL,
    PRIMARY KEY (tenant_id, fingerprint)
);
```

Writes go through `cost_findings_upsert()` stored procedure (the SOLE legal writer) which atomically claims the mirror slot, then INSERTs / UPDATEs / self-heals an orphan canonical row. Returns `outcome ∈ {inserted, updated, reinstated}`. Direct INSERTs that skip the SP risk orphan mirror rows or duplicate findings.

### 0.4 §5.1.2 failure classifier — column landed, code pending

`canonical_events.failure_class TEXT` column + CHECK enum + partial index landed in CA-P0 (`services/canonical_ingest/migrations/0011_add_failure_class.sql`). Classifier code (`services/canonical_ingest/src/classify.rs` per spec §5.1.2) is gated on the P1 / P1.5 issue cycle (#50 / #51). Pre-migration rows stay NULL (canonical_events is append-only — backfill requires out-of-band trigger drop). Rules read NULL as "not classified" and don't fire — degraded launch is mechanically supported.

### 0.5 §9 phasing table revision

Add P0.5 (5d) + P0.6 (2d) rows. Revise P1 (was 6d, now 4d) — was "skinny rule" assuming the rule was fireable; reality is "runtime + first rule wired against P0.5/P0.6 outputs". P1.5 stays at 5d.

**New v0.1 critical path**: P0 4d (done) + max(P0.5 5d, P0.6 2d) + P1 4d + P3 4d + P3.5 3d = **20 days** elapsed. (v3 said 17.) Schedule contingency consumed: hit the spec §11.5 A2 "scenario 3: 3+ fields missing OR fundamental shape mismatch" branch (+5 days envelope).

### 0.6 §11.5 A2 branch decision recorded

The audit-report `docs/specs/cost-advisor-p0-audit-report.md` is the authoritative scenario-3 record. v3 §11.5 A2 contingency table planned for this case; v4 confirms the case is what happened.

### 0.7 §11.5 Q5 retention — now schema-backed

v3 Q5 said "90 days for `open`; 30 days for `dismissed` / `fixed`; auto-purge with `retention_sweeper`". CA-P0 landed the schema (`tenant_data_policy.cost_findings_retention_days_open` default 90, `tenant_data_policy.cost_findings_retention_days_resolved` default 30, in `services/ledger/migrations/0038_*.sql`). The retention sweeper sweep kind itself + reconciler (per integration doc §9) is P1 work.

### 0.8 Cross-DB referential safety — reconciler design

Cross-DB soft FK from `approval_requests.proposing_finding_id → cost_findings.finding_id`. Codex r5 P1-5 + r6 P1-2 rejected "validate-before-INSERT" as insufficient. v4 design (per integration doc §9): `cost_findings.referenced_by_pending_proposal` flag maintained by the proposal-write path; retention sweeper refuses DELETE when TRUE; periodic reconciler covers 4 drift states with a 10-minute grace window. Lands in P1.

### 0.9 Service identity + INSERT scope — concrete mechanism

v3 hand-waved "column-level INSERT scoped to proposal_source='cost_advisor' via a BEFORE INSERT trigger". v4 integration doc §5.2.1 spells out two implementable mechanisms: RLS with FORCE ROW LEVEL SECURITY + per-role policies, OR a BEFORE INSERT trigger using `pg_has_role(session_user, 'cost_advisor_application_role', 'MEMBER')`. Decision: RLS if the runtime can guarantee `SET ROLE` at session start, otherwise trigger.

### 0.10 §1.1 closed loop still holds

The fundamental v3 rescope (cost_advisor is a feature, not a parallel product; proposals flow through existing approval queue; no new dashboard tab) is unchanged by v4. The closed-loop diagram in §1.1 remains correct.

---

## 1. Problem statement

After SpendGuard caps and audits LLM spend, customers still need to know:

- **What patterns are wasting money?** (e.g., runaway loops, redundant tool calls, idle reservations)
- **Why?** (root-cause hypothesis tied to a specific incident, not a generic alert)
- **What to do?** (concrete contract DSL patch, ideally one approve-click away from being live)

Today the product produces an immutable audit chain (`canonical_events`) but no analysis layer on top. Customers must:
- Write their own SQL queries against `canonical_events`
- Or buy a separate observability tool (LangSmith, Helicone, Langfuse)
- Or just guess

This is the "事後建議優化" gap. **Cost Advisor closes the loop**: detected waste in the audit chain → proposed contract DSL patches → operator approves via existing approval workflow → patches take effect at next sidecar reload. The customer's experience is **one new screen** (the proposed-patches queue, which is just a view filter on the existing approval queue), not a parallel analytics product.

### 1.1 The closed loop

```
┌──────────────────────────────────────────────────────────────────┐
│  agent run                                                        │
│  → sidecar enforces contract (existing)                           │
│  → audit row written to canonical_events (existing)               │
│                                                                   │
│              ▼ (post-event, async)                                │
│                                                                   │
│  cost_advisor rule engine reads canonical_events (NEW)            │
│  → detects waste pattern                                          │
│  → emits FindingEvidence + a proposed contract DSL patch          │
│                                                                   │
│              ▼                                                    │
│                                                                   │
│  proposal queued in control_plane approval queue (EXISTING        │
│  workflow that operators already use for REQUIRE_APPROVAL paths)  │
│                                                                   │
│              ▼                                                    │
│                                                                   │
│  operator reviews proposal in existing dashboard approval tab     │
│  (no new dashboard tab — just a filter for `proposal_source =     │
│  cost_advisor`)                                                   │
│                                                                   │
│              ▼ approve                                            │
│                                                                   │
│  contract_bundle CD pipeline picks up patch (existing)            │
│  → next sidecar reload enforces new contract                      │
│                                                                   │
│              ▼ enforced                                           │
│                                                                   │
│  next agent run is now capped/redirected/throttled per the patch  │
└──────────────────────────────────────────────────────────────────┘
```

**Why this framing is better** (codex r3 insight): the customer is already trained on the approval workflow. Adding a parallel "Cost Advisor dashboard" doubles the surface area to learn. Routing proposals through the existing queue means: zero new operator UI, zero new RBAC story, zero new audit chain (proposals already go through audit), zero new gRPC API.

---

## 2. Non-goals

- **Not a real-time abort tool**. SpendGuard sidecar already does that (Tier 0 of the stack). Cost Advisor runs **after the fact** on the audit chain.
- **Not a generic LLM observability platform**. We don't store prompts, completions, or traces beyond what SpendGuard already keeps in `canonical_events`. Helicone / Langfuse are stronger here; we don't compete.
- **Not a billing reconciler**. Provider invoices reconcile via the existing `usage_poller` + `webhook_receiver` services.
- **Not autonomous remediation**. Findings produce *proposals*; the existing operator approval workflow gates whether they take effect.
- **NOT a separate product surface (codex r3)**. No new dashboard tab, no new gRPC service, no new Python SDK methods, no Slack/email digest. Everything flows through existing control_plane / dashboard / approval primitives. The only NEW user-visible thing is a filter on the existing approval queue (`proposal_source = cost_advisor`) and a CLI subcommand `spendguard advise` for power users.

---

## 3. Architecture — 4 layers, each independently togglable

> **Naming clarification (codex r1)**: the 4 tiers describe **marginal cost ramp** (free → free → ~$0.02/tenant/day → premium), not lifecycle phase. Some tiers will reuse infrastructure (e.g. Tier 2 is rule-class with materialized aggregates). Tier 4 is conditional on opt-in **prompt-retention extension** that is OUT of scope for the base "we don't store prompts" non-goal — to use Tier 4, customers must explicitly enable a separate retention policy on a separate `prompt_archive` table.

```
┌─────────────────────────────────────────────────────────────────┐
│  Tier 1: Rule engine (always on, $0/month marginal cost)        │
│  ────────────────────────────────────────────────────────────   │
│  postgres views on canonical_events / ledger_transactions /     │
│  reservations. Detects deterministic anti-patterns. Outputs     │
│  structured findings (no narrative).                            │
└──────────────────────────┬──────────────────────────────────────┘
                           │
┌──────────────────────────▼──────────────────────────────────────┐
│  Tier 2: Statistical baseline (always on, $0/month)             │
│  ────────────────────────────────────────────────────────────   │
│  per-tenant / per-agent rolling baselines (median, p95,         │
│  completion-rate). Flags outliers (>3σ). Postgres percentile.   │
└──────────────────────────┬──────────────────────────────────────┘
                           │
┌──────────────────────────▼──────────────────────────────────────┐
│  Tier 3: LLM narrative wrapper (opt-in, ~$0.01/tenant/day)      │
│  ────────────────────────────────────────────────────────────   │
│  Take Tier 1+2 structured findings → gpt-4o-mini → human-       │
│  readable narrative with specific numbers + actionable fix +    │
│  contract DSL snippet.                                          │
└──────────────────────────┬──────────────────────────────────────┘
                           │
┌──────────────────────────▼──────────────────────────────────────┐
│  Tier 4: Embedding clustering (premium, ~$0.0001/event)         │
│  ────────────────────────────────────────────────────────────   │
│  Embed prompts; cluster by semantic similarity; flag clusters   │
│  with abnormal cost variance. Catches anomalies rules miss.     │
│  Defer to v2.                                                   │
└─────────────────────────────────────────────────────────────────┘
```

**Design principle**: detection is cheap and deterministic (SQL). LLM is only used to *rephrase* findings, not to *find* them. This decouples cost from quality.

---

## 4. Data model

### 4.0 `FindingEvidence` contract (added codex r1, Must-Fix #1)

Every finding produced by ANY rule MUST emit evidence conforming to this contract. Stable shape so dashboard / CLI / narrative validator / dedup all depend on the same fields.

```jsonschema
{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "title": "CostFindingEvidence",
  "type": "object",
  "required": ["rule_id", "rule_version", "fingerprint", "scope", "metrics", "decision_refs"],
  "properties": {
    "rule_id": {"type": "string", "pattern": "^[a-z0-9_]+_v[0-9]+$"},
    "rule_version": {"type": "integer", "minimum": 1},
    "fingerprint": {
      "type": "string",
      "description": "Deterministic SHA-256 hex of (rule_id, scope.canonical_repr, time_bucket). MUST be stable across nightly re-runs of the same underlying data so UPSERTs idempotently mark the same finding rather than insert duplicates.",
      "pattern": "^[0-9a-f]{64}$"
    },
    "scope": {
      "type": "object",
      "required": ["scope_type"],
      "properties": {
        "scope_type": {"enum": ["agent", "run", "tool", "tenant_global"]},
        "agent_id": {"type": ["string", "null"]},
        "run_id": {"type": ["string", "null"]},
        "tool_name": {"type": ["string", "null"]},
        "model_family": {"type": ["string", "null"]}
      }
    },
    "metrics": {
      "type": "array",
      "description": "Typed numeric facts. Narrative validator renders every numeric token in the narrative text by string-templating from this array — the LLM NEVER emits free-form numeric strings (codex r2). Open additionalProperties was too lax.",
      "items": {
        "type": "object",
        "required": ["name", "value", "unit", "source_field", "pii_classification"],
        "properties": {
          "name": {"type": "string", "pattern": "^[a-z_][a-z0-9_]*$", "description": "Stable key for narrative templates"},
          "value": {"type": "number"},
          "unit": {"enum": ["micros_usd", "usd", "tokens", "calls", "seconds", "ratio", "count", "percent", "multiplier"]},
          "source_field": {"type": "string", "description": "Where this value came from: 'canonical_events.payload.committed_micros_usd' or 'derived: SUM(failed_retry.commit)' etc. Audit trail for the value"},
          "pii_classification": {"enum": ["none", "model_id", "agent_id", "tool_name", "prompt_text_excerpt", "tenant_metadata"], "description": "Defaults 'none'. If anything other than 'none' or 'model_id', this metric MAY NOT be sent to Tier-3 LLM"},
          "derivation": {"type": "string", "description": "Optional formula explaining derived metrics. Required if name contains 'ratio', 'multiplier', or 'delta'"},
          "ci95_low": {"type": "number"},
          "ci95_high": {"type": "number"}
        },
        "additionalProperties": false
      }
    },
    "decision_refs": {
      "type": "array",
      "description": "Sample decision_ids from canonical_events that this finding is derived from. Used by dashboard 'view raw evidence' link and by validator to confirm narrative claims trace back to real data.",
      "items": {"type": "string", "format": "uuid"},
      "minItems": 1,
      "maxItems": 100
    },
    "waste_estimate": {
      "type": ["object", "null"],
      "required": ["micros_usd", "method", "confidence"],
      "properties": {
        "micros_usd": {"type": "integer"},
        "method": {"enum": ["counterfactual_diff", "baseline_excess", "redundant_call_sum", "heuristic"]},
        "confidence": {"enum": ["high", "medium", "low"]},
        "explanation": {"type": "string", "maxLength": 280}
      }
    },
    "category": {"enum": ["detected_waste", "optimization_hypothesis"], "description": "Per codex r1: SHIP V0 ONLY WITH detected_waste rules. optimization_hypothesis rules ship behind a feature flag."}
  }
}
```

**Severity rubric**:
- `critical`: waste_estimate.confidence=high AND micros_usd > $100/week per agent
- `warn`: waste_estimate.confidence ≥ medium AND any quantified waste
- `info`: optimization_hypothesis category OR waste_estimate=null but pattern matched

**Idempotency**: `fingerprint` is the natural unique key for upserts. Re-runs of the same rule on the same time bucket UPDATE in place (refreshing metrics + evidence) rather than inserting duplicates.

### 4.1 New table: `cost_findings`

Lives in the same `spendguard_canonical` database as `canonical_events`.

```sql
CREATE TABLE cost_findings (
    finding_id          UUID PRIMARY KEY,           -- UUIDv7
    fingerprint         CHAR(64) NOT NULL,          -- SHA-256 hex from FindingEvidence (idempotency key, codex r1)
    tenant_id           UUID NOT NULL,
    detected_at         TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    rule_id             TEXT NOT NULL,              -- e.g. 'runaway_loop_v1'
    rule_version        INT NOT NULL DEFAULT 1,
    category            TEXT NOT NULL,              -- 'detected_waste' / 'optimization_hypothesis' (codex r1)
    severity            TEXT NOT NULL,              -- 'critical' / 'warn' / 'info'
    confidence          NUMERIC(3,2) NOT NULL,      -- 0.00–1.00
    -- Scope (one of these is set)
    agent_id            TEXT,                       -- denormalized from canonical_events
    run_id              TEXT,
    contract_bundle_id  TEXT,
    -- Evidence (JSONB so rules can carry rule-specific data)
    evidence            JSONB NOT NULL,
    -- Quantified impact
    estimated_waste_micros_usd  BIGINT,             -- nullable; some findings can't quantify
    sample_decision_ids         UUID[] NOT NULL,    -- pointers into canonical_events
    -- Lifecycle
    status              TEXT NOT NULL DEFAULT 'open', -- 'open' / 'dismissed' / 'fixed'
    feedback            TEXT,                        -- '👍' / '👎' / NULL
    -- Optional Tier-3 narrative (lazy-populated)
    narrative_md        TEXT,                       -- markdown
    narrative_model     TEXT,                       -- e.g. 'gpt-4o-mini-2024-07-18'
    narrative_cost_usd  NUMERIC(10,6),              -- track Tier-3 cost
    narrative_at        TIMESTAMPTZ,
    -- Audit
    created_at          TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at          TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE UNIQUE INDEX cost_findings_fingerprint ON cost_findings (tenant_id, fingerprint);  -- idempotency
CREATE INDEX cost_findings_tenant_detected ON cost_findings (tenant_id, detected_at DESC);
CREATE INDEX cost_findings_open ON cost_findings (tenant_id, severity) WHERE status = 'open';
```

### 4.2 New table: `cost_baselines`

```sql
CREATE TABLE cost_baselines (
    tenant_id           UUID NOT NULL,
    agent_id            TEXT NOT NULL,
    metric              TEXT NOT NULL,              -- 'cost_per_run', 'tokens_per_call', 'retries_per_run'
    window_days         INT NOT NULL,               -- 7 / 28
    median              NUMERIC NOT NULL,
    p95                 NUMERIC NOT NULL,
    sample_count        INT NOT NULL,
    computed_at         TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (tenant_id, agent_id, metric, window_days)
);
```

Refreshed nightly by a `baseline_refresher` worker (~10 min/night for 1k tenants).

---

## 5. Tier 1 rule library — split per codex r1 must-fix #3

Rules ship as `services/cost_advisor/rules/<category>/<rule_id>.sql` files. Each file MUST:
- Be a `SELECT` returning rows shaped to populate `cost_findings` AND emit `FindingEvidence` JSONB (per §4.0 contract)
- Document the pattern in a top-of-file comment: what it detects, **why it provably costs money** (vs. a heuristic guess), recommended fix
- Include unit tests with synthetic `canonical_events` fixtures
- Declare its `category`: `detected_waste` (provable, ships in v0) or `optimization_hypothesis` (heuristic, ships behind feature flag)

### 5.1 v0 ships ONLY `detected_waste` rules (codex r1 must-fix #3, refined r2)

These are rules where we can **mathematically demonstrate** money was wasted (paid for something with no value).

**Failure taxonomy (codex r2 must-fix #3)**: "failed" was previously overloaded. v2 defines explicit failure classes and which ones count as `detected_waste`:

| Failure class | Definition | Counts as provable waste? |
|---|---|---|
| `provider_5xx` | HTTP 500/502/503/504 from provider; no completion returned | ✅ Yes — billed but server failed |
| `provider_4xx_billed` | 400 with `usage` field (e.g. context-overflow with prompt charged) | ✅ Yes — explicitly billed for rejection |
| `provider_4xx_unbilled` | 400/401/429 with no `usage` (provider didn't charge) | ❌ No waste — no billing |
| `tool_error` | Tool returned exception; LLM had to retry | ⚠️ Conditional — only counts if downstream LLM call was billed |
| `malformed_json_response` | LLM returned unparseable JSON; framework retried | ✅ Yes — billed call with no usable output |
| `timeout_billed` | Provider returned partial response then timed out; usage field present | ✅ Yes |
| `timeout_unbilled` | Client-side timeout before any response | ❌ No waste detected |
| `retry_then_success` | Eventually succeeded after N failed attempts | ✅ Wasted = first N-1 attempts only |

`failed_retry_burn_v1` ONLY fires on the ✅ classes. ⚠️ tool_error is gated behind a confidence flag (medium, not high). The 2 ❌ classes never fire this rule.

| `rule_id` | Detects | Why provably wasted | Recommended fix |
|---|---|---|---|
| `failed_retry_burn_v1` ⭐ | Same `(run_id, prompt_hash)` retried after a `provider_5xx` / `provider_4xx_billed` / `malformed_json_response` / `timeout_billed` (per failure taxonomy above), all retries `committed_micros_usd > 0` | Sum of `committed_micros_usd` for the failed attempts; provider billed; output rejected | Cap retries; switch model on repeated 5xx |
| `runaway_loop_v1` | Same `(run_id, prompt_hash)` retried > 5 in 60s with no terminal output AND no failure-class match | Loop without completion = zero value (orthogonal to failed_retry_burn — this is the "infinite agent loop" case where each call individually succeeds but the agent never converges) | Add max_iterations cap; convergence criterion |
| `tool_call_repeated_v1` | Same `tool_name + tool_args_hash` invoked > 3 in one run, where tool is declared `idempotent: true` in contract DSL | Idempotent tool call repeated = redundant compute paid for (note: requires `idempotent` flag in contract; absent → rule does NOT fire to avoid false positives on intentionally-repeated tool calls) | Add tool result cache |
| `idle_reservation_rate_v1` | TTL'd reservations / total > 20% in 7 days, AND median TTL <= configured `min_ttl_for_finding` | Reservations expired without commit = paid contention with no spend reflected (gated on min TTL to avoid flagging short-TTL test environments) | Fix release path; tighten estimator |

### 5.1.2 Failure classification ownership (codex r3 must-fix #1)

The 8-class failure taxonomy raises the question: WHO assigns a class to a given audit event? Answer (v3 decision):

- **Owner**: `canonical_ingest` service (existing). When a `spendguard.audit.outcome` event is ingested, a new `failure_class` field is computed and persisted on the canonical event itself. This means rules read `canonical_events.failure_class` instead of doing classification at query time.
- **Classifier logic**: lives in a new module `services/canonical_ingest/src/classify.rs` with table-driven rules:
  - HTTP status + presence/absence of `usage` field in provider response → maps to one of `provider_5xx` / `provider_4xx_billed` / `provider_4xx_unbilled`
  - Tool error vs malformed_json vs timeout: matched against framework-specific signatures (LangChain `ToolException`, OpenAI Agents SDK `ToolCallError`, etc.)
  - Default fallback: `unknown` (never fires waste rules)
- **Test fixtures**: ship 30+ provider/framework response samples per class as JSON fixtures in `services/canonical_ingest/tests/fixtures/failure_classes/` — verified against real OpenAI/Anthropic/Bedrock response shapes.
- **Schema migration**: add `failure_class TEXT` column to `canonical_events`; default `NULL`; populate going forward; backfill existing rows in batch (~10 min for 1M rows per benchmark).
- **Versioning**: `classify.rs` uses a `CLASSIFIER_VERSION` constant; bumping it triggers a re-classification job for events ≤ 30 days old.

This means `failed_retry_burn_v1`'s SQL is trivial — just `WHERE failure_class IN ('provider_5xx', 'provider_4xx_billed', 'malformed_json_response', 'timeout_billed')` — and the hard logic is centralized, tested, and versioned in one place.

### 5.1.1 Incident grouping / dedup (codex r2 #4 — overlap concern)

`failed_retry_burn_v1` and `runaway_loop_v1` can fire on the same incident (a failing-then-looping agent triggers both). v0 ships a **post-rule deduplication step**:

1. After all rules run, group findings by `(tenant_id, agent_id, run_id, time_bucket)`
2. Within each group, prefer the rule with `severity=critical` over `warn`; `detected_waste` over `optimization_hypothesis`; higher `waste_estimate.micros_usd` wins on ties
3. Keep ONE finding per incident; merge `decision_refs` and `metrics` from suppressed rules into a `co_observed_rules` field on the surviving finding
4. Suppressed findings are persisted with `status='superseded'` and `superseded_by=<surviving_finding_id>` for auditability

This means a customer sees ONE actionable finding per incident, not 4 noisy ones.

### 5.2 v0.1 (behind `--enable-hypothesis-rules` flag): `optimization_hypothesis`

These are heuristics where the action is **probably** beneficial but not provable from audit data alone:

| `rule_id` | Detects | Confidence caveat |
|---|---|---|
| `prompt_completion_ratio_v1` | `prompt_tokens / completion_tokens > 50` sustained 7d | High ratio MAY indicate bloated system prompt OR may be correct for the task type. Surface, don't auto-recommend |
| `over_reservation_v1` | `avg(estimated / committed)` per agent > 5× over 7d | Estimator might be conservative for safety reasons; not all over-reservation is waste |

### 5.3 Cut from v0 (codex r1 attack on `model_burn_v1`)

`model_burn_v1` (gpt-4 used for short responses) is **removed** from v0. Rationale: completion_tokens < 50 does NOT prove a cheaper model would suffice — could be a binary classification, JSON schema extraction, or quality-critical short-form task. Without an eval signal (success rate, downstream agent satisfaction, user reaction), we cannot recommend model downgrade. Defer until we have a separate `eval_signal` data source (out of scope for v0).

### 5.4 Patterns that need Rust plugin (not pure SQL — codex r1 Q1)

The following patterns are genuinely hard or impossible to express in vanilla postgres SQL and justify a Rust plugin trait in v0 (see §11 Q1 update):

- **Cross-run causal chain**: "Run A produced output that Run B consumed and which caused Run B's loop" — requires graph traversal
- **Suppression / cooldown**: "Don't fire `failed_retry_burn` if the same agent already got it in the last 24h AND user dismissed it"
- **Multi-stage retry narrative**: "Tool call → 5xx → retry → 4xx → switch model → retry → success" sequencing across reservation lifecycle
- **Statistical sampling joins**: "For agents in cohort X, compare cost trajectory vs cohort Y over rolling 28d window with seasonality adjustment"

Plugin trait (expanded per codex r2 — v0.1 trait MUST cover these or it'll need a breaking change):
```rust
pub trait CostRule: Send + Sync {
    fn rule_id(&self) -> &'static str;
    fn rule_version(&self) -> u32;
    fn category(&self) -> Category;

    /// Fields this rule reads from canonical_events; checked at startup
    /// against schema audit (§11.5 A2). Rule fails to register if a
    /// declared field isn't present in the live schema.
    fn declared_input_fields(&self) -> &'static [&'static str];

    /// Whether this rule needs cost_baselines populated (Tier 2).
    fn requires_baselines(&self) -> bool { false }

    /// Suppression: if a finding from this rule fired within `cooldown`
    /// of a new candidate, the new one is suppressed (deduped at
    /// fingerprint level). Default 0 = no cooldown.
    fn cooldown(&self) -> std::time::Duration { Duration::ZERO }

    /// Per-tenant rate limit for THIS rule per day (e.g. cap noisy
    /// rules so one tenant can't generate 10k findings/day).
    fn per_tenant_daily_cap(&self) -> Option<u32> { None }

    /// Stable identity for dedup. Default: hash(rule_id, scope, time_bucket)
    /// matches the FindingEvidence.fingerprint computation.
    fn dedupe_key(&self, finding: &FindingEvidence) -> String { /* default impl */ }

    /// The actual evaluation. Returns 0..N findings per call.
    fn evaluate(&self, ctx: &EvaluationContext) -> Vec<FindingEvidence>;
}
```

V0 ships SQL-only `CostRule` impls (a generic `SqlCostRule` adapter wraps any `.sql` file). Trait surface above is committed-to in v0; v0.1 native-Rust plugins implement the same trait. **No trait breaking change planned through v1.0.**

**Open question (now closed)**: contributed rules license — Apache-2.0; require CLA on PR (see §11 Q3 unchanged).

---

## 6. Tier 2 baseline computation

Nightly batch job (`baseline_refresher` worker):

```sql
-- Per (tenant, agent) compute cost_per_run baseline for 7-day rolling window
INSERT INTO cost_baselines (tenant_id, agent_id, metric, window_days, median, p95, sample_count, computed_at)
SELECT
    tenant_id,
    extract_agent_id(payload) AS agent_id,
    'cost_per_run' AS metric,
    7 AS window_days,
    PERCENTILE_CONT(0.5) WITHIN GROUP (ORDER BY (payload->>'committed_micros_usd')::bigint) AS median,
    PERCENTILE_CONT(0.95) WITHIN GROUP (ORDER BY (payload->>'committed_micros_usd')::bigint) AS p95,
    COUNT(*) AS sample_count,
    NOW()
FROM canonical_events
WHERE detected_at > NOW() - INTERVAL '7 days'
  AND event_type = 'spendguard.audit.outcome'
GROUP BY tenant_id, agent_id
HAVING COUNT(*) >= 10  -- min sample size
ON CONFLICT (tenant_id, agent_id, metric, window_days) DO UPDATE
    SET median = EXCLUDED.median,
        p95 = EXCLUDED.p95,
        sample_count = EXCLUDED.sample_count,
        computed_at = EXCLUDED.computed_at;
```

Outlier detection (a separate finding rule):

```sql
INSERT INTO cost_findings (rule_id, severity, confidence, agent_id, evidence, estimated_waste_micros_usd, sample_decision_ids, ...)
SELECT
    'baseline_outlier_v1',
    'warn',
    0.85,
    a.agent_id,
    jsonb_build_object(
        'this_week_cost_usd', a.this_week_cost,
        'baseline_p95_usd', b.p95,
        'multiplier', a.this_week_cost / NULLIF(b.p95, 0)
    ),
    GREATEST(0, a.this_week_cost - b.p95) AS estimated_waste_micros_usd,
    a.sample_decisions
FROM (this-week-aggregation a) JOIN cost_baselines b USING (tenant_id, agent_id, metric)
WHERE a.this_week_cost > b.p95 * 3;  -- > 3× p95 = outlier
```

---

## 7. Tier 3 LLM narrative

### 7.1 Trigger
Two modes:
- **On-demand**: user opens the Cost Advisor dashboard → if `narrative_md IS NULL` for an open finding, lazily generate it. Cap: max 1 narrative per finding per 24h.
- **Daily digest**: opt-in cron job generates narratives for all unread findings of severity ≥ `warn`, packages into a digest email/Slack.

### 7.1.5 Structured output + server-side number rendering (codex r1 #2 + r2 #2)

**Round 2 escalation**: regex-based numeric token extraction misses scientific notation, written-out numbers, localized formats (中文 / 一萬二), ranges, and currency words. v2 changes the contract: **the LLM does NOT emit numeric strings at all**. Every number in the rendered narrative is template-substituted server-side from the `cited_metrics` array (which is itself a subset of `FindingEvidence.metrics`). The LLM emits `{{metric:total_waste}}` placeholders; the validator + renderer resolves them.

The narrative is generated via gpt-4o-mini's **structured output mode** (JSON schema enforced by the API). Free-form prose is rejected. Schema:

```jsonschema
{
  "title": "FindingNarrative",
  "type": "object",
  "required": ["summary_template", "root_cause_template", "recommended_action_template", "contract_dsl_yaml", "cited_metric_names"],
  "properties": {
    "summary_template": {"type": "string", "maxLength": 200, "description": "ONE sentence with PLACEHOLDERS like {{metric:total_waste_usd}} for numbers. NO numeric tokens allowed. Validator rejects digits in this string."},
    "root_cause_template": {"type": "string", "maxLength": 200, "description": "ONE sentence; placeholders allowed; no digits"},
    "recommended_action_template": {"type": "string", "maxLength": 240, "description": "ONE action; placeholders allowed; no digits; no 'consider'/'might'"},
    "contract_dsl_yaml": {"type": "string", "description": "YAML snippet using when:/then:/reason: keys; MUST compile against current contract DSL schema. Numeric thresholds in YAML are server-rendered from cited_metric_names too."},
    "cited_metric_names": {
      "type": "array",
      "items": {"type": "string"},
      "description": "Names referenced via {{metric:NAME}} placeholders. Validator: every {{metric:NAME}} in templates MUST appear here AND in FindingEvidence.metrics by name."
    }
  }
}
```

**Validation + render pipeline** (v2 — codex r2):
1. Parse JSON output (fails if not valid JSON → retry once, then fall back to no-narrative)
2. **Lex check on templates**: assert NO digit characters [0-9] anywhere in `summary_template`, `root_cause_template`, `recommended_action_template`. Reject on first violation. (This eliminates entire classes of bypass: scientific notation, written numbers like "twelve thousand", localized formats like "一萬", currency words, ranges — none can sneak in because there are no numbers in the template at all.)
3. **Placeholder resolution**: extract all `{{metric:NAME}}` tokens. For each, assert `NAME` is in `cited_metric_names` AND `cited_metric_names ⊆ FindingEvidence.metrics[*].name`.
4. **PII gate**: for each `cited_metric_names[i]`, look up the metric in `FindingEvidence.metrics`; reject if `pii_classification` is anything other than `none` or `model_id` (we don't send PII to the LLM in any form, but this also catches accidentally-cited PII).
5. Compile `contract_dsl_yaml` against the current contract DSL schema (`spendguard_contract_dsl::compile()`); reject if it doesn't compile.
6. Reject if any banned phrase appears in any string field (`consider`, `might`, `potentially`, `you may want to`, `it would be wise`).
7. **Server-side render**: substitute each `{{metric:NAME}}` in templates with formatted value from `FindingEvidence.metrics[name=NAME]` according to its `unit` (e.g. `micros_usd` → `$X.XX`, `percent` → `XX%`, `multiplier` → `X.X×`). The narrative the user sees is rendered HTML/markdown, not the LLM's raw output.

If any of these checks fail, the system retries up to 2 times. If all retries fail, **the finding is surfaced WITHOUT narrative** (no Tier-3 enrichment) — never with a low-quality narrative.

**Pre-flight validator unit tests** ship as part of P3, with fixtures for:
- Valid narrative passing
- Narrative with hallucinated dollar amount (must reject)
- Narrative with banned phrase (must reject)
- Narrative with uncompiled DSL (must reject)
- Narrative referencing percentages / ratios / multipliers (must validate via derivation)

### 7.2 Prompt template (per finding)

```
You are a senior LLM cost engineer. A customer has the following finding from
their automated spend monitor:

{evidence_json}

Write a recommendation in this exact structure:

1. ONE sentence summarizing what happened (with the specific dollar amount and
   pattern name).
2. ONE sentence on the most likely root cause.
3. ONE concrete fix the customer can apply this week.
4. A YAML snippet for the SpendGuard contract DSL that would prevent this
   pattern going forward (use `when:` / `then:` / `reason:` keys).

Be terse. No marketing language. No "consider" or "you might want to" — say
exactly what to do. Use the customer's actual numbers.
```

### 7.3 Cost budget — honest version (codex r1)

Per finding (single attempt):
- ~500 input tokens (system + evidence) + ~200 output tokens (narrative)
- gpt-4o-mini: $0.150/M input + $0.600/M output → **$0.000195/finding**

Per finding **with worst-case overhead**:
- 1 base call + up to 2 retries (validation failures) = up to 3× = **~$0.0006/finding**
- Plus dashboard re-render on user view (cached for 24h, so amortized to ~1.05× over a week) = +5% ≈ negligible
- Plus daily digest LLM summarization (1 extra call per tenant per day, ~1k tokens) = +$0.0006/tenant/day

Realistic per-tenant numbers (NOT the optimistic floor):
- **Quiet tenant** (10 findings/day, no digest, no narratives): ~$0
- **Typical tenant** (30 findings/day, narratives on, daily digest): 30 × $0.0006 + $0.0006 = **~$0.019/tenant/day = $0.57/month**
- **Heavy tenant** (200 findings/day, narratives on, hourly digest): 200 × $0.0006 + 24 × $0.0006 = **~$0.13/tenant/day = $4/month**

For 1,000 tenants average mix: **~$30–80/month** total Tier-3 inference + retry budget. Still cheap, but the v0 spec's "$0.01/tenant/day" headline was hand-wavy. The honest range is $0.005 (quiet) to $0.13 (heavy).

**Cost we're NOT counting** (and should disclose):
- Postgres CPU for nightly baseline computation (~10–60 min depending on tenant count) — runs on existing infra
- DB storage growth from `cost_findings` and `cost_baselines` tables (estimated ~10MB/tenant/month at heavy load)
- Embedding cost for Tier 4 (deferred to v2; would add ~$0.0001/decision × 10k decisions/day = $1/tenant/day at the heavy end — premium tier only)

### 7.4 Quality bar

Every narrative MUST include:
- ✅ Specific dollar amount (from evidence, not invented)
- ✅ Specific entity name (agent_id, tool name, model name)
- ✅ Specific time window
- ✅ Concrete fix (not "consider optimizing")
- ✅ Contract DSL snippet that compiles against current schema

A narrative MUST NOT:
- ❌ Use phrases: "consider", "you might want to", "potentially"
- ❌ Invent numbers not in the evidence
- ❌ Recommend tools / services outside the SpendGuard ecosystem
- ❌ Apologize or hedge ("This is just a suggestion...")

A post-generation validator (regex + JSON schema) rejects narratives that violate the bar; the system retries up to 2 times then falls back to the structured Tier 1+2 finding without narrative.

---

## 8. Surfaces (v3 — collapsed per codex r3 rescope)

| Surface | What it shows | Owner |
|---|---|---|
| **CLI** `spendguard advise --tenant X --since 7d` | Open findings + their proposed contract patches in JSON or markdown | `cost_advisor` CLI subcommand on existing `spendguard` binary |
| **Existing approval queue** (filter) | Cost-advisor-sourced proposed contract patches, alongside REQUIRE_APPROVAL items operators already review | existing `dashboard` + `control_plane` services; ONE new query filter `proposal_source IN ('cost_advisor', 'require_approval', ...)` |
| **Existing audit chain** | Approve / deny / modify decisions on cost-advisor proposals are audited the same way other contract changes are | no new infra |
| **Optional: Slack/email digest** (DEFERRED to v1.0 only if customers ask) | Daily list of unreviewed proposals | reuse existing alerting infrastructure (e.g. dashboard webhook) |

Cut from v0/v1 (per rescope):
- ❌ Standalone `/dashboard/findings` tab (was P4)
- ❌ Standalone `cost_advisor.v1.ListFindings` gRPC service (was P6)
- ❌ Python SDK methods for cost_advisor (was P6)
- ❌ Standalone `digest_dispatcher` worker (was P5)

These would have been ~10 days of build + maintenance for surface area that duplicates what `control_plane` + `dashboard` already provide.

---

## 9. Implementation phasing — v3 collapsed plan

Per codex r3 rescope: cut P4 (dashboard), P5 (digest), P6 (separate gRPC). Reuse existing approval queue.

| Phase | Scope | Estimated work |
|---|---|---|
| **P0 (prep)** | Schema reality check audit (§11.5 A2); define + ratify `FindingEvidence` schema (§4.0); proto in `proto/spendguard/cost_advisor/v1/`; `cost_findings` + `cost_baselines` migrations; integrate with `retention_sweeper`; integrate with `control_plane.proposed_contract_patches` table (extend existing schema, don't fork) | 4 days (was 3 — added control_plane integration design) |
| **P1 (skinny)** | New `services/cost_advisor/` Rust crate skeleton; `failed_retry_burn_v1` rule; CLI `spendguard advise --tenant X` returning JSON findings AND proposed contract patches; idempotent UPSERT; tenant isolation; failure classifier owned by `canonical_ingest` (see §5.1.2 below); integration tests against real benchmark fixtures | 6 days (was 5 — added classifier ownership work) |
| **P2** | Rest of Tier-1 detected_waste rules (3 more); Tier-2 baseline refresher with seasonality; outlier rule; incident-grouping/dedup phase (§5.1.1) | 5 days |
| **P3** | Tier-3 LLM narrative behind `--narrative` flag; structured output schema (§7.1.5); server-side number rendering; validation pipeline with all 5 unit-test fixtures | 4 days |
| **P3.5 (NEW)** | Wire `cost_advisor` proposals into existing `control_plane` approval queue: extend approval queue schema to include `proposal_source` enum and `proposed_dsl_patch` field; teach existing dashboard to filter by `proposal_source` | 3 days |
| **P4 (DEFERRED to v1.0 only if asked)** | Slack/email digest reusing existing alerting infra | 2 days |
| **P5** | `optimization_hypothesis` rules behind feature flag | 2 days |
| **P6 (defer to v2)** | Tier 4 embedding clustering (requires opt-in `prompt_archive` extension) | 3+ weeks |

**Critical path to v0.1**: P0 + P1 + P3 + P3.5 = **17 days**. Up from v2's 12 days because P3.5 is a new but necessary integration; net effect vs. v2 plan = wash because we cut P4/P5/P6 (~10 days saved).

**Net build savings vs. v2**: ~10 days (cut P4, P5, P6) − 5 days (added P3.5 + extra P0/P1 work for integration + classifier ownership) = **~5 days net cheaper** AND a much tighter product surface.

**Schedule contingency for A2 audit failure** (codex r3 valid concern):
- If audit reveals 1-2 missing fields → P0 + 3 days = 7 days, total = 20 days
- If audit reveals 3+ missing fields → P0 + 5 days + rule re-design → 22+ days
- Branch decision happens at end of P0; if 22+ days is unacceptable, scope reduces to ONE rule + ONE proposal type + JSON-only CLI (research preview).

---

## 10. Naming

The repo already has `services/doctor/` for the diagnostics CLI, so we cannot reuse `doctor`. Candidates:

| Name | Pros | Cons |
|---|---|---|
| **`cost-advisor`** ⭐ | Neutral, descriptive, won't get stale | Slightly bland |
| `spend-coach` | Friendly | Coach implies long-term mentoring; we're closer to one-shot recommendations |
| `bill-cfo` | Memorable; positions as "a CFO that audits your bill" | "CFO" overpromises authority |
| `agent-receipts` | Connects to existing receipts metaphor in benchmarks | Doesn't say what it does |
| `runaway-detector` | Clear, but narrow | Limits future features beyond runaway detection |

**Decision**: ship as `cost-advisor` for now (matches the file/service naming convention `services/cost_advisor/`); revisit after first design-partner feedback.

---

## 11. Open questions (for codex review)

| # | Question | Default answer if no objection |
|---|---|---|
| Q1 | Should rules be SQL-only or do we allow Python/Rust rule plugins for complex patterns (e.g. cross-run state machines)? | **REVISED post-codex-r1**: define `CostRule` trait in v0 (Rust); v0 ships only SQL-backed implementations of the trait, but the trait surface is stable so v0.1 can add native-Rust plugins (cross-run state machines, suppression/cooldown logic) without breaking changes. See §5.4 |
| Q2 | Where do narratives get persisted long-term? (`cost_findings.narrative_md` is fine for ≤ 1k findings/tenant; what about 1M?) | `narrative_md` in cost_findings; archive to S3 after 90 days |
| Q3 | License for contributed rules in `services/cost_advisor/rules/`? | Apache-2.0 like rest of repo; require CLA on PR |
| Q4 | Do we expose Tier 3 narratives in the **free tier** or paywall it? | **REVISED post-codex-r1**: free tier gets up to 5 narratives/day (so users can see the value); paid gates volume, daily digest, custom rules, retention beyond 30d, team workflows. Self-host with own OpenAI key always works without quota. The pure-paywall version was a competitive risk per codex |
| Q5 | What's the data-retention policy for `cost_findings`? | 90 days for `open`; 30 days for `dismissed` / `fixed`; auto-purge with `retention_sweeper` |
| Q6 | Could `cost-advisor` be the **first paid feature** of SpendGuard's hosted offering? | Yes — frame it as "free OSS = cap & audit; paid hosted = advice & analytics" |
| Q7 | If a rule's `evidence` includes prompt text snippets, do we redact PII? | Yes — reuse `retention_sweeper`'s redaction utilities; store only token counts + prompt_hash, not raw text |
| Q8 | Should narratives be cacheable across tenants for shared rule patterns? | No — narratives reference specific tenant data; sharing risks data leak |
| Q9 | Failure mode: if Tier-3 LLM is down, do we still surface Tier 1+2 findings? | Yes — narrative is enrichment, not gating |
| Q10 | Multi-tenant noisy-neighbor: one tenant generating 10k findings/day shouldn't starve others' Tier-3 budget. Per-tenant rate limit? | Yes — 100 narratives/tenant/day default; configurable per-tenant in control plane |

---

## 11.5 Answers to codex r1's "5 questions spec avoids"

**A1. Idempotency / unique constraint preventing duplicate findings on nightly re-runs**:
`fingerprint = SHA-256(rule_id || canonical_repr(scope) || time_bucket_iso8601)`. UNIQUE index on `(tenant_id, fingerprint)` (see §4.1). UPSERT-on-conflict updates `evidence` + `updated_at`, leaves `created_at` immutable. Time-bucket granularity per rule: e.g. `failed_retry_burn_v1` = 1-hour buckets, `idle_reservation_rate_v1` = 1-day buckets.

**A2. Are required fields actually consistently in `canonical_events.payload`?** (codex r2 contingency added)
Audit needed. v0 P0 includes a "schema reality check": run a `SELECT` against current `canonical_events` to enumerate which fields rules need (`agent_id`, `run_id`, `prompt_hash`, `tool_name`, `model_family`, etc.) and which are NULL or missing.

**Branch plan if audit reveals missing fields** (was missing in v1):

| Audit result | Action | Schedule impact |
|---|---|---|
| All required fields present + populated > 80% | Proceed to P1 as planned | 0 days |
| 1-2 fields missing OR populated < 80% | Backfill via one-time enrichment job from existing payload (e.g. compute `prompt_hash` from prompt text) | +3 days |
| 3+ fields missing OR fundamental shape mismatch | Restrict v0 rules to only fields confirmed present; defer the affected rules to v0.1; revise rule list in §5.1 | +5 days + rule re-design |
| Schema audit reveals data quality issues (PII in unexpected fields) | Pause P1; integrate `retention_sweeper` redaction as P0 prerequisite | +5-10 days |

The `CostRule.declared_input_fields()` method (§5.4) auto-checks against the audit results at startup, so a rule whose declared fields aren't present fails to register cleanly — degraded launch is mechanically supported.

**A3. How is "estimated waste" computed; what's the confidence interval?**
Per `WasteEstimate.method`:
- `redundant_call_sum` (e.g. `failed_retry_burn`): sum of `committed_micros_usd` for the failed calls. Confidence: **high** — these are paid bytes for known-failed responses.
- `counterfactual_diff` (e.g. `tool_call_repeated`): `committed_micros_usd × (n_repeated - 1) / n_repeated`. Confidence: **medium** — assumes deduped call would have produced same outcome.
- `baseline_excess` (e.g. baseline_outlier): `this_period_cost - baseline_p95`. Confidence: **medium** if sample_count ≥ 30, **low** otherwise.
- `heuristic` (optimization_hypothesis category): NULL — these rules don't claim quantified waste.

**A4. Baselines under seasonality / batch jobs / weekly spikes**:
Default baseline window = 28 days (rolling), NOT 7. Rationale: 28d covers ~4 weekly cycles. Outlier rule requires BOTH:
- This week > 3× p95 of 28d window
- Same day-of-week vs 4-week-ago day-of-week > 2× (catches Monday spikes that are normal Mondays)

For tenants with < 28 days of data: outlier rule does not fire (`info` finding only: "insufficient baseline").

**A5. Dismissal scope (rule / fingerprint / agent / tenant)**:
UI dismissal modes (per finding):
- `dismiss_this_one`: only this fingerprint, this finding, dismissed
- `dismiss_this_pattern_for_agent`: this rule_id + this agent for 30 days
- `dismiss_this_rule_for_tenant`: this rule_id, all agents, until manually re-enabled (admin-only)

Default: `dismiss_this_one`. Bulk-dismiss requires explicit confirmation. Reactivation logged in audit chain (cost_findings.audit_outbox or equivalent).

**A6. Backward compatibility on rule_version bumps** (codex r2 new concern #3):

When `failed_retry_burn_v1` evolves to `_v2`:
- **Old v1 findings remain in DB** with `rule_version=1`. They are NOT auto-migrated or re-evaluated.
- New v2 findings have a different `fingerprint` (rule_id includes the version), so they don't dedupe against v1 findings — they're treated as separate findings.
- Dashboard shows both with a `[deprecated rule_v1]` badge on old ones.
- After 90 days, v1 findings auto-archive to S3 (per retention policy). New evaluations only emit v2.
- Customers who scripted against v1 evidence shape get 90 days notice via deprecation warnings in CLI / API.

For breaking schema changes within a major version: bump rule_id (`failed_retry_burn_v2`), don't quietly mutate v1's evidence shape.

**A7. Storage strategy for cost_findings at scale (codex r3 concern)**:

Estimate: 1k tenants × 100 findings/day × 700-byte JSONB = ~70 MB/day = ~25 GB/year.

Storage plan:
- **Hot tier** (postgres): last 90 days only (≈6 GB at 1k-tenant scale). Indexed for dashboard queries.
- **Cold tier** (S3): findings older than 90 days archived as Parquet partitioned by tenant + month. Reads possible via `aws athena` for compliance / audit.
- **Partitioning**: `cost_findings` PostgreSQL-natively partitioned by `(tenant_id, range(detected_at))` monthly. Drop old partitions to S3 monthly.
- **Aggregation queries**: nightly baseline computation runs on a read replica (existing pattern in repo per `usage_poller` setup). 25 GB on a properly indexed PG read replica is well within scaling envelope; no new infra needed for this scale.
- **Beyond 10k tenants**: revisit. Likely sharded postgres or columnar store (e.g. ClickHouse for analytics layer). Out of scope for v0/v1.

**Cited metric `usd_per_week` unit alignment** (codex r3 contradiction caught in §4.0):
The §4.0 unit allowlist is `[micros_usd, usd, tokens, calls, seconds, ratio, count, percent, multiplier]`. Severity rubric uses "$100/week per agent" — this is a derived metric expressed via two cited metrics: `total_waste_usd` (unit `usd`) + `time_window_days` (unit `count`). The severity rubric is computed in code, not stored as a single metric. Fixed.

**Per-tenant rate-limit precedence** (codex r3 contradiction):
- `CostRule.per_tenant_daily_cap()` = rule's intrinsic max
- Tenant-level admin override = enforced ceiling (admin-only setting in control_plane, default = unset = use rule's cap)
- **Effective cap = MIN(rule_cap, admin_override)** if admin sets one; otherwise rule_cap applies.
- Documented in §11.5 dismissal scope (extending A5).

---

## 12. Risks & mitigations

| Risk | Likelihood | Impact | Mitigation |
|---|---|---|---|
| LLM produces hallucinated numbers in narrative | Medium | High (credibility kill) | Post-generation validator that cross-checks every dollar amount in narrative_md against evidence JSON |
| Rule library grows to 100+ rules with overlap | Medium | Medium (noise) | Rule de-duplication step before insert (same finding from multiple rules → highest-severity wins) |
| Customer rule contributions lower quality bar | Medium | Medium | PR review checklist + required test fixtures; community rules quarantined to `rules/community/` until vetted |
| Tier 3 cost runaway (a tenant generating 100k findings) | Low | Medium | Per-tenant rate limit (Q10); circuit breaker if tenant cost > $10/day |
| `cost_findings` table grows unbounded | High | Low | Retention policy (Q5) + standard `retention_sweeper` integration |
| Customers expect autonomous remediation | High | Low | Crystal-clear messaging: Cost Advisor recommends, humans (or operator policy) decides. Document this in every surface. |

---

## 13. Success criteria

For v0.1 (P1 + P3 shipped):
- [ ] At least 1 design partner runs `spendguard advise` against their real `canonical_events` and finds 1+ true-positive finding
- [ ] At least 1 narrative is rated 👍 by a real customer
- [ ] Tier 3 cost per tenant per day < $0.05 in the partner cohort
- [ ] Zero hallucinated dollar amounts (validated by post-generation validator + audit)
- [ ] At least 2 framework-specific rules ship (one for LangChain agent loop, one for OpenAI Agents SDK tool loop)

For v1.0 (all P1–P6 shipped, codex challenge round complete):
- [ ] 4 detected_waste Tier-1 rules + 1 baseline outlier rule shipping (model_burn cut per codex r1)
- [ ] 2 optimization_hypothesis rules behind feature flag
- [ ] Dashboard tab live behind feature flag
- [ ] Slack/email digest opt-in available
- [ ] At least 5 design partners using it weekly
- [ ] First community-contributed rule merged

---

## 14. Changelog

### v4 (2026-05-13, post-CA-P0 implementation + codex r5-r8)

CA-P0 prep phase shipped (branch `feat/cost-advisor-p0`, 9 commits, merged in `42cb787`). Four codex adversarial rounds on the branch: r5 RED (7 P1) → r6 RED (3 P1) → r7 RED (1 P1) → r8 **GREEN**. v4 reconciles the spec text with the verified-reality findings. §0 above summarizes all 10 corrections; details:

**§0.1 v0.1 scope cut**: zero fireable rules until P0.5 + P0.6 land (was: 4 rules). `idle_reservation_rate_v1` blocked by missing `reservations_with_ttl_status_v1` view; other 3 rules blocked by missing payload enrichment.

**§0.2 payload_json shape**: actually base64-encoded CloudEvent envelope (`data_b64`), not decoded `{kind, ...}`. Tier-2 baseline SQL in §6 must decode + join ledger.commits for cost; spec §6 example as written is wrong.

**§0.3 cost_findings idempotency**: mirror table (`cost_findings_fingerprint_keys`) + `cost_findings_upsert()` SP, not direct UNIQUE INDEX (Postgres partitioning constraint). Stale-mirror self-heal returns `outcome='reinstated'`.

**§0.4 failure_class column landed**, classifier code pending (P1).

**§0.5 §9 phasing revision**: +P0.5 (5d) +P0.6 (2d), P1 cut to 4d. New v0.1 critical path = 20 days.

**§0.6 §11.5 A2 scenario 3 triggered**: audit-report is authoritative record.

**§0.7 Retention schema-backed**: tenant_data_policy gains two retention-day columns (CA-P0 commit `e52f40a`).

**§0.8 Cross-DB FK reconciler**: 4-state drift table + 10-min grace window. Lands P1.

**§0.9 Service identity mechanism**: RLS or `pg_has_role(MEMBER)` BEFORE INSERT trigger, integration doc §5.2.1.

**§0.10 closed loop unchanged** — v3 rescope still correct.

GitHub tracking issues opened: #48 (CA-P0.5), #49 (CA-P0.6), #50 (CA-P1), #51 (CA-P1.5), #52-#56 (owner-acks Q1-Q5), #57 (this spec v4 patch).

### v1 (2026-05-13, post-codex round 1)

Codex review verdict: 5/6/4/4/5/3 across 6 dimensions. Changes:

**Must-Fix #1 (FindingEvidence contract)**: added §4.0 with full JSONSchema, fingerprint-based idempotency, severity rubric, and waste-estimate confidence levels.

**Must-Fix #2 (validator hand-waviness)**: added §7.1.5 with structured output schema, 5-step validation pipeline (JSON parse → metric cross-check → numeric token extraction → DSL compile → banned phrase regex), and 5 required unit-test fixtures.

**Must-Fix #3 (mixed waste vs. hypothesis)**: split §5 into §5.1 `detected_waste` (4 rules ship in v0) and §5.2 `optimization_hypothesis` (2 rules behind feature flag). Cut `model_burn_v1` entirely (§5.3) — needs eval signal we don't have. Replaced with `failed_retry_burn_v1` (codex's recommendation).

**Cost model honesty (codex dim 2)**: §7.3 rewritten with quiet/typical/heavy tenant tiers, retry overhead, and an explicit "cost we're NOT counting" disclosure.

**Architecture taxonomy clarity (codex dim 1)**: §3 added clarification that the 4 tiers describe marginal cost ramp, not lifecycle phase. Also flagged Tier 4's prompt-retention requirement that contradicts a non-goal — Tier 4 needs explicit opt-in.

**Plugin trait (Q1 revision)**: §5.4 added explicit `CostRule` Rust trait stub; v0 ships SQL-backed impls but trait is stable.

**Free tier narrative quota (Q4 revision)**: per codex, paywalling all narratives is competitive risk. Free tier now gets 5 narratives/day; paid gates volume + digest + custom rules.

**Implementation timeline (codex dim 6)**: §9 totally redone. Original P1 = 3-5 days was wrong. Now P0 + P1 + P3 = 12 days for v0.1. Aggressive cut to 5 days = 1 rule + JSON CLI only (`v0.0.1` research preview).

**5 questions spec ducked**: new §11.5 with concrete answers to all 5 (idempotency, schema audit, waste calc methods, baseline seasonality, dismissal scope).

### v3 + r4 verdict (2026-05-13)

**Codex round 4 (attempt 2): GREEN_LIGHT_FOR_P0** ✅

- Round 1: avg 4.5/10. 3 must-fix items (FindingEvidence schema, validator, rule split). All addressed in v1.
- Round 2: avg 5.2/10. 3 must-fix items (typed metrics, server-side rendering, failure taxonomy). All addressed in v2.
- Round 3: avg 5.0/10. FUNDAMENTAL_RESCOPE: stop building parallel product. Addressed in v3 by collapsing surface area into existing approval workflow.
- Round 4: GREEN. "Coherent closed-loop MVP that reuses existing SpendGuard control-plane primitives instead of creating a parallel product surface."

Iteration converged in 4 rounds. User's "5-round max → Staff team escalation" trigger NOT activated. Spec is ready for P0 implementation per §9 phasing (17 days for v0.1).

### v3 (2026-05-13, post-codex round 3 — fundamental rescope)

Round 3 verdict: 6/5/6/5/4/4 (avg 5.0, regressed from r2's 5.2). Codex declared **FUNDAMENTAL_RESCOPE**: stop building a parallel product, integrate into SpendGuard's existing closed loop.

**Big change — product framing**:
Cost Advisor is now a *feature* of SpendGuard's closed loop: rule engine reads audit chain → emits proposed contract DSL patches → existing `control_plane` approval queue → operator approves via existing dashboard → `contract_bundle` CD pipeline → next sidecar reload enforces new contract. Eliminates ~40% of previously-planned surface area.

**Cut**: standalone `/dashboard/findings` tab, standalone `cost_advisor.v1.ListFindings` gRPC, separate Python SDK methods, standalone `digest_dispatcher` worker. (~10 days saved.)

**Added**: §1.1 closed-loop diagram; §5.1.2 failure classification owned by `canonical_ingest` service (with versioned classifier + 30+ test fixtures); §11.5 A7 storage strategy (hot postgres / cold S3 / partitioned by tenant+month); P3.5 phase to wire proposals into existing approval queue.

**Net schedule**: 17 days for v0.1 (vs v2's 12). +5 days for proper integration & classification, but net-cheaper than v2 because cut surfaces saved more.

**Open**: round 4 to validate rescope holds; if green → implement, if not → consider escalation.

### v2 (2026-05-13, post-codex round 2)

Round 2 verdict: 6/6/5/5/5/4 (avg 5.2 vs r1 4.5). 3 must-fix items + 3 new concerns addressed.

**Must-Fix #1 (metrics schema too open)**: `metrics` changed from `additionalProperties` to typed array of metric entries with `{name, value, unit, source_field, pii_classification, derivation, ci95}`. Allowed units enumerated. PII classification gates which metrics can be sent to LLM.

**Must-Fix #2 (number hallucination still possible)**: §7.1.5 fundamental redesign. The LLM no longer emits numeric strings — it emits templates with `{{metric:NAME}}` placeholders. Server-side renderer substitutes from `FindingEvidence.metrics`. Validator step 2 lexes templates and rejects ANY digit character. This eliminates whole classes of bypass (scientific notation, written numbers, localized formats, currency words) by construction.

**Must-Fix #3 ("failed" overloaded)**: §5.1 added explicit failure taxonomy with 8 classes. `failed_retry_burn_v1` ONLY fires on classes provably billed without value. `tool_error` requires confidence-medium gate. Unbilled failures never fire the rule.

**New Concern #1 (A2 contingency)**: §11.5 A2 expanded with 4-row decision matrix for what happens if schema audit reveals missing/dirty fields, plus auto-degradation via `CostRule.declared_input_fields()`.

**New Concern #2 (CostRule trait too thin)**: §5.4 trait expanded to 8 methods (rule_version, declared_input_fields, requires_baselines, cooldown, per_tenant_daily_cap, dedupe_key, evaluate). Committed-to API through v1.0.

**New Concern #3 (backward compat absent)**: §11.5 A6 added with rule_version bump policy: old findings preserved in place, deprecation warning, 90-day archive, no quiet shape mutations.

**Rule overlap (r2 dim 4)**: §5.1.1 added explicit incident-grouping / dedup phase. Findings from multiple rules on same `(tenant, agent, run, time_bucket)` collapse to one surviving finding with `co_observed_rules` field; suppressed findings persisted with `superseded_by` pointer.

### v1 (2026-05-13, post-codex round 1)

Codex review verdict: 5/6/4/4/5/3 across 6 dimensions. Changes:

**Must-Fix #1 (FindingEvidence contract)**: added §4.0 with full JSONSchema, fingerprint-based idempotency, severity rubric, and waste-estimate confidence levels.

**Must-Fix #2 (validator hand-waviness)**: added §7.1.5 with structured output schema, 5-step validation pipeline (JSON parse → metric cross-check → numeric token extraction → DSL compile → banned phrase regex), and 5 required unit-test fixtures.

**Must-Fix #3 (mixed waste vs. hypothesis)**: split §5 into §5.1 `detected_waste` (4 rules ship in v0) and §5.2 `optimization_hypothesis` (2 rules behind feature flag). Cut `model_burn_v1` entirely (§5.3) — needs eval signal we don't have. Replaced with `failed_retry_burn_v1` (codex's recommendation).

**Cost model honesty (codex dim 2)**: §7.3 rewritten with quiet/typical/heavy tenant tiers, retry overhead, and an explicit "cost we're NOT counting" disclosure.

**Architecture taxonomy clarity (codex dim 1)**: §3 added clarification that the 4 tiers describe marginal cost ramp, not lifecycle phase. Also flagged Tier 4's prompt-retention requirement that contradicts a non-goal — Tier 4 needs explicit opt-in.

**Plugin trait (Q1 revision)**: §5.4 added explicit `CostRule` Rust trait stub; v0 ships SQL-backed impls but trait is stable.

**Free tier narrative quota (Q4 revision)**: per codex, paywalling all narratives is competitive risk. Free tier now gets 5 narratives/day; paid gates volume + digest + custom rules.

**Implementation timeline (codex dim 6)**: §9 totally redone. Original P1 = 3-5 days was wrong. Now P0 + P1 + P3 = 12 days for v0.1. Aggressive cut to 5 days = 1 rule + JSON CLI only (`v0.0.1` research preview).

**5 questions spec ducked**: new §11.5 with concrete answers to all 5 (idempotency, schema audit, waste calc methods, baseline seasonality, dismissal scope).

### v0 (2026-05-13, original draft)

Initial 4-tier architecture; 6 Tier-1 rules; LLM narrative wrapper sketch; 6-phase rollout; 10 open questions.

---

## 15. Codex iteration log

Adversarial review history. Spec rounds 1-4 are in §14 changelog; implementation rounds 5-8 fire on the CA-P0 branch (`feat/cost-advisor-p0`, merged in `42cb787`).

| Round | Scope | Verdict | Major findings | Where addressed |
|---|---|---|---|---|
| r1 | spec v0 draft | 4.5/10 | 3 must-fix: FindingEvidence schema, validator, rule split | v1 (§14) |
| r2 | spec v1 | 5.2/10 | 3 must-fix: typed metrics, server-side rendering, failure taxonomy | v2 (§14) |
| r3 | spec v2 | 5.0/10 | FUNDAMENTAL_RESCOPE: stop building parallel product | v3 (§14) |
| r4 | spec v3 | **GREEN_LIGHT_FOR_P0** | "Coherent closed-loop MVP" | proceed to CA-P0 |
| r5 | CA-P0 branch | RED (7 P1) | (1) idle_reservation_rate not actually fireable — wrong column name; (2) 0038 immutability trigger wiped 0036 bundling protections; (3) CHECK lacked NOT VALID; (4) CREATE INDEX not CONCURRENT on hot canonical_events; (5) payload_json shape wrong (data_b64); (6) missed webhook_receiver emitter; (7) cross-DB FK unsafe | branch commit `39bac29` + §0 corrections |
| r6 | CA-P0 + r5 fixes | RED (3 P1) | (1) partition-creator SP TOCTOU race; (2) reconciler missed orphan-flag drift state; (3) fingerprint mirror needed transactional writer SP | branch commit `994ed74` |
| r7 | CA-P0 + r6 fixes | RED (1 P1) | cost_findings_upsert stale-mirror hole — UPDATE returns 0 rows silently if canonical was deleted | branch commit `91ea451` (self-heal via outcome='reinstated') |
| r8 | CA-P0 + r7 fixes | **GREEN** | 2 P2 doc-accuracy (RLS bypass wording; pg_has_role USAGE vs MEMBER), 2 P3 stale comments | branch commit `42cb787` (this v4 changelog) |

Iteration converged in 4 implementation rounds (8 total spec+impl). Total CA-P0 work: 9 commits, 18 files, 1688 + 86 + 33 = ~1800 line insertions across schema migrations, proto, Rust crate skeleton, integration design doc, and audit-report.

**Stopping rule met**: r8 GREEN with 0 P1 → P1 readiness gate cleared.

**Codex usage pattern**: each round used `medium` reasoning effort and a focused prompt with the specific fixes-to-verify + 2-3 new attack vectors. Earlier (r5) used `high` reasoning + comprehensive 6-thesis attack prompt; that round ran longer but produced the largest finding set. The pattern that worked: bound the diff via `/tmp/cap0-diff-r*.patch` ranging from 1449-2446 lines so codex didn't need to recursively read the whole repo.

**Memory pattern confirmed** (project memory `feedback_codex_review.md`): every spec must run multiple codex rounds in adversarial mode; irreversible decisions must trigger a second round. The CA-P0 implementation hit BOTH gates: spec-level decisions stayed locked from r1-r4 work, AND adversarial review on the implementation caught what static spec review couldn't.
