# Cost Advisor — Design Spec

> **Status**: v0 draft (2026-05-13). To be reviewed via codex challenge cycle.
> **Codename**: `cost-advisor` (final brand TBD; see §10).
> **Owner**: Agentic SpendGuard
> **Closes**: post-event suggestion gap noted in `benchmarks/real-stack-e2e/REAL_LANGCHAIN_E2E.md` (the "事後建議優化" row flagged ❌)

---

## 1. Problem statement

After SpendGuard caps and audits LLM spend, customers still need to know:

- **What patterns are wasting money?** (e.g., runaway loops, oversized prompts, model overprovision)
- **Why?** (root-cause hypothesis, not just an alert)
- **What to do?** (concrete fix — ideally a contract DSL snippet or code change)

Today the product produces an immutable audit chain (`canonical_events`) but no analysis layer on top. Customers must:
- Write their own SQL queries against `canonical_events`
- Or buy a separate observability tool (LangSmith, Helicone, Langfuse)
- Or just guess

This is the "事後建議優化" gap. **Cost Advisor** closes it.

---

## 2. Non-goals

- **Not a real-time abort tool**. SpendGuard sidecar already does that (Tier 0 of the stack). Cost Advisor runs **after the fact** on the audit chain.
- **Not a generic LLM observability platform**. We don't store prompts, completions, or traces beyond what SpendGuard already keeps in `canonical_events`. Helicone / Langfuse are stronger here; we don't compete.
- **Not a billing reconciler**. Provider invoices reconcile via the existing `usage_poller` + `webhook_receiver` services.
- **Not autonomous remediation**. Findings recommend; humans (or operators with explicit policy) decide whether to apply.

---

## 3. Architecture — 4 layers, each independently togglable

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

### 4.1 New table: `cost_findings`

Lives in the same `spendguard_canonical` database as `canonical_events`.

