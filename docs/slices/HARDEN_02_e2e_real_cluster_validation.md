# HARDEN 02 — E2E real-cluster validation

> **Branch**: `harden/HARDEN_02_e2e_real_cluster_validation`
> **Status**: draft
> **Spec ancestor(s)**: `predictor-upgrade-hardening-spec-v1alpha1.md`, `predictor-architecture-spec-v1alpha1.md`
> **Depends on prior slices**: HARDEN_01
> **Blocks subsequent slices**: HARDEN_03 issue triage uses failures found here
> **Estimated change size**: medium; mostly scripts, runbooks, and fixes discovered by real execution

---

## §0. TL;DR

Actually boot the stack and run the predictor upgrade through real docker-compose, demo modes, kind, Helm production install, and the release benchmark binary. This slice turns "static review ready" into "integration verified" and records exact command output under versioned artifacts.

---

## §1. Architectural context

`feedback_demo_quality_gate.md` is explicit: codex review is necessary but insufficient. Wire, OS, Helm, and runtime failures only appear when services boot together. HARDEN_02 validates the SLICE_01-15 system as deployed software, not as source files.

---

## §2. Scope (must-do)

- Run `bash tests/e2e/predictor_upgrade.sh` against docker-compose
- Run `make demo-up DEMO_MODE=default` and verify `tests/e2e/verify_audit_columns.py` exits 0
- Run `make demo-up DEMO_MODE=m1_benchmark_runaway_loop` and verify `RUN_BUDGET_PROJECTION_EXCEEDED` fires
- Run `make demo-up DEMO_MODE=multi_provider_usd` and verify four providers route correctly
- Run `make demo-up DEMO_MODE=agent_real_anthropic`; use mock provider mode only if no real key is present and record that explicitly
- Run `make demo-up DEMO_MODE=plugin_c_synthetic` and verify Strategy C circuit breaker behavior
- Create a kind cluster and run `helm install spendguard charts/spendguard --set chart.profile=production` with required production values
- Run `cargo run --release -p spendguard-predictor-upgrade-benchmarks` and record real benchmark numbers
- If docker or kind is missing, install it through the approved setup path instead of skipping

---

## §3. Out of scope

| Item | Pushed to |
|---|---|
| Fixing unrelated flaky legacy demos | File as P2 unless it blocks predictor upgrade verification |
| Performance tuning beyond spec SLO breaches | Later optimization slice |
| Real cloud provider credentials provisioning | Maintainer environment; mock path is allowed only when key absent |

---

## §4. File-level change list

### 4.1 New files

- `docs/reviews/hardening/HARDEN_02/e2e-runbook.md`
- `docs/reviews/hardening/HARDEN_02/command-results.md`
- `docs/reviews/hardening/HARDEN_02/kind-production-values.example.yaml`
- `docs/reviews/hardening/HARDEN_02/benchmark-results.json`

### 4.2 Modified files

- `tests/e2e/predictor_upgrade.sh` if execution reveals missing waits, wrong service names, or false success checks
- `tests/e2e/verify_audit_columns.py` if it misses required columns or cannot connect in the demo topology
- `Makefile` demo targets if demo modes do not actually route through the predictor services
- `deploy/demo/compose.yaml` if services fail to boot together
- `charts/spendguard/**` if demo/production profiles do not template or install cleanly
- `benchmarks/predictor-upgrade/**` if the benchmark crate does not run from a clean checkout

---

## §5. Schema / proto changes

No planned schema or proto changes. If real execution reveals a required wire fix, the change must remain additive and include regenerated clients where applicable.

---

## §6. Audit-chain impact

This slice is the audit-chain proof gate:

- `verify_audit_columns.py` must validate all 17 prediction columns plus 4 commit-side columns
- `verify-chain --check-prediction-mirror` must run on demo-produced rows
- Demo artifacts must include the audit event IDs used for validation
- No demo mode may silently bypass canonical_ingest for `spendguard.audit.*` events

---

## §7. Failure mode coverage

| Scenario | Expected behavior |
|---|---|
| Docker compose service exits | Capture logs, fix config/code if in scope, rerun |
| Provider key absent | Use documented mock path and mark provider-real coverage as not exercised |
| kind unavailable | Install kind or document install failure with exact command output |
| Helm production values incomplete | Add a minimal production values artifact; do not weaken production gates |
| Benchmark cannot build | Fix Cargo/package drift in-slice or route to HARDEN_07 if lockfile-wide |

