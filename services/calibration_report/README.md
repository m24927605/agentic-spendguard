# `spendguard-calibration-report`

SLICE_13 deliverable. Operator-facing CLI that turns SpendGuard's
audit chain into actionable calibration evidence.

**Spec ancestor**: [`docs/calibration-report-spec-v1alpha1.md`](../../docs/calibration-report-spec-v1alpha1.md)
**Slice ancestor**: [`docs/slices/SLICE_13_calibration_report_cli.md`](../../docs/slices/SLICE_13_calibration_report_cli.md)

## What this CLI does

Reads SpendGuard's `canonical_events` table (or the stats_aggregator
cache for the fast path) and produces:

1. **Tokenizer tier distribution** — T1/T2/T3 hit rates over the
   window. Tier 3 > 0.1% triggers a warning (spec §8.1 rule 2).
2. **Per-(model, strategy) calibration ratio** — P50/P95/P99 of
   `actual_output_tokens / predicted_<strategy>_tokens`. The
   healthy band for Strategy B is 0.95-1.30 (spec §7.2). These exact
   ratios require `--proof-mode=canonical`; cache mode omits the
   ratio table because the cache has no predicted-token denominator.
3. **Drift alerts** — events emitted by stats_aggregator when a
   bucket's distribution drifts.
4. **Recommendations** — 9 heuristic rules covering tier burst,
   P95 outliers, plugin failure, cold-start fallback, and more
   (spec §8.1).
5. **Integrity attestation** — optional `--verify-chain` runs the
   audit-chain replay verifier inline; failure aborts the report
   with exit code 3.

## Usage

```bash
spendguard-calibration-report \
    --tenant 00000000-0000-4000-8000-000000000001 \
    --from 7d \
    --to now \
    --format text \
    --proof-mode cache \
    --canonical-url postgres://...
```

### Flags

| Flag | Default | Description |
|---|---|---|
| `--tenant <uuid>` | (required) | Tenant scope. |
| `--from <iso\|7d>` | `7d` | Window start. |
| `--to <iso\|now>` | `now` | Window end. |
| `--format <text\|json\|markdown>` | `text` | Output format. |
| `--proof-mode <cache\|canonical>` | `cache` | Source of truth. |
| `--output <-\|path>` | `-` | Output destination. |
| `--include-recommendations` | (auto) | Toggle §8 section. JSON defaults to false. |
| `--verify-chain` | `false` | Run audit-chain replay; implies canonical. |
| `--canonical-url <url>` | env | Postgres connection URL. |
| `--auth-subject <subj>` | env | mTLS subject / operator id. |
| `--auth-tenants <list>` | env | Comma-separated allowed tenant scope. |
| `--self-audit <bool>` | `true` | Emit `report_generated` CloudEvent. |

### Exit codes (spec §2.3)

- `0` — success, no critical findings.
- `1` — critical findings present (P95 > 1.50, Strategy C P95 > 1.05
  with n >= 30, Tier 3 > 0.1%, drift > 0).
- `2` — query / canonical_events unreachable / cross-tenant rejection.
- `3` — verify-chain integrity violation.

## Sample output (text format)

