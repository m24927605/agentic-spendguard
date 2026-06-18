# POST_GA_02 Review Round 4

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
| P2 | `CHANGELOG.md` and `charts/spendguard/values.yaml` still said SLICE_09 would make `condition:` expressions valid through a CEL accessor. That contradicted the round-3 fix and could lead operators to deploy rules that fail bundle validation. | Replaced the stale accessor wording with the projector-owned activation boundary and kept the fail-closed bundle validation guidance for any lingering `condition:` field. |

## Verification

```sh
scripts/ga/validate-post-ga-docs.sh
git diff --check main..HEAD
rg -n 'SLICE_09 CEL accessor|until the SLICE_09|CEL accessor surface is active|condition:` fail to load with `bundle_validation_failed` until' CHANGELOG.md charts/spendguard/values.yaml docs/contract-dsl-spec-v1alpha2.md
```

Results:

- POST_GA docs validation: PASS
- diff whitespace check: PASS
- stale-text grep: no matches
