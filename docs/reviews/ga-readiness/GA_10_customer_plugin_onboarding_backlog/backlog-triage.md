# GA 10 Backlog Triage

Date: 2026-06-01

Branch: `ga/GA_10_customer_plugin_onboarding_backlog`

Source command:

```bash
gh issue list --repo m24927605/agentic-spendguard --limit 120 --state open
gh issue list --repo m24927605/agentic-spendguard --limit 120 --state all --json number,title,state,labels,updatedAt,url
```

## Staff+ Triage Policy

The GA_10 panel used these buckets:

| Bucket | Meaning |
|---|---|
| GA_10 closure | Already fixed by committed hardening work, duplicate, or historical process issue. Close with commit/test evidence in this slice. |
| GA-after named slice | Real issue, not a GA blocker, assigned to an explicit post-GA implementation slice. |
| Roadmap | Product or quality improvement that does not affect the current GA safety bar. |
| Closed before GA_10 | Already closed before this slice. |
| Not present | Issue number in #85-#177 range was not present in GitHub issue results. |

No open P1 production blocker remained in the #85-#177 set at triage
time. The remaining open items are P2/P3 hardening, performance,
documentation, or roadmap work. Customer-critical plugin onboarding
requirements are covered by:

- `docs/customer/plugin-onboarding.md`
- `docs/customer/plugin-certification-checklist.md`
- `docs/customer/plugin-error-taxonomy.md`
- `contrib/output_predictor_template/conformance_test.py`

## Named Post-GA Slices

| Slice | Scope | Issues |
|---|---|---|
| POST_GA_01_ledger_replay_semantics | Release reservation replay/fencing/status semantics. | #85, #86, #87 |
| POST_GA_02_contract_spec_cleanup | Documentation/spec title and wording drift. | #91, #93, #97, #99, #101, #113, #121, #123, #131, #136, #147, #154, #158, #159, #167, #177 |
| POST_GA_03_tokenizer_runtime_hardening | Tokenizer readiness, rate limiting, request IDs, UDS docs, parity, security, partition/retention, and tests. | #92, #94, #96, #98, #100, #103, #105, #110, #111, #112, #114, #115, #117, #118, #119, #126, #127, #129, #133, #135, #146, #148, #149, #151, #152 |
| POST_GA_04_tokenizer_asset_performance | Tokenizer asset size, dispatch performance, duplication cleanup, encoder bench expansion. | #102, #104, #108, #109, #116, #120, #122, #125, #130, #134, #140 |
| POST_GA_05_provider_coverage | Tier 1 provider client expansion and envelope tuning. | #139 |
| POST_GA_06_stats_drift_hygiene | Prediction drift alert source/dedup/cooldown and NaN guard. | #157, #162 |
| POST_GA_07_predictor_api_evolution | Output predictor response/policy shape and per-tenant Predict API rate limits. | #161, #165 |
| POST_GA_08_db_index_and_rls_polish | Output cache index cardinality, nil UUID sentinel, advisory-lock runbook, migration hardening. | #128, #163, #164, #166 |
| POST_GA_09_strategy_c_resilience | Strategy C stale-cache, herd control, input caps, reset audit enrichment, and reason caps. | #172, #173, #174, #175, #176 |
| POST_GA_10_test_quality | Cross-check fixtures and remaining smoke-test improvements. | #107, #124, #126, #129 |

Notes:

- #107 is listed in `POST_GA_10_test_quality` only as historical
  lineage. The unused tonic gzip feature was already removed by
  HARDEN_05 and is closed during GA_10.
- #128 is kept in `POST_GA_08_db_index_and_rls_polish` only as lineage.
  The current migration is schema-qualified and is closed during GA_10.

## GA_10 Closure Set