```sql
CREATE TABLE cost_findings (
    finding_id          UUID PRIMARY KEY,           -- UUIDv7
    tenant_id           UUID NOT NULL,
    detected_at         TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    rule_id             TEXT NOT NULL,              -- e.g. 'runaway_loop_v1'
    rule_version        INT NOT NULL DEFAULT 1,
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

## 5. Tier 1 rule library — initial 6 rules

Rules ship as `services/cost_advisor/rules/<rule_id>.sql` files. Each file MUST:
- Be a `SELECT` returning rows shaped to populate `cost_findings` (or a thin SQL wrapper does the insert)
- Document the pattern in a top-of-file comment with: what it detects, why it costs money, recommended fix
- Include unit tests with synthetic `canonical_events` fixtures

| `rule_id` | Detects | Trigger condition | Recommended fix |
|---|---|---|---|
| `runaway_loop_v1` | Same `(run_id, prompt_hash)` retried > 5 in 60s | retry count + window | Cap retries; add error logging |
| `tool_call_repeated_v1` | Same `tool_name + tool_args_hash` invoked > 3 in one run | dedup count | Add tool result cache |
| `prompt_completion_ratio_v1` | `prompt_tokens / completion_tokens > 50` (sustained over 7 days) | rolling avg | Trim system prompt; use few-shot |
| `over_reservation_v1` | `avg(estimated / committed)` per agent > 5× over 7 days | ratio | Tighten estimator |
| `idle_reservation_rate_v1` | TTL'd reservations / total > 20% in 7 days | ratio | Loosen quota OR fix release path |
| `model_burn_v1` | gpt-4 family used for runs where `completion_tokens < 50` | model + size | Switch to gpt-4o-mini for short tasks |

**Open question**: do we enforce a license on contributed rules? Default ASL-2.0 like the rest of the repo? See §11 open question Q3.

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

### 7.3 Cost budget

Per finding: ~500 input tokens (system + evidence) + ~200 output tokens (narrative).
- gpt-4o-mini: $0.150/M input + $0.600/M output → **$0.000195/finding**.
- 100 findings/tenant/day = $0.0195/tenant/day = $0.59/tenant/month.
- 1,000 tenants = $585/month total Tier 3 inference cost.

This is well within "low cost" territory.

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

## 8. Surfaces

| Surface | What it shows | Owner |
|---|---|---|
| **CLI** `spendguard advise --tenant X --since 7d` | Open findings + narratives | `cost_advisor` CLI |
| **Dashboard tab** `/dashboard/findings` | Findings list, filter by severity, mark dismissed | existing `dashboard` service, new tab |
| **Slack/email digest** (opt-in) | Daily/weekly digest of new ≥ warn findings | new `digest_dispatcher` worker |
| **gRPC API** `cost_advisor.v1.ListFindings` | For embedding in customer dashboards | new proto |

---

## 9. Implementation phasing

| Phase | Scope | Estimated work |
|---|---|---|
| **P1** | New `services/cost_advisor/` Rust crate; `cost_findings` + `cost_baselines` tables; first 3 rules (`runaway_loop`, `tool_call_repeated`, `over_reservation`); CLI `spendguard advise --tenant X` returning structured findings (no narrative) | 3–5 days |
| **P2** | All 6 Tier-1 rules; Tier-2 baseline refresher; outlier finding rule | 2 days |
| **P3** | Tier-3 LLM narrative (opt-in flag in tenant config); on-demand only | 1–2 days |
| **P4** | Dashboard `/findings` tab; feedback (👍/👎) UI | 2 days |
| **P5** | Slack/email digest dispatcher | 2 days |
| **P6** | gRPC API + proto + Python SDK methods | 2 days |
| **P7** (defer) | Tier 4 embedding clustering (premium) | 2+ weeks |

**Critical path to v0.1**: P1+P3 = 5–7 days for an MVP that produces real narratives on real benchmark data.

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
| Q1 | Should rules be SQL-only or do we allow Python/Rust rule plugins for complex patterns (e.g. cross-run state machines)? | SQL-only for v0; revisit if a real pattern can't be expressed |
| Q2 | Where do narratives get persisted long-term? (`cost_findings.narrative_md` is fine for ≤ 1k findings/tenant; what about 1M?) | `narrative_md` in cost_findings; archive to S3 after 90 days |
| Q3 | License for contributed rules in `services/cost_advisor/rules/`? | Apache-2.0 like rest of repo; require CLA on PR |
| Q4 | Do we expose Tier 3 narratives in the **free tier** or paywall it? | Free tier: structured findings only; narratives = paid (or self-host with own OpenAI key) |
| Q5 | What's the data-retention policy for `cost_findings`? | 90 days for `open`; 30 days for `dismissed` / `fixed`; auto-purge with `retention_sweeper` |
| Q6 | Could `cost-advisor` be the **first paid feature** of SpendGuard's hosted offering? | Yes — frame it as "free OSS = cap & audit; paid hosted = advice & analytics" |
| Q7 | If a rule's `evidence` includes prompt text snippets, do we redact PII? | Yes — reuse `retention_sweeper`'s redaction utilities; store only token counts + prompt_hash, not raw text |
| Q8 | Should narratives be cacheable across tenants for shared rule patterns? | No — narratives reference specific tenant data; sharing risks data leak |
| Q9 | Failure mode: if Tier-3 LLM is down, do we still surface Tier 1+2 findings? | Yes — narrative is enrichment, not gating |
| Q10 | Multi-tenant noisy-neighbor: one tenant generating 10k findings/day shouldn't starve others' Tier-3 budget. Per-tenant rate limit? | Yes — 100 narratives/tenant/day default; configurable per-tenant in control plane |

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
- [ ] 6 Tier-1 rules + 1 baseline outlier rule shipping
- [ ] Dashboard tab live behind feature flag
- [ ] Slack/email digest opt-in available
- [ ] At least 5 design partners using it weekly
- [ ] First community-contributed rule merged