```text
SpendGuard Calibration Report
Tenant: 00000000-0000-4000-8000-000000000001
Window: 2026-05-22 00:00 → 2026-05-29 00:00
Proof mode: canonical (reads canonical_events directly — tamper-evident)

=== Tokenizer tier distribution ===
  Tier 2 (local exact)         :   98.5%   (985000 events)
  Tier 3 (heuristic)           :    1.5%   (15000 events)        ⚠ exceeds 0.1% target — see recommendations

=== Per-(model, strategy) calibration ratio (actual / predicted) ===
  gpt-4o                   + Strategy B:  P50= 1.04  P95= 1.18  P99= 1.34  (n=50000)  ✓ healthy
  gpt-4o                   + Strategy C:  P50= 0.98  P95= 1.05  P99= 1.12  (n=12000)  ✓ healthy

=== Drift alerts in window ===
  prediction_drift_alert events: 1
    - 2026-05-15 14:32 UTC  bucket=(gpt-4o, support-agent, chat_long)  z_score=2.4

  RUN_DRIFT_DETECTED events: 0
  RUN_BUDGET_PROJECTION_EXCEEDED events: 12  (5.0% of runs)

=== Recommendations ===
  1. [WARNING] Tier 3 hit rate 1.5% exceeds 0.1% target
     Possible cause: Unknown or unmapped model fingerprints in the tokenizer dispatch table
     Suggested action: Inspect top-N Tier 3 contributing models; PR the dispatch table

  2. [WARNING] 1 prediction_drift_alert event(s) in window
     Possible cause: Agent prompt-template change or vendor tokenizer update.
     Suggested action: Investigate cited bucket(s); consider retraining the customer plugin

Report integrity: verify-chain check NOT run.
   To validate cryptographic integrity, re-run with --verify-chain.
```

## JSON schema (v1alpha1)

```json
{
  "schema_version": "v1alpha1",
  "tenant_id": "...",
  "window": { "from": "...", "to": "..." },
  "proof_mode": "cache",
  "tier_distribution": {
    "T2": { "pct": 98.5, "count": 985000, "threshold_violation": false },
    "T3": { "pct": 1.5, "count": 15000, "threshold_violation": true }
  },
  "calibration_ratios": [ ... ],
  "drift_alerts": [ ... ],
  "run_summary": { ... },
  "recommendations": [ ... ],
  "verify_chain_run": false,
  "verify_chain_failure": null,
  "exit_code": 1
}
```

GA prereq (spec §0.3): JSON schema stability commitment for SIEM /
data warehouse consumption. SLICE_13 ships v1alpha1; downstream
consumers should pin `schema_version` and use the `exit_code` field
for batch ingestion.

## Recommendation rule set (spec §8.1)

| # | Code | Severity | Trigger |
|---|---|---|---|
| 1 | `P95_CRITICAL_OVER_1_50` | critical | any strategy P95 > 1.50 |
| 2 | `TIER3_BURST` | warning / critical | T3 pct > 0.1% / > 1.0% |
| 3 | `PREDICTION_DRIFT_ALERTS_PRESENT` | warning | drift_alerts > 0 in window |
| 4 | `STRATEGY_C_UNDER_PREDICTION` | critical | C P95 > 1.05, n >= 30 |
| 5 | `COLD_START_L1_DOMINANT` | warning | Strategy A sample share ≥ 50% |
| 6 | `STRATEGY_C_ABSENT` | warning | no C samples + > 100 non-C samples |
| 7 | `RUN_PROJECTION_EXCEEDED_HIGH` | info | RUN_BUDGET_PROJECTION_EXCEEDED > 5% of runs |
| 8 | `TIER3_KNOWN_VENDOR_FINGERPRINT` | warning | T3 + known-vendor name in scope |
| 9 | `STRATEGY_A_DOMINANT_CACHE_WARMUP` | info | A share in 50-80% band |

Per spec §8.2: every rule outputs both **possible cause** and
**suggested action**. The CLI is heuristic, not prescriptive.

## Self-audit (spec §5.3)

Every report run emits a
`spendguard.audit.calibration.report_generated.v1alpha1` CloudEvent
containing the run identity, tenant, window, format, proof_mode, and
exit code. Cross-tenant rejections emit
`spendguard.audit.calibration.unauthorized_access.v1alpha1`. Both are
fail-closed local logs by default; production deployments wire the
canonical_ingest push via the `--self-audit` opt-in path.

## Helm cron (optional)

Set `values.calibrationReport.cronEnabled=true` to schedule a daily
4:00 UTC report run. See
[`charts/spendguard/templates/calibration_report_cron.yaml`](../../charts/spendguard/templates/calibration_report_cron.yaml).

## Tests

Run the focused service suite before release:

```bash
cargo test --manifest-path services/calibration_report/Cargo.toml
```

Pass counts are intentionally not pinned in this README; use the raw
test output from the release run as the trusted evidence.
