# GA 03 - Production Helm Values

> **Branch**: `ga/GA_03_production_helm_values`
> **Status**: design
> **Spec ancestor(s)**: `ga-readiness-spec-v1alpha1.md`
> **Estimated change size**: medium; Helm values, validation script, deployment docs

---

## §0. TL;DR

Ship a production values example and validator that render all required SpendGuard workloads without embedding credentials.

## §1. Architectural Context

HARDEN verified Helm mechanics, but GA operators still need a production-ready values file with Secrets, SVID bindings, NetworkPolicy, and security defaults explained.

## §2. Scope

- Production values example
- Production values guide
- Helm validation script
- Secret and cert-manager/SVID documentation
- Negative checks for plaintext DB URLs and missing required references

## §3. Out of Scope

| Item | Pushed to |
|---|---|
| Cloud-provider Terraform modules | Future deployment automation |
| Customer-managed CA UI | Future control-plane work |

## §4. File-Level Changes

- Add `charts/spendguard/values-production.example.yaml`
- Add `docs/deployment/production-helm-values.md`
- Add `scripts/release/validate-production-helm-values.sh`
- Update `charts/spendguard/README.md` if needed
- Add evidence under `docs/reviews/ga-readiness/GA_03_production_helm_values/`

## §5. Schema / Config / API Impact

No runtime schema changes. Helm values become the public production config contract.

## §6. Audit / Security / Operational Impact

Examples must reference Kubernetes Secrets and must preserve container security baseline and NetworkPolicy behavior.

## §7. Failure Modes

| Scenario | Expected behavior |
|---|---|
| Plaintext DB URL in values | Validator exits non-zero |
| Missing SVID binding for Strategy C | Production render fails |
| Missing required Secret name | Production render fails or validator exits non-zero |
| Security context disabled | Validator exits non-zero |

## §8. Acceptance Gates

- `scripts/release/validate-production-helm-values.sh`
- `helm template spendguard charts/spendguard --set chart.profile=demo`
- `helm template spendguard charts/spendguard -f charts/spendguard/values-production.example.yaml`
- Negative grep proves no plaintext DB URL values in production example

## §9. Review Checklist

1. Are DB URLs only Secret references?
2. Are cert-manager and SVID fields explicit?
3. Does production render all required workloads?
4. Does the example avoid credentials?
5. Are read-only filesystem and dropped capabilities preserved?

## §10. Deferrals

| Item | Why deferred |
|---|---|
| Cloud-specific values examples | Future provider-specific guides |

## §11. Risk / Rollback

Rollback by reverting values/docs/scripts. Runtime chart defaults remain unless this slice explicitly changes them.

## §12. AIT Execution Notes

Reviewer: codex CLI via `ait run --adapter codex --review-mode adversarial`. Max 5 rounds. Staff+ panel arbitration if 5 rounds fail.

Reviewer must inspect production render, plaintext secret prevention, SVID binding coverage, and security context invariants.

## §13. Adoption History

| Role | Decision | Outcome |
|---|---|---|
| Release Engineering Architect | Production Helm values need a dedicated slice | GA_03 owns values |
| Security Engineer | Example values must be credential-free | Secret-only contract |
| Release/Helm Architect | R5 unsupported predictor endpoint and UDS hostPath gaps are in-scope | Production chart rejects unsupported egress-proxy predictor wiring and all non-root `DirectoryOrCreate` UDS mounts |
| Security Engineer | R5 invalid Secret/SVID names are security-baseline gaps | Helm and validator enforce DNS-safe Secret, issuer, and client cert IDs |
| SRE/Ops Engineer | R5 runtime-broken green renders are unacceptable | All R5 repros fail closed before merge |

## §14. Merge Checklist

- [x] Production values example exists
- [x] Validator passes
- [x] Demo and production Helm renders pass
- [x] AIT review clean or arbitration recorded
- [ ] Memory updated
