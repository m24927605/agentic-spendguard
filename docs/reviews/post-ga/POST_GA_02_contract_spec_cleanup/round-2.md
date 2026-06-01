# POST_GA_02 Review Round 2

Date: 2026-06-02

Branch: `post-ga/POST_GA_02_contract_spec_cleanup`

## Reviewer Invocation

Primary AIT command attempted:

```sh
ait run --adapter codex --review-mode adversarial --base main --branch post-ga/POST_GA_02_contract_spec_cleanup --slice-doc docs/slices/POST_GA_02_contract_spec_cleanup.md --review-budget deep
```

Result: local AIT rejected `--review-mode` as an unrecognized argument.

Fallback reviewer:

```sh
/Users/michael.chen/.nvm/versions/node/v24.14.1/bin/node /Users/michael.chen/.nvm/versions/node/v24.14.1/lib/node_modules/@openai/codex/bin/codex.js review --base main
```

## Findings Fixed

| Severity | Finding | Fix |
|---|---|---|
| P2 | `docs/operations/runbooks/tokenizer-key-rotation.md` used `deployment/<release>-tokenizer`, but Helm renders `<release>-spendguard-tokenizer`. | Updated rollout, status, logs, and port-forward commands to `<release>-spendguard-tokenizer`. |
| P2 | The runbook referenced non-exported metrics `tokenizer_shadow_queue_lag_seconds` and `tokenizer_provider_auth_failures_total`. | Replaced with exported counters `spendguard_tokenizer_shadow_worker_dead_total`, `spendguard_tokenizer_shadow_dropped_full_total`, `spendguard_tokenizer_provider_count_tokens_schema_drift_total`, plus log checks for `schema-drift or auth error`. |

Implementer self-review additionally removed stale quickstart wording that implied operators could uncomment a projection rule after SLICE_09; the block is now explicitly non-deployable and owned by run_cost_projector integration.

## Verification

```sh
scripts/ga/validate-post-ga-docs.sh
git diff --check main..HEAD
cargo test --manifest-path services/output_predictor/Cargo.toml classify_status_unavailable_with_not_serving_sentinel_is_not_serving
rg '<release>-tokenizer|tokenizer_shadow_queue_lag_seconds|tokenizer_provider_auth_failures_total|Uncomment it ONLY after|When SLICE_09 ships' docs/operations/runbooks/tokenizer-key-rotation.md examples/contracts/quickstart-v1alpha2.yaml
rg '<release>-spendguard-tokenizer|spendguard_tokenizer_shadow_worker_dead_total|spendguard_tokenizer_shadow_dropped_full_total|spendguard_tokenizer_provider_count_tokens_schema_drift_total' docs/operations/runbooks/tokenizer-key-rotation.md
```

Results:

- POST_GA docs validation: PASS
- diff whitespace check: PASS
- targeted output_predictor test: PASS
- stale-text grep: no matches
- Helm-name/exported-metric grep: matches expected runbook text