---

## §8. Acceptance criteria

### 8.1 Docker-compose E2E

- `bash tests/e2e/predictor_upgrade.sh` exits 0
- Logs show tokenizer, output_predictor, run_cost_projector, canonical_ingest, sidecar, and egress_proxy healthy

### 8.2 Demo modes

- `default`, `m1_benchmark_runaway_loop`, `multi_provider_usd`, `agent_real_anthropic`, and `plugin_c_synthetic` all run to completion or record an environment-only blocker
- RUN budget projection and Strategy C circuit breaker are verified from logs or audit rows

### 8.3 Helm/kind

- `helm template charts/spendguard --set chart.profile=demo` passes
- `helm template charts/spendguard --set chart.profile=production -f docs/reviews/hardening/HARDEN_02/kind-production-values.example.yaml` passes
- kind cluster install reaches all required pods Ready

### 8.4 Benchmark

- `cargo run --release -p spendguard-predictor-upgrade-benchmarks` completes
- Results are committed under `docs/reviews/hardening/HARDEN_02/`

---

## §9. Slice-specific adversarial review checklist

1. Are command outputs real and dated, not copied from SLICE_15 docs?
2. Does each demo mode verify the predictor behavior it claims to verify?
3. Does `agent_real_anthropic` distinguish real-key vs mock execution?
4. Does kind production install use production security gates instead of demo shortcuts?
5. Do logs prove that audit rows reached canonical_ingest?
6. Does the runaway-loop mode verify `RUN_BUDGET_PROJECTION_EXCEEDED`, not just process exit?
7. Does multi-provider mode verify four provider routes and tokenizer versions?
8. Are docker/kind installation steps reproducible on a clean machine?
9. Are benchmark numbers produced by the release binary from this branch?
10. Are failures fixed rather than documented away?

---

## §10. Out-of-scope deferrals

| Item | Why deferred |
|---|---|
| Cloud-managed Kubernetes validation | kind is the local production-profile gate |
| Real provider credentials in CI | Secret provisioning is maintainer-owned |
| Long soak testing > 1 hour | Future reliability pass |

---

## §11. Risk / rollback plan

- Risk: demo execution uncovers many pre-existing failures. Mitigation: fix predictor-upgrade blockers first; unrelated flakes become P2 only with evidence.
- Risk: production Helm values artifact accidentally weakens gates. Mitigation: use required secrets/cert placeholders, not disabled security.
- Rollback: revert code/script fixes from this slice; retain command-results docs as historical evidence if useful.

---

## §12. AIT execution notes

Reviewer: codex CLI via `ait run --adapter codex --review-mode adversarial`. Max 5 rounds. Staff+ panel arbitration if 5 rounds fail.

The reviewer must inspect `docs/reviews/hardening/HARDEN_02/command-results.md` and reject the slice if demo commands were not actually run.

---

## §13. Adoption history

| Round | Reviewer / panelist | Decision | Outcome |
|---|---|---|---|
| Design | Software Architect | Runtime validation must happen before issue triage | HARDEN_02 precedes HARDEN_03 |
| Design | Backend Architect | Demo failures need logs and command provenance | §4 creates command-results artifacts |
| Design | Security Engineer | Production Helm install must keep strict gates | §8 requires production values instead of disabling checks |
| Design | Database Optimizer | Audit column verification must query produced rows | §6 requires audit event IDs |
| Design | Performance Benchmarker | Benchmark numbers must come from release binary | §8.4 gates release benchmark execution |

---

## §14. Merge checklist

- [ ] All required demo commands run and results committed
- [ ] Docker-compose E2E exits 0
- [ ] kind production install succeeds
- [ ] Release benchmark completes with recorded numbers
- [ ] Any environment-only skips are explicit and justified
- [ ] AIT adversarial review passes or Staff+ arbitration is recorded

---

*Slice version: HARDEN_02_e2e_real_cluster_validation v1alpha1 | Spec ancestor: predictor-upgrade-hardening-spec-v1alpha1 | Branch: `harden/HARDEN_02_e2e_real_cluster_validation`*
