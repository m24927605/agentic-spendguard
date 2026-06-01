# POST_GA_02 Review Round 5 and Staff+ Arbitration

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

## Round 5 Findings

| Severity | Finding | Staff+ decision | Fix |
|---|---|---|---|
| P2 | `docs/operations/runbooks/tokenizer-key-rotation.md` ran `kubectl port-forward` in the foreground, so the following `curl` command could not execute in the same shell. | Fix anyway. All panel members treated the runbook as operationally broken and in-scope because it was introduced by this slice. | Start `port-forward` in the background, capture `PF_PID`, install cleanup with `trap`, run the metrics check, then kill the background process and clear the trap. |
| P3 | `examples/contracts/quickstart-v1alpha2.yaml` still said the sample asks for human approval on projected overruns even though the projection block is commented and non-deployable. | Fix anyway. All panel members treated the intro contradiction as in-scope doc drift. | Reword the intro to say only the v1alpha1 hard-cap rule is active; the projection block is a commented future approval shape and RUN_* activation is owned by `run_cost_projector`. |

## Staff+ Panel

Round 5 still had findings, so codex review iteration stopped at the required maximum and Staff+ arbitration was dispatched through separate `ait run` invocations. Local AIT returned `exit_code=3` for these panel runs, but each run produced a usable vote and evidence in stdout.

| Panel role | Vote |
|---|---|
| Software Architect | Fix P2 anyway; preserve quickstart intent by saying the sample declares `EMPIRICAL_RUN_CEILING` and includes a commented projection-approval illustration. |
| Backend Architect | Fix both; the foreground `port-forward` blocks the runbook and the quickstart introduction contradicts the deployable sample. |
| Security Engineer | Fix both; stale quickstart wording can cause unsafe operator behavior. |
| Database Optimizer | Fix both; both are branch-introduced documentation defects. |
| SpendGuard Domain Expert | Fix both; quickstart should say only the hard-cap rule is active and projection approval is future projector-owned behavior. |

Final arbitration decision: fix both findings, do not run a sixth codex review round, and proceed to merge after verification.

## Verification

```sh
scripts/ga/validate-post-ga-docs.sh
git diff --check main..HEAD
cargo test --manifest-path services/sidecar/Cargo.toml quickstart_v1alpha2_sample_still_parses_after_round1_m2_disable
rg -n 'human approval on projected overruns|asks for|Uncomment|When SLICE_09 ships|DISABLED UNTIL SLICE_09|POST-SLICE_09: replace' docs/operations/runbooks/tokenizer-key-rotation.md examples/contracts/quickstart-v1alpha2.yaml
```

Results:

- POST_GA docs validation: PASS
- diff whitespace check: PASS
- targeted sidecar quickstart sample test: PASS
- stale-text grep: no matches
