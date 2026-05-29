# Calibration Report Specification — v1alpha1 (DRAFT)

> 📝 **Status: DRAFT** (writing in design phase on branch `design/predictor-upgrade`)
> **DRAFT → LOCKED criteria**: locks together with the predictor-upgrade spec set per `predictor-architecture-spec-v1alpha1.md` §0.2; additionally requires (a) report runs on a 7-day window with 1M+ events in ≤ 30 seconds, (b) walkthrough approval from audit / CFO / 第三方審計 reviewer profiles (per `predictor-architecture-spec-v1alpha1.md` §0.3 #6), (c) recommendation engine output reviewed by 2 design partners.
> **Companion specs (this set)**: `predictor-architecture-spec-v1alpha1.md` (umbrella; defines this as operator-facing differentiator), `audit-chain-prediction-extension-v1alpha1.md` (defines columns this report reads), `stats-aggregator-spec-v1alpha1.md` (alt consumer of cache; this report bypasses cache and reads canonical_events directly for tamper-evident proof), `tokenizer-service-spec-v1alpha1.md` (tier distribution semantics).
> **Pre-existing LOCKED dependencies**: `trace-schema-spec-v1alpha1.md` (`canonical_events` storage class; `verify-chain` CLI baseline), `ledger-storage-spec-v1alpha1.md` (canonical_events schema).
> **Compatibility policy**: alpha — CLI flags additive; output formats versioned via `--format-version`; SQL query strategy can switch between cache (fast) and canonical_events (tamper-evident) per `--proof-mode` flag.

---

## §0. Lock status & prerequisites

### 0.1 範圍

本 spec 定義 **`spendguard calibration-report` CLI**：integrated operator-facing 工具，從 `canonical_events`（per `audit-chain-prediction-extension-v1alpha1.md` §2 newly populated columns）讀 prediction metadata，輸出 calibration audit report。

涵蓋：

1. CLI 表面（subcommand + flags）
2. SQL query layer（直接讀 canonical_events 不靠 cache，為 tamper-evident proof）
3. Output formats（text / JSON / Markdown）
4. Per-tenant access control
5. Calibration metric 定義（reserved/actual ratio P50/P95/P99 等）
6. Recommendation engine 規則

**不在本 spec 範圍**：

- audit_outbox / canonical_events 新欄位 schema（推給 `audit-chain-prediction-extension-v1alpha1.md`）
- stats_aggregator cache 算法（推給 `stats-aggregator-spec-v1alpha1.md`）
- Predictor strategies（推給 `output-predictor-service-spec-v1alpha1.md`）

### 0.2 DRAFT → LOCKED criteria

進入 LOCKED 之前下列 4 項必達成：

1. SLICE 13 PR merged：CLI binary + SQL queries + 3 output formats + access control
2. CLI 對 7 天窗口 1M+ events 跑完 ≤ 30 秒
3. Walkthrough approval：1 audit reviewer + 1 CFO-profile reviewer + 1 第三方審計 reviewer
4. Recommendation engine 對 5 synthetic scenarios（healthy / drift / cold-start dominated / plugin failing / Tier 3 burst）產生正確 recommendation

### 0.3 GA prerequisites

於 `predictor-architecture-spec-v1alpha1.md` §0.3 列出。本 spec 額外要求：

1. CLI 透過 5 production tenants × 90 日 routine use 證實 report 有 operational value（operators 真的在看 + 採取 action）
2. Output JSON schema 提供 stable downstream integration（SIEM / data warehouse consumption）

### 0.4 何時可能需要 v2

- 新增 prediction strategy 改變 metric 結構
- 加 real-time dashboard 替代 CLI（觸發 v2 因為從一次性 binary 變 service）

---

## §1. Context (self-contained)

### 1.1 為什麼有這份 spec

per `predictor-architecture-spec-v1alpha1.md` §1.4：

> **Audit chain 是 calibration 證據**；所有 prediction 欄位都進 audit chain，被簽章、被 replicated、被 `verify-chain` replay。

但這個證據如果沒有 operator-friendly surface，產品承諾「calibration-grade audit」就只是 raw data 在 Postgres。`calibration-report` CLI 是把 raw audit 變成可讀證據的關鍵元件 —— 這是 SpendGuard 對 competitor 的核心 differentiator surface（無人有這個 CLI）。

### 1.2 三類 audience

| Reader | 看 report 的目的 | 想找的訊號 |
|---|---|---|
| Platform operator | 日常 health monitoring | Tier 3 hit rate、drift alerts、plugin error rate |
| CFO / FinOps | 月度 budget audit | 各 (model, strategy) 的 reserved/actual ratio、overspend 風險 |
| 第三方審計 / 規範 | 合規證明（SOX / FedRAMP / FINRA） | Cryptographic chain integrity + reservation discipline 證據 |

CLI 必須對三類 audience 都 actionable。

### 1.3 為什麼讀 canonical_events 不靠 cache

- cache（stats_aggregator）可能 stale
- cache 不被 signed（每個 row 在 cache 是 derived data，不在 audit chain 內）
- 規範性 reader 要 raw audit 證據，不要 trust intermediate cache

`--proof-mode` flag 切換：

- `--proof-mode=cache`（fast；default for operator daily use；幾秒）
- `--proof-mode=canonical`（tamper-evident；for audit；數十秒-數分鐘）

### 1.4 v1alpha1 核心哲學

> **Operator-facing 不是 marketing surface**；CLI 簡單到 oncall 半夜也能 run。
>
> **Cryptographic proof on demand**；`--proof-mode=canonical` 跑 `verify-chain` integration 證實 chain integrity。
>
> **Recommendation 是 heuristic**；operator 知道行動方向；不是 prescriptive。
>
> **No real-time dashboard**；CLI 一次性 binary；dashboard 是另一個產品。

---

## §2. CLI surface

### 2.1 基本用法

```bash
spendguard calibration-report \
    --tenant <tenant-id> \
    --from <iso-ts-or-relative> \
    --to <iso-ts-or-relative> \
    [--format text|json|markdown] \
    [--proof-mode cache|canonical] \
    [--output -|<path>] \
    [--include-recommendations]
```

### 2.2 Flag details

| Flag | Type | Default | Description |
|---|---|---|---|
| `--tenant` | UUID | (required) | Tenant scope（per §5） |
| `--from` | ISO timestamp or `7d`/`30d`/`1m` | `7d` | Window start |
| `--to` | ISO timestamp or `now` | `now` | Window end |
| `--format` | enum: `text` / `json` / `markdown` | `text` | Output format |
| `--proof-mode` | enum: `cache` / `canonical` | `cache` | Source of truth |
| `--output` | file path or `-` | `-` (stdout) | Output destination |
| `--include-recommendations` | bool | `false` (in `--format=json`)；`true` else | Whether to include heuristic recommendations |
| `--verify-chain` | bool | `false` | Run `verify-chain` integration; reject report if any row fails verify（implies `--proof-mode=canonical`） |

### 2.3 Exit codes

- `0` — report generated, no critical findings
- `1` — report generated, critical findings present（Tier 3 hit > 0.1%; drift alerts > 0; calibration P95 > 1.50）
- `2` — error: cannot query；canonical_events unreachable
- `3` — error: verify-chain failed（chain integrity violated）

Exit codes 讓 CI / monitoring 可直接 parse。

---

## §3. SQL query layer

### 3.1 Tier distribution query

```sql
SELECT
  tokenizer_tier,
  count(*) AS event_count,
  count(*) * 100.0 / sum(count(*)) OVER () AS pct
FROM canonical_events
WHERE tenant_id = $1
  AND event_type = 'spendguard.audit.decision'
  AND recorded_at BETWEEN $2 AND $3
GROUP BY tokenizer_tier
ORDER BY tokenizer_tier;
```

### 3.2 Per-(model, strategy) calibration ratio query

```sql
WITH paired AS (
  SELECT
    decision.cloudevent_payload->>'model' AS model,
    decision.prediction_strategy_used AS strategy,
    decision.predicted_b_tokens,
    decision.predicted_c_tokens,
    decision.predicted_a_tokens,
    outcome.actual_output_tokens
  FROM canonical_events decision
  JOIN canonical_events outcome
    ON decision.decision_id = outcome.decision_id
    AND outcome.event_type = 'spendguard.audit.outcome'
  WHERE decision.tenant_id = $1
    AND decision.event_type = 'spendguard.audit.decision'
    AND decision.recorded_at BETWEEN $2 AND $3
    AND outcome.actual_output_tokens IS NOT NULL
)
SELECT
  model,
  strategy,
  percentile_cont(0.50) WITHIN GROUP (ORDER BY actual_output_tokens / NULLIF(
    CASE strategy
      WHEN 'A' THEN predicted_a_tokens
      WHEN 'B' THEN predicted_b_tokens
      WHEN 'C' THEN predicted_c_tokens
    END, 0)) AS ratio_p50,
  percentile_cont(0.95) WITHIN GROUP (ORDER BY ...) AS ratio_p95,
  percentile_cont(0.99) WITHIN GROUP (ORDER BY ...) AS ratio_p99,
  count(*) AS sample_size
FROM paired
WHERE -- exclude null predictions per strategy
  CASE strategy
    WHEN 'A' THEN predicted_a_tokens IS NOT NULL
    WHEN 'B' THEN predicted_b_tokens IS NOT NULL
    WHEN 'C' THEN predicted_c_tokens IS NOT NULL
  END
GROUP BY model, strategy
ORDER BY model, strategy;
```

### 3.3 Drift alert count query

```sql
SELECT count(*) AS drift_alerts_in_window
FROM canonical_events
WHERE tenant_id = $1
  AND event_type = 'spendguard.prediction.drift_alert'
  AND recorded_at BETWEEN $2 AND $3;
```

### 3.4 verify-chain integration

當 `--verify-chain` flag set：

- 對 query 範圍內每筆 audit_outbox row 跑 `verify_cloudevent`（per `audit-chain-prediction-extension-v1alpha1.md` §7）
- 對每筆 row 跑 mirror consistency check（column ↔ proto field 一致；per audit-chain extension §11.2）
- 任一 row fail → CLI exit code 3 + 標記 row id

---

## §4. Output format

### 4.1 Text default

```
SpendGuard Calibration Report
Tenant: acme-corp
Window: 2026-05-01 → 2026-05-29
Proof mode: cache (use --proof-mode=canonical for tamper-evident proof)

=== Tokenizer tier distribution ===
  Tier 1 (provider API shadow):  N/A (off hot path)
  Tier 2 (local exact):  98.5%
  Tier 3 (heuristic):     1.5%        ⚠ exceeds 0.1% target — see recommendations

=== Per-(model, strategy) calibration ratio (reserved / actual) ===
  gpt-4o + Strategy A:     P50=2.14  P95=4.32  P99=8.10   (ceiling; expected high ratio)
  gpt-4o + Strategy B:     P50=1.04  P95=1.18  P99=1.34   ✓ healthy
  gpt-4o + Strategy C:     P50=0.98  P95=1.05  P99=1.12   ✓ excellent
  claude-3-5-sonnet + B:   P50=1.02  P95=1.11  P99=1.22   ✓ healthy
  gpt-4o-mini + A (cold):  P50=2.13  P95=4.02  P99=8.50   (cold start; expected)

=== Drift alerts in window ===
  prediction_drift_alert events: 3
    - 2026-05-15 14:32 UTC  bucket=(gpt-4o, support-agent, chat_long)  z_score=2.4
    - 2026-05-20 09:18 UTC  bucket=(claude-3-5-sonnet, code-reviewer, code_gen)  z_score=2.1
    - 2026-05-22 11:45 UTC  bucket=(gpt-4o, support-agent, chat_long)  z_score=2.6

  RUN_DRIFT_DETECTED events: 0
  RUN_BUDGET_PROJECTION_EXCEEDED events: 12  (per-run projection caught stuck loops)

=== Recommendations ===
  1. Tier 3 hit rate 1.5% exceeds 0.1% target. Top contributing models:
     - "claude-internal-finetune-v2" (43% of Tier 3 hits)
       → Action: add dispatch entry; investigate fine-tune family fingerprint
     - "gpt-4o-custom-2024-12" (28% of Tier 3 hits)
       → Action: PR add to dispatch table (likely matches gpt-4o family)

  2. Bucket (gpt-4o, support-agent, chat_long) has repeated drift alerts.
     Possible causes:
     - Recent agent prompt template change → re-baseline expected
     - Vendor tokenizer update → check tokenizer_t1_samples for matching window

  3. Strategy C calibration excellent (P95=1.05). Consider gradually
     adopting EMPIRICAL_RUN_CEILING policy for non-regulated tenants.

Report integrity: Audit chain verify-chain check NOT run.
   To validate cryptographic integrity, re-run with --verify-chain.
```

### 4.2 JSON format

```json
{
  "tenant_id": "...",
  "window": { "from": "...", "to": "..." },
  "proof_mode": "cache",
  "tier_distribution": {
    "T2": { "pct": 98.5, "count": 985000 },
    "T3": { "pct": 1.5, "count": 15000, "threshold_violation": true }
  },
  "calibration_ratios": [
    { "model": "gpt-4o", "strategy": "B", "p50": 1.04, "p95": 1.18, "p99": 1.34, "sample_size": 50000 },
    ...
  ],
  "drift_alerts": [
    { "ts": "2026-05-15T14:32:00Z", "bucket": "...", "z_score": 2.4 }
  ],
  "recommendations": [
    { "severity": "warning", "code": "TIER3_BURST", "details": {...} }
  ],
  "verify_chain_run": false
}
```

### 4.3 Markdown format

近似 text 但加 markdown 標記，適合貼 Slack / Confluence / GitHub Issue。

---

## §5. Per-tenant access control

### 5.1 Authentication

CLI 認證模式：

- 開發 / demo：`SPENDGUARD_AUTH_TOKEN` env var
- Production：mTLS client cert（per Sidecar §5 internal transport）

### 5.2 Tenant scope check

每次 query 對 `--tenant` flag tenant_id 與 caller 認證的 identity 對照：

- Single-tenant deploy：caller 必須是該 tenant 的 admin role
- Multi-tenant deploy（SaaS）：caller 必須有該 tenant scope（per control plane RBAC）

跨 tenant query → exit code 2 + audit log。

### 5.3 Audit log

每次 CLI run 自身 emit `spendguard.calibration.report_generated` CloudEvent，記錄：

- 跑 report 的 user identity
- tenant_id + 時間範圍
- exit code + summary

Signed + immutable per audit chain。確保「誰看了 calibration report」也有 trail。

---

## §6. Sample report

per HANDOFF §3.6 範例已在 §4.1 verbatim 重現。

實際輸出將包含具體 customer 數據；範例是 reference shape。

---

## §7. Calibration metric 定義

### 7.1 Reserved / actual ratio

```
ratio = predicted_strategy_tokens / actual_output_tokens
```

P50 / P95 / P99 計算 over 所有 paired (decision, outcome) rows。

- ratio > 1.0：reserved 超過 actual（over-reservation；浪費 budget 但不 unsafe）
- ratio < 1.0：reserved 少於 actual（under-reservation；可能觸發 BUDGET_EXHAUSTED 或 overrun debt）

### 7.2 預期 ratio 分布

| Strategy | Expected P50 | Expected P95 | Healthy ratio |
|---|---|---|---|
| A (ceiling) | > 2.0 | > 4.0 | A 是 ceiling；高 ratio 正常 |
| B (P95 lookup) | 0.95–1.15 | 1.10–1.30 | Calibrated；窄 distribution |
| C (plugin) | 0.95–1.15 | 1.05–1.20 | Tightest（客戶自訓有 advantage） |

突破 healthy 範圍 → recommendation engine 觸發 alert。

### 7.3 Cold-start 影響

Cold-start L1（無 distribution）→ ratio 顯示為 A 的 expected 分布。Report 區分 `Strategy A (cold)` vs `Strategy A (no cold)` 為了讓 reader 看出哪些 audit row 是 cold-start fallback。

---

## §8. Recommendation engine

### 8.1 Heuristic rules

| 觸發條件 | Severity | Recommendation |
|---|---|---|
| Tier 3 hit rate > 0.1% | Warning | List top 5 contributing models; suggest dispatch table PR |
| Tier 3 hit rate > 1.0% | Critical | Same + page on-call |
| Strategy B P95 ratio > 1.30 over 7 days | Warning | Suggest reviewing prompt class definitions or refreshing stats_aggregator baseline |
| Strategy B P95 ratio > 1.50 over 7 days | Critical | Suggest investigating systematic agent behavior change |
| Strategy C error rate > 5% over 7 days | Warning | List customer plugin error reasons; suggest plugin maintenance |
| Strategy C P95 < 0.95 (under-prediction) | Critical | Suggest plugin retraining (risky territory) |
| RUN_BUDGET_PROJECTION_EXCEEDED rate > 5% of runs | Info | Suggest reviewing per-run budget caps |
| RUN_DRIFT_DETECTED rate > 1% of runs | Warning | Suggest reviewing agent stability |
| Tier 1 drift_alert count > 1 in window | Info | Vendor tokenizer may have updated; review |

### 8.2 Recommendations 是 heuristic, not prescriptive

CLI 永遠展示 「Possible cause / Suggested action」 兩段，operator 知道行動方向；不直接 emit "do this"。

### 8.3 Recommendation 不進 audit chain

Recommendations 是 derived 從 stable audit data；本身不需要 audit chain 條目（避免 recursive audit）。CLI 自身的 audit log（per §5.3）只記 report run metadata。

---

## §9. Failure modes

| 場景 | 行為 |
|---|---|
| canonical_events 不可達 | exit code 2 + clear error message |
| Tenant scope mismatch | exit code 2 + audit log 記嘗試 |
| Window too large (event count > 100M) | warn user; suggest 縮小 window；仍跑但可能超 30s |
| verify-chain fail | exit code 3 + 標記 row + 整 report aborted |
| JSON parse fail on cloudevent_payload | skip row + emit metric `report_skipped_rows` |

---

## §10. GA prerequisites

於 `§0.3` 列出。本 spec 不重複。

---

## §11. Adoption history

| Round | Reviewer | 採納率 | 主要產出 |
|---|---|---|---|
| (placeholder) | (placeholder) | (placeholder) | (placeholder — filled during Codex / panel adversarial review rounds per HANDOFF §9) |

---

## §12. Lock 後的下一步

1. SLICE 13 PR：CLI binary + SQL queries + 3 output formats + access control + recommendation engine + verify-chain integration
2. SLICE 13 acceptance：5 synthetic scenarios recommendation correctness + walkthrough approval
3. JSON schema stabilization for downstream SIEM integration
4. Optional dashboard surface deferred to separate slice（post-launch）

---

*Document version: calibration-report-spec-v1alpha1 (DRAFT) | Drafted: 2026-05-29 | Critical surface: §2 CLI flags;  §3 SQL queries;  §4.1 sample text output;  §7 metric definitions;  §8 recommendation rules | The operator-facing differentiator no competitor ships | Branch: `design/predictor-upgrade`*
