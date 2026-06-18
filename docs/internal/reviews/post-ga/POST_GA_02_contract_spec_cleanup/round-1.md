# POST_GA_02 Review Round 1

Date: 2026-06-02

Branch: `post-ga/POST_GA_02_contract_spec_cleanup`

## Reviewer Invocation

Primary AIT command attempted:

```sh
ait run --adapter codex --review-mode adversarial --base main --branch post-ga/POST_GA_02_contract_spec_cleanup --slice-doc docs/internal/slices/POST_GA_02_contract_spec_cleanup.md --review-budget deep
```

Result: local AIT rejected `--review-mode` as an unrecognized argument.

Fallback reviewer:

```sh
/Users/michael.chen/.nvm/versions/node/v24.14.1/bin/node /Users/michael.chen/.nvm/versions/node/v24.14.1/lib/node_modules/@openai/codex/bin/codex.js review --base main
```

## Findings Fixed

| Severity | Finding | Fix |
|---|---|---|
| P2 | `docs/contract-dsl-spec-v1alpha2.md` exposed a deployable `claim_amount_atomic_gt: 0` RUN_* example, which would block every positive claim. | Replaced the deployable rule with `rules: []` and marked the post-SLICE_09 shape as non-authoritative pseudo-shape. |
| P2 | `docs/tokenizer-service-spec-v1alpha1.md` described CJK Tier 3 fallback as fail-closed over-reservation. | Reworded to state CJK-heavy unknown-model traffic can under-reserve and must trigger dispatch-entry remediation. |
| P3 | `crates/spendguard-tokenizer/src/versions.rs` decoded UUIDv7 prefix `0x019180000000` as 2024-10-18. | Corrected both SLICE_03 and SLICE_04 comments to `2024-08-23T16:09:29.344Z`. |

Implementer self-review additionally fixed `services/output_predictor/src/endpoint_cache.rs` stale wording so disabled endpoints map to `EndpointCacheError::NotConfigured`, not `not_serving`.

## Verification

```sh
scripts/ga/validate-post-ga-docs.sh
git diff --check main..HEAD
bash -n scripts/verify-migrations-postgres16.sh
cargo test --manifest-path services/output_predictor/Cargo.toml classify_status_unavailable_with_not_serving_sentinel_is_not_serving
scripts/verify-migrations-postgres16.sh
rg '2024-10-18|For CJK input this intentionally over-reserves|claim_amount_atomic_gt: 0|POST-SLICE_09: replace|DISABLED UNTIL SLICE_09|reported as `not_serving`' docs/contract-dsl-spec-v1alpha2.md docs/tokenizer-service-spec-v1alpha1.md crates/spendguard-tokenizer/src/versions.rs examples/contracts/quickstart-v1alpha2.yaml services/output_predictor/src/endpoint_cache.rs
```

Results:

- POST_GA docs validation: PASS
- diff whitespace check: PASS
- migration script syntax: PASS
- targeted output_predictor test: PASS
- Postgres 16 migration apply/smoke: PASS
- stale-text grep: no matches
