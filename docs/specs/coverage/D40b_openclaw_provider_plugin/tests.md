# D40b - Tests

## 1. Unit and integration tests

| ID | Test | Verifies |
|---|---|---|
| TP-D40B-01 | Public barrel exports only the factory, options types, VERSION, and typed errors. | Surface lock. |
| TP-D40B-02 | Options validation rejects missing client, tenantId, budgetId, windowInstanceId, unitId, pricing. | Day-1 tuple discipline. |
| TP-D40B-03 | Identity derivation delegates to `@spendguard/sdk`; no local hash imports. | D05 §13 invariant. |
| TP-D40B-04 | Reserve request uses `LLM_CALL_PRE`, `stepId="llm_call"`, route default `openclaw-provider`. | Wire shape. |
| TP-D40B-05 | DENY aborts before upstream provider method is called. | Fail-closed. |
| TP-D40B-06 | Sidecar outage aborts before upstream provider method is called. | Fail-closed outage. |
| TP-D40B-07 | ALLOW calls upstream once and commits SUCCESS once. | Happy path. |
| TP-D40B-08 | Provider error commits PROVIDER_ERROR and rethrows original error. | Failure settlement. |
| TP-D40B-09 | Client timeout/run abort commits CLIENT_TIMEOUT or RUN_ABORTED per `OB-V5`. | Abort settlement. |
| TP-D40B-10 | Streaming commits exactly once at terminal event; no per-chunk commits. | Streaming bracket. |
| TP-D40B-11 | Unit, window, and pricing tuple on commit match the reserve-time tuple. | HARDEN_D05_WI. |
| TP-D40B-12 | Bundle and source contain no `node:crypto`, `@noble/hashes`, `createHash`, or `blake2`. | Hash reuse. |

## 2. Acceptance tests

| ID | Command | Pass condition |
|---|---|---|
| TA-D40B-01 | package test command in `integrations/openclaw-provider-plugin` | exits 0. |
| TA-D40B-02 | package typecheck/build/size commands | exits 0; bundle <= 50 KB minified. |
| TA-D40B-03 | `make demo-down` | exits 0 before demo rerun. |
| TA-D40B-04 | `make demo-up DEMO_MODE=openclaw_provider_plugin` | prints locked 4-step success line. |
| TA-D40B-05 | `make -C deploy/demo demo-verify-openclaw-provider-plugin` | SQL hard gates pass. |
| TA-D40B-06 | docs-site build command | exits 0. |

## 3. Slice mapping

| Slice | Tests |
|---|---|
| `COV_D40B_01_plugin_package_init` | TP-D40B-01, package install/typecheck skeleton |
| `COV_D40B_02_provider_wrapper_reserve` | TP-D40B-02..06 |
| `COV_D40B_03_commit_failure_streaming` | TP-D40B-07..11 |
| `COV_D40B_04_failclosed_tests` | TP-D40B-01..12, TA-D40B-01..02 |
| `COV_D40B_05_openclaw_plugin_demo` | TA-D40B-03..05 |
| `COV_D40B_06_docs_publish` | TA-D40B-06 |
