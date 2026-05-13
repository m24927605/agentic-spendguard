# Cost Advisor — Design Spec

> **Status**: v2 (2026-05-13, post-codex round 2). v0 → v1 → v2 changelog at §14.
> **Codename**: `cost-advisor` (final brand TBD; see §10).
> **Owner**: Agentic SpendGuard
> **Closes**: post-event suggestion gap noted in `benchmarks/real-stack-e2e/REAL_LANGCHAIN_E2E.md` (the "事後建議優化" row flagged ❌)
>
> **Codex review log**: round 1 verdict 5/6/4/4/5/3 across 6 dimensions. v1 addresses all 3 must-fix items + the 5 questions the spec ducked. Defended Tier 2's role and the 4-tier cost taxonomy. Open: round 2 to verify fixes hold.

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

## 8. Surfaces

| Surface | What it shows | Owner |
|---|---|---|
| **CLI** `spendguard advise --tenant X --since 7d` | Open findings + narratives | `cost_advisor` CLI |
| **Dashboard tab** `/dashboard/findings` | Findings list, filter by severity, mark dismissed | existing `dashboard` service, new tab |
| **Slack/email digest** (opt-in) | Daily/weekly digest of new ≥ warn findings | new `digest_dispatcher` worker |
| **gRPC API** `cost_advisor.v1.ListFindings` | For embedding in customer dashboards | new proto |

---

## 9. Implementation phasing — honest timeline (codex r1)

Codex challenged the original 3–5 day P1 estimate as optimistic. Revised:

| Phase | Scope | Estimated work |
|---|---|---|
| **P0 (prep)** | Define + ratify `FindingEvidence` schema (§4.0); ship as proto in `proto/spendguard/cost_advisor/v1/`; add `cost_findings` + `cost_baselines` migrations; integrate with `retention_sweeper` for auto-purge | 3 days |
| **P1 (skinny)** | New `services/cost_advisor/` Rust crate skeleton; ONE rule (`failed_retry_burn_v1`); CLI `spendguard advise --tenant X --since 7d` returning JSON findings (no narrative); idempotent insert via fingerprint UPSERT; tenant isolation; integration test against real benchmark fixtures | 5 days |
| **P2** | Rest of Tier-1 detected_waste rules (3 more); Tier-2 baseline refresher with seasonality (7d AND 28d windows); outlier rule | 5 days |
| **P3** | Tier-3 LLM narrative behind `--narrative` flag; structured output schema (§7.1.5); validation pipeline with all 5 unit-test fixtures | 4 days |
| **P4** | Dashboard `/findings` tab; feedback (👍/👎) UI; dismissal scope-picker (per-fingerprint vs per-rule vs per-agent — codex r1 Q5) | 4 days |
| **P5** | Slack/email digest dispatcher with per-tenant rate limit (default 100 narratives/day) | 3 days |
| **P6** | gRPC API + proto + Python SDK methods; CLA enforcement on rule contributions | 3 days |
| **P7** | `optimization_hypothesis` rules behind feature flag (the 2 from §5.2) | 2 days |
| **P8 (defer to v2)** | Tier 4 embedding clustering (requires opt-in `prompt_archive` extension) | 3+ weeks |

**Revised critical path to v0.1**: P0 + P1 + P3 = **12 days**, not 5–7. Original "P1+P3 = 5–7 days" was wrong because:
- Skipped P0 (schema + migrations)
- Underestimated test fixture work (integration tests against real `canonical_events` shape)
- Underestimated tenant isolation, idempotency, and dedup edge cases

**Aggressive cut option** (if 5 days is the only budget): ship ONLY `failed_retry_burn_v1` + CLI + JSON output, no dashboard, no narrative, no digest. Customers query findings via SQL or CLI. Document this as v0.0.1 (research preview) rather than v0.1.

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
