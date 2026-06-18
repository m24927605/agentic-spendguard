# SpendGuard Documentation

Start with the [project README](../README.md) and
[ARCHITECTURE.md](../ARCHITECTURE.md). This folder holds the deeper reference
material.

## Specs (authoritative, versioned)

The source of truth for wire formats and invariants. Read before changing
`proto/` or `migrations/`.

| Spec | What it covers |
|---|---|
| [agent-runtime-spend-guardrails-complete](agent-runtime-spend-guardrails-complete.md) | Full system design doc |
| [ledger-storage-spec](ledger-storage-spec-v1alpha1.md) | Double-entry model, idempotency, replay |
| [contract-dsl-spec](contract-dsl-spec-v1alpha1.md) ([v1alpha2](contract-dsl-spec-v1alpha2.md)) | Decision boundary semantics |
| [trace-schema-spec](trace-schema-spec-v1alpha1.md) | CloudEvent / audit chain schema |
| [sidecar-architecture-spec](sidecar-architecture-spec-v1alpha1.md) | Fencing, drain, capability handshake |
| [stage2-poc-topology-spec](stage2-poc-topology-spec-v1alpha1.md) | Phase 1 SaaS topology + durability invariants |
| [output-predictor-service-spec](output-predictor-service-spec-v1alpha1.md) · [plugin contract](output-predictor-plugin-contract-v1alpha1.md) | Output prediction |
| [run-cost-projector-spec](run-cost-projector-spec-v1alpha1.md) · [tokenizer-service-spec](tokenizer-service-spec-v1alpha1.md) · [stats-aggregator-spec](stats-aggregator-spec-v1alpha1.md) | Predictor subsystem |
| [calibration-report-spec](calibration-report-spec-v1alpha1.md) · [cold-start-baseline-spec](cold-start-baseline-spec-v1alpha1.md) | Calibration & cold start |
| [ga-readiness-spec](ga-readiness-spec-v1alpha1.md) · [post-ga-backlog-spec](post-ga-backlog-spec-v1alpha1.md) | GA readiness & backlog |

More detailed and per-coverage specs live under [`specs/`](specs/).

## Guides & reference

- [**Integrations**](integrations.md) — full adapter matrix + demo modes.
- [**Operations**](operations/) — runbooks, drills, metrics inventory,
  migration & rollback playbooks, soak, SLO proof.
- [**Security**](security/) — [threat model](security/threat-model-ga.md),
  [supply chain](security/supply-chain.md), GA signoff.
- [**Deployment**](deployment/) — production Helm values · [release](release/) —
  versioning policy, release notes, bundle.
- [**Customer onboarding**](customer/) — plugin onboarding, certification
  checklist, error taxonomy.
- [**Strategy**](strategy/) — framework coverage taxonomy.

## Documentation sites

- [`site-v2/`](site-v2/) — the published documentation site (Astro Starlight).
- [`site/`](site/) — the previous documentation site.

## Internal

[`internal/`](internal/) holds development-process artifacts — implementation
slice docs, review records, launch notes, retrospectives, and marketing
drafts. Kept for project history; not part of the user-facing documentation.