| Issue | Disposition | Evidence |
|---|---|---|
| #106 | Close as resolved. | `services/tokenizer/src/main.rs` installs `rustls::crypto::aws_lc_rs::default_provider().install_default()` before TLS use; HARDEN_05 notes record the invariant. |
| #107 | Close as resolved. | `rg` shows tokenizer `tonic` uses `tls, transport` only; HARDEN_05 removed unused gzip feature flags. |
| #128 | Close as resolved. | `services/ledger/migrations/0050_tokenizer_versions_slice04_seed.sql` uses `INSERT INTO public.tokenizer_versions` and `FROM public.tokenizer_versions`. |
| #138 | Close as resolved. | `services/tokenizer/src/shadow/security.rs` and `services/control_plane/migrations/0004_tokenizer_shadow_security_settings.sql` enforce per-tenant provider quota. |
| #142 | Close as resolved. | Missing tenant shadow security settings default to `pii_shadow_enabled=false`; provider keys alone cannot send raw text. |
| #144 | Close as resolved. | Canonical ingest replay dedup reserves `(producer_id, event_id)` and globally reserves `event_id`; tokenizer drift alerts route through canonical ingest. |
| #153 | Close as resolved. | GA_09 scan evidence passes `production_values_no_plaintext_db` and `production_render_no_plaintext_db`; tokenizer DB URLs render through `secretKeyRef`. |
| #155 | GA_10 closure. | Historical scope-creep process issue is captured here; no runtime defect. Conformance and GA_10 docs validation provide slice evidence. |
| #170 | GA_10 closure. | Duplicate tracker for #157. #157 remains the canonical implementation issue under `POST_GA_06_stats_drift_hygiene`. |

## Issue Coverage #85-#177

