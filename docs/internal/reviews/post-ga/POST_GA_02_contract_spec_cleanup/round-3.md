# POST_GA_02 Review Round 3

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

## Finding Fixed

| Severity | Finding | Fix |
|---|---|---|
| P2 | `docs/contract-dsl-spec-v1alpha2.md` still described the projection upgrade path as replacing disabled CEL `condition:` rules with supported `when:` thresholds. The supported thresholds cannot read projector signals, so the advice would still make every positive claim eligible for approval or blocking. | Reworded the upgrade path to state that projection CEL rules have no SLICE_02 declarative replacement and must remain disabled or removed until SLICE_09/10 `run_cost_projector` owns activation. Updated the sidecar parse error string and regression assertion to reject the same unsafe migration path. |

## Verification

```sh
scripts/ga/validate-post-ga-docs.sh
git diff --check main..HEAD
cargo test --manifest-path services/sidecar/Cargo.toml
```

Results:

- POST_GA docs validation: PASS
- diff whitespace check: PASS
- full sidecar test suite: PASS, 123 library tests and 6 integration tests
