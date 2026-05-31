# GA Readiness Acceptance Standard v1alpha1

> **Status**: draft
> **Applies to**: GA_01 through GA_10

---

## §0. Purpose

This standard prevents GA readiness work from becoming documentation-only optimism. A slice passes only when its stated artifact exists, its automation runs, its evidence is recorded, and its review findings are closed.

---

## §1. Universal Gates

Every slice must record:

- base branch and head commit
- commands run
- pass/fail results
- evidence path when applicable
- review rounds taken
- arbitration result when applicable
- memory update path

Every slice touching deployable behavior must run:

```bash
helm template spendguard charts/spendguard --set chart.profile=demo
```

Every production deployment slice must also run a production render with the slice's required values file.

---

## §2. Evidence Requirements

Evidence belongs under:

```text
docs/reviews/ga-readiness/GA_NN_<name>/
```

The minimum bundle is:

- `README.md` with command, environment, commit, result
- structured output if the tool supports it
- log excerpt or summary for long-running commands

Benchmarks must include p50, p95, p99, max, sample count, error count, and environment descriptor.

---

## §3. Demo Requirements

Demo gates must actually run. Valid demo modes include:

- `default`
- `m1_benchmark_runaway_loop`
- `multi_provider_usd`
- `agent_real_anthropic`
- `plugin_c_synthetic`

The slice doc decides which are relevant. If a slice changes a path exercised by a demo mode, that mode is required.

---

## §4. Operational Readiness Gates

Dashboards, alerts, and runbooks pass only when:

- every metric name in docs is emitted or explicitly non-GA
- Prometheus rule files parse
- dashboards load as JSON
- runbooks contain detection, diagnosis, mitigation, rollback, and evidence collection
- at least one drill command or reproducible scenario validates the runbook path

---

## §5. Security Gates

Security signoff passes only when:

- Helm examples contain no plaintext secrets
- supply-chain artifact generation succeeds or explicitly fails closed when tools are missing
- container security baseline remains intact
- RLS and audit immutability checks pass where DB paths are touched
- SVID and cert rotation docs match shipped runtime behavior

---

## §6. Review Closure

All codex CLI adversarial review findings are blockers for the slice regardless of severity label. If R5 still has findings, Staff+ arbitration must decide one of:

- fix in-slice
- accept as out-of-scope with named cross-slice prerequisite
- choose one implementation option and ship

The decision is final and must be recorded in the slice doc adoption history.