| Issue | State at triage | Bucket | Disposition |
|---|---|---|---|
| #85 | OPEN | GA-after named slice | POST_GA_01_ledger_replay_semantics. |
| #86 | OPEN | GA-after named slice | POST_GA_01_ledger_replay_semantics. |
| #87 | OPEN | GA-after named slice | POST_GA_01_ledger_replay_semantics. |
| #88 | Not present | Not present | No GitHub issue returned for this number. |
| #89 | Not present | Not present | No GitHub issue returned for this number. |
| #90 | CLOSED | Closed before GA_10 | P1 Python SDK proto regen closed before GA_10. |
| #91 | OPEN | GA-after named slice | POST_GA_02_contract_spec_cleanup. |
| #92 | OPEN | GA-after named slice | POST_GA_03_tokenizer_runtime_hardening. |
| #93 | OPEN | GA-after named slice | POST_GA_02_contract_spec_cleanup. |
| #94 | OPEN | GA-after named slice | POST_GA_03_tokenizer_runtime_hardening. |
| #95 | OPEN | Roadmap | POST_GA_04_tokenizer_asset_performance; size optimization, not GA blocker. |
| #96 | OPEN | GA-after named slice | POST_GA_03_tokenizer_runtime_hardening. |
| #97 | OPEN | GA-after named slice | POST_GA_02_contract_spec_cleanup. |
| #98 | OPEN | GA-after named slice | POST_GA_03_tokenizer_runtime_hardening. |
| #99 | OPEN | GA-after named slice | POST_GA_02_contract_spec_cleanup. |
| #100 | OPEN | GA-after named slice | POST_GA_03_tokenizer_runtime_hardening. |
| #101 | OPEN | GA-after named slice | POST_GA_02_contract_spec_cleanup. |
| #102 | OPEN | Roadmap | POST_GA_04_tokenizer_asset_performance. |
| #103 | OPEN | GA-after named slice | POST_GA_03_tokenizer_runtime_hardening. |
| #104 | OPEN | Roadmap | POST_GA_04_tokenizer_asset_performance. |
| #105 | OPEN | GA-after named slice | POST_GA_03_tokenizer_runtime_hardening. |
| #106 | OPEN | GA_10 closure | Resolved by HARDEN_05 rustls provider install. |
| #107 | OPEN | GA_10 closure | Resolved by HARDEN_05 gzip feature removal. |
| #108 | OPEN | Roadmap | POST_GA_04_tokenizer_asset_performance. |
| #109 | OPEN | Roadmap | POST_GA_10_test_quality. |
| #110 | OPEN | GA-after named slice | POST_GA_03_tokenizer_runtime_hardening. |
| #111 | OPEN | GA-after named slice | POST_GA_03_tokenizer_runtime_hardening. |
| #112 | OPEN | GA-after named slice | POST_GA_03_tokenizer_runtime_hardening. |
| #113 | OPEN | GA-after named slice | POST_GA_02_contract_spec_cleanup. |
| #114 | OPEN | GA-after named slice | POST_GA_03_tokenizer_runtime_hardening. |
| #115 | OPEN | GA-after named slice | POST_GA_03_tokenizer_runtime_hardening. |
| #116 | OPEN | Roadmap | POST_GA_04_tokenizer_asset_performance. |
| #117 | OPEN | GA-after named slice | POST_GA_03_tokenizer_runtime_hardening. |
| #118 | OPEN | GA-after named slice | POST_GA_03_tokenizer_runtime_hardening. |
| #119 | OPEN | GA-after named slice | POST_GA_03_tokenizer_runtime_hardening. |
| #120 | OPEN | Roadmap | POST_GA_04_tokenizer_asset_performance. |
| #121 | OPEN | GA-after named slice | POST_GA_02_contract_spec_cleanup. |
| #122 | OPEN | Roadmap | POST_GA_04_tokenizer_asset_performance. |
| #123 | OPEN | GA-after named slice | POST_GA_02_contract_spec_cleanup. |
| #124 | OPEN | Roadmap | POST_GA_10_test_quality. |
| #125 | OPEN | Roadmap | POST_GA_04_tokenizer_asset_performance. |
| #126 | OPEN | GA-after named slice | POST_GA_03_tokenizer_runtime_hardening. |
| #127 | OPEN | GA-after named slice | POST_GA_03_tokenizer_runtime_hardening. |
| #128 | OPEN | GA_10 closure | Current migration is schema-qualified. |
| #129 | OPEN | GA-after named slice | POST_GA_03_tokenizer_runtime_hardening. |
| #130 | OPEN | Roadmap | POST_GA_04_tokenizer_asset_performance. |
| #131 | OPEN | GA-after named slice | POST_GA_02_contract_spec_cleanup. |
| #132 | CLOSED | Closed before GA_10 | Dispatch RegexSet conversion closed before GA_10. |
| #133 | OPEN | GA-after named slice | POST_GA_03_tokenizer_runtime_hardening. |
| #134 | OPEN | Roadmap | POST_GA_04_tokenizer_asset_performance. |
| #135 | OPEN | GA-after named slice | POST_GA_03_tokenizer_runtime_hardening. |
| #136 | OPEN | GA-after named slice | POST_GA_02_contract_spec_cleanup. |
| #137 | CLOSED | Closed before GA_10 | P1 control-plane persistence closed before GA_10. |
| #138 | OPEN | GA_10 closure | Resolved by HARDEN_05 per-tenant count_tokens quota. |
| #139 | OPEN | Roadmap | POST_GA_05_provider_coverage. |
| #140 | OPEN | Roadmap | POST_GA_04_tokenizer_asset_performance. |
| #141 | OPEN | GA-after named slice | POST_GA_02_contract_spec_cleanup. |
| #142 | OPEN | GA_10 closure | Resolved by HARDEN_05 tenant raw-text opt-in default deny. |
| #143 | CLOSED | Closed before GA_10 | P1 verify-chain admission closed before GA_10. |
| #144 | OPEN | GA_10 closure | Resolved by HARDEN_05 canonical ingest replay dedup. |
| #145 | CLOSED | Closed before GA_10 | P1 plaintext DB URL closed before GA_10. |
| #146 | OPEN | GA-after named slice | POST_GA_08_db_index_and_rls_polish; defense-in-depth revoke. |
| #147 | OPEN | GA-after named slice | POST_GA_02_contract_spec_cleanup. |
| #148 | OPEN | GA-after named slice | POST_GA_03_tokenizer_runtime_hardening. |
| #149 | OPEN | GA-after named slice | POST_GA_03_tokenizer_runtime_hardening. |
| #150 | CLOSED | Closed before GA_10 | P1 pg_indexes filter closed before GA_10. |
| #151 | OPEN | GA-after named slice | POST_GA_03_tokenizer_runtime_hardening. |
| #152 | OPEN | GA-after named slice | POST_GA_03_tokenizer_runtime_hardening. |
| #153 | OPEN | GA_10 closure | Resolved by GA_09 plaintext DB URL scan and production Secret rendering. |
| #154 | OPEN | GA-after named slice | POST_GA_02_contract_spec_cleanup. |
| #155 | OPEN | GA_10 closure | Historical process issue recorded by this triage. |
| #156 | OPEN | Roadmap | Serialization concern; not customer-critical GA blocker. |
| #157 | OPEN | GA-after named slice | POST_GA_06_stats_drift_hygiene. |
| #158 | OPEN | GA-after named slice | POST_GA_02_contract_spec_cleanup. |
| #159 | OPEN | GA-after named slice | POST_GA_02_contract_spec_cleanup. |
| #160 | CLOSED | Closed before GA_10 | P1 integration tests closed before GA_10. |
| #161 | OPEN | GA-after named slice | POST_GA_07_predictor_api_evolution. |
| #162 | OPEN | GA-after named slice | POST_GA_06_stats_drift_hygiene. |
| #163 | OPEN | GA-after named slice | POST_GA_08_db_index_and_rls_polish. |
| #164 | OPEN | GA-after named slice | POST_GA_08_db_index_and_rls_polish. |
| #165 | OPEN | GA-after named slice | POST_GA_07_predictor_api_evolution. |
| #166 | OPEN | GA-after named slice | POST_GA_08_db_index_and_rls_polish. |
| #167 | OPEN | GA-after named slice | POST_GA_02_contract_spec_cleanup. |
| #168 | CLOSED | Closed before GA_10 | P1 AppendEvents envelope closed before GA_10. |
| #169 | CLOSED | Closed before GA_10 | P1 sidecar mirror population closed before GA_10. |
| #170 | OPEN | GA_10 closure | Duplicate of #157; close with this triage evidence. |
| #171 | CLOSED | Closed before GA_10 | P1 per-tenant SVID cert minting closed before GA_10. |
| #172 | OPEN | GA-after named slice | POST_GA_09_strategy_c_resilience. |
| #173 | OPEN | GA-after named slice | POST_GA_09_strategy_c_resilience. |
| #174 | OPEN | GA-after named slice | POST_GA_09_strategy_c_resilience. |
| #175 | OPEN | GA-after named slice | POST_GA_09_strategy_c_resilience. |
| #176 | OPEN | GA-after named slice | POST_GA_09_strategy_c_resilience. |
| #177 | OPEN | GA-after named slice | POST_GA_02_contract_spec_cleanup. |

## Panel Decisions

| Role | Decision | Outcome |
|---|---|---|
| Customer Plugin/Backend Architect | Customer onboarding must be certifiable without private maintainer knowledge. | Added onboarding guide, checklist, and template README certification path. |
| Security Engineer | SVID/mTLS and tenant isolation are hard fail criteria. | Checklist and taxonomy treat SVID mismatch as fail-closed and security-incident evidence. |
| SRE/Operations Architect | Plugin failures need operator action mapping. | Error taxonomy maps every Strategy C metric label to customer/operator actions. |
| Database Optimizer | Remaining DB/index/RLS polish is not customer-plugin GA blocking. | Items are grouped under named post-GA slices rather than hidden as roadmap. |
| Software Architect | Duplicate and historical issues must close with evidence, not silently disappear. | #155 and #170 are explicitly recorded in the closure set. |
