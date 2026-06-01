# POST_GA_02 Implementation Evidence

Date: 2026-06-02

Slice: `docs/slices/POST_GA_02_contract_spec_cleanup.md`

Branch: `post-ga/POST_GA_02_contract_spec_cleanup`

## Issue Coverage

| Issue | Evidence |
|---|---|
| #91 | `docs/contract-dsl-spec-v1alpha2.md` §6.3 split into SLICE_02 wedge surface and post-SLICE_09 CEL surface. |
| #93 | `docs/contract-dsl-spec-v1alpha2.md` §8.4, `CHANGELOG.md`, and `charts/spendguard/values.yaml` document upgrade grep and `bundle_validation_failed`. |
| #97 | `docs/audit-chain-prediction-extension-v1alpha1.md` and `docs/tokenizer-service-spec-v1alpha1.md` clarify HEURISTIC marker row vs Tier 3 NULL sentinel. |
| #99 | `services/tokenizer/tests/golden_samples.rs` and `docs/slices/SLICE_03_tokenizer_service_skeleton_openai.md` say 51 samples. |
| #101 | `docs/tokenizer-service-spec-v1alpha1.md` §5.1 documents CJK over-reservation as conservative Tier 3 behavior. |
| #113 | `proto/spendguard/tokenizer/v1/tokenizer.proto` documents 1 MiB protocol cap and field-level defense-in-depth. |
| #121 | Anthropic and Cohere encoder comments now reference `discover_fixture_tokens`. |
| #123 | `crates/spendguard-tokenizer/Cargo.toml` onig justification now distinguishes Gemini from Anthropic/Cohere/Llama. |
| #131 | `crates/spendguard-tokenizer/src/versions.rs` states UUID timestamp half is decorative and decodes to 2024-10-18. |
| #136 | Gemini §7.1 disclosure is split into source/license/honest-disclosure bullets. |
| #141 | Added `docs/operations/runbooks/tokenizer-key-rotation.md`. |
| #147 | `docs/tokenizer-service-spec-v1alpha1.md` uses `tokenizer_t1_samples_alert_idx`. |
| #154 | `docs/slices/SLICE_05_tokenizer_tier1_shadow_drift.md` §13/§14 are coherent. |
| #158 | `docs/stats-aggregator-spec-v1alpha1.md` source URI matches `build_drift_alert`. |
| #159 | `docs/stats-aggregator-spec-v1alpha1.md` event type matches `PREDICTION_DRIFT_ALERT_EVENT_TYPE`. |
| #167 | `docs/stats-aggregator-spec-v1alpha1.md` pins non-negative sample-size CHECK invariants; `scripts/verify-migrations-postgres16.sh` asserts negative inserts fail. |
| #177 | Strategy C comments and plugin contract distinguish Predict `not_serving` sentinel from disabled endpoint and breaker state. |

## Acceptance Commands

```sh
scripts/ga/validate-post-ga-docs.sh
git diff --check main..HEAD
bash -n scripts/verify-migrations-postgres16.sh
cargo test --manifest-path services/output_predictor/Cargo.toml classify_status_unavailable_with_not_serving_sentinel_is_not_serving
scripts/verify-migrations-postgres16.sh
```

Results:

- `scripts/ga/validate-post-ga-docs.sh`: pass
- `git diff --check main..HEAD`: pass
- `bash -n scripts/verify-migrations-postgres16.sh`: pass
- Targeted output_predictor test: pass, 1 test
- `scripts/verify-migrations-postgres16.sh`: pass against `postgres:16-alpine`

Postgres evidence:

- image digest: `postgres@sha256:16bc17c64a573ef34162af9298258d1aec548232985b33ed7b1eac33ba35c229`
- server: PostgreSQL 16.14
- new smoke result: `sample_size_check_constraints_reject_negative = true`

## Grep Evidence

Drift alert type and source are aligned:

```text
services/stats_aggregator/src/drift_detector.rs: PREDICTION_DRIFT_ALERT_EVENT_TYPE = "spendguard.audit.prediction_drift_alert.v1alpha1"
services/stats_aggregator/src/drift_detector.rs: source = "spendguard://stats-aggregator/<tenant_id>"
docs/stats-aggregator-spec-v1alpha1.md: type: spendguard.audit.prediction_drift_alert.v1alpha1
docs/stats-aggregator-spec-v1alpha1.md: source: spendguard://stats-aggregator/<tenant_id>
```

Runtime build scope:

- No schema/proto fields changed.
- Rust edits are comments only except `scripts/verify-migrations-postgres16.sh` smoke coverage.
- Targeted output_predictor test was run because Strategy C comments/tests were touched.
