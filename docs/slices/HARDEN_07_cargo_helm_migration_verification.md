# HARDEN 07 — Cargo, Helm, migration, and NetworkPolicy verification

> **Branch**: `harden/HARDEN_07_cargo_helm_migration_verification`
> **Status**: draft
> **Spec ancestor(s)**: `predictor-upgrade-hardening-spec-v1alpha1.md`
> **Depends on prior slices**: HARDEN_01 through HARDEN_06
> **Blocks subsequent slices**: HARDEN_08 final identity verification
> **Estimated change size**: medium; verification scripts, chart fixes, migration tests

---

## §0. TL;DR

Prove the repository is mechanically consistent after the predictor upgrade and hardening work: Cargo.lock, all Helm profiles, all migrations, and NetworkPolicy enforcement. This slice makes deployment and schema application reproducible from a clean checkout.

---

## §1. Architectural context

SLICE_01-15 added multiple Rust services, crates, Helm templates, and migration directories. HARDEN_05/06 add security and signing changes. Before SVID cert minting lands, the base deployment substrate must be verified across package, chart, database, and Kubernetes policy layers.

---

## §2. Scope (must-do)

- Verify Cargo.lock consistency across all services and crates
- Run `cargo build` and affected `cargo test` suites for the workspace or documented service set
- Render `helm template charts/spendguard --set chart.profile=demo`
- Render `helm template charts/spendguard --set chart.profile=production` with required values
- Render all supported chart profiles and key values combinations
- Start `postgres:16` and apply migrations in order:
  - ledger `0001` through `0051`
  - canonical_ingest `0013` through `0018`
  - control_plane `0001` through `0002` plus hardening migrations
- Run a Kubernetes 1.24+ NetworkPolicy egress enforcement chaos test
- Commit verification scripts and command results

---

## §3. Out of scope

| Item | Pushed to |
|---|---|
| Long-running production soak | Future reliability pass |
| Cloud provider-specific CNI behavior | Future platform validation |
| Performance tuning unrelated to failed gates | Future optimization |

---

## §4. File-level change list

### 4.1 New files

- `scripts/verify-cargo-workspace.sh`
- `scripts/verify-helm-profiles.sh`
- `scripts/verify-migrations-postgres16.sh`
- `tests/k8s/networkpolicy_egress_chaos.sh`
- `docs/reviews/hardening/HARDEN_07/verification-results.md`

### 4.2 Modified files

- `Cargo.toml` and service `Cargo.toml` files if feature/dependency drift is found
- `Cargo.lock` if lock consistency requires update
- `charts/spendguard/**` if profile rendering fails
- Migration files only if they fail from a clean Postgres 16 database and can be fixed additively
- CI workflow files if verification scripts should run continuously

---

## §5. Schema / proto changes

No proto changes. Migration fixes must be additive unless correcting a migration that has not shipped to a real production database. If a shipped migration is wrong, add a new migration rather than rewriting history.

---

## §6. Audit-chain impact

- Migration verification must prove audit_outbox, canonical_events, prediction mirror columns, RLS policies, and control-plane audit_outbox schema apply in the correct order
- NetworkPolicy enforcement must preserve the L2 bypass-resistance promise by blocking direct provider egress where policy is enabled
- Helm production rendering must keep readOnlyRootFilesystem, cap_drop ALL, non-root user, and secret-based DB URLs

---

## §7. Failure mode coverage

| Scenario | Expected behavior |
|---|---|
| Cargo.lock stale | Regenerate/update intentionally and verify build |
| Helm profile fails on missing value | Add documented required value or default only if safe |
| Migration fails mid-sequence | Add forward migration or fix unshipped migration with rationale |
| NetworkPolicy not enforced by CNI | Test fails loudly and documents cluster capability |
| Production template leaks plaintext secret | Treat as blocker and fix before merge |

---

## §8. Acceptance criteria

### 8.1 Cargo

- Workspace or affected-service `cargo build` passes
- Relevant tests pass
- Cargo.lock has no unexplained drift

### 8.2 Helm

- Demo profile templates clean
- Production profile templates clean with required values
- Profile matrix results are committed

### 8.3 Migrations

- Fresh Postgres 16 applies all required migration sequences in order
- RLS and mirror-column smoke queries pass after migration

### 8.4 NetworkPolicy

- kind/Kubernetes 1.24+ egress chaos test proves direct provider egress is blocked when policy is enabled

### 8.5 Demo-mode regression

- `make demo-up DEMO_MODE=default` runs after verification fixes

---

## §9. Slice-specific adversarial review checklist

1. Did verification scripts actually run, and are results committed?
2. Does Cargo.lock match the workspace after HARDEN_05/06 changes?
3. Do Helm production values use secrets, not plaintext DB URLs?
4. Are all chart profiles rendered, not only demo?
5. Were migrations applied to a fresh Postgres 16 container in order?
6. Were shipped migrations preserved with additive fixes where needed?
7. Do RLS smoke tests prove writer `set_config` pattern still works?
8. Does NetworkPolicy chaos test fail if egress bypass is possible?
9. Are container security baselines preserved in rendered manifests?
10. Are any verification failures hidden behind `|| true` or non-failing scripts?

---

## §10. Out-of-scope deferrals

| Item | Why deferred |
|---|---|
| Managed Kubernetes provider matrix | kind is sufficient for local hardening |
| Full database downgrade testing | Project uses forward migrations |
| Non-Rust SDK dependency locking beyond Python SDK touched in HARDEN_03 | Future packaging pass |

---

## §11. Risk / rollback plan

- Risk: migration verification requires changing shipped migrations. Mitigation: prefer new forward migrations; document any exception.
- Risk: Helm production values become too permissive for template success. Mitigation: required values artifact supplies placeholders, not disabled gates.
- Rollback: revert specific chart/script/migration fixes; verification artifacts may remain as evidence.

---

## §12. AIT execution notes

Reviewer: codex CLI via `ait run --adapter codex --review-mode adversarial`. Max 5 rounds. Staff+ panel arbitration if 5 rounds fail.

Reviewer should run or inspect the verification scripts and reject any script that masks command failures.

---

## §13. Adoption history

| Round | Reviewer / panelist | Decision | Outcome |
|---|---|---|---|
| Design | Software Architect | Verification belongs after security/signing changes | HARDEN_07 follows HARDEN_06 |
| Design | Backend Architect | Scripts must be reusable, not one-off shell history | §4 creates scripts |
| Design | Security Engineer | Rendered manifests must preserve container baseline and secret handling | §6 and §9 require checks |
| Design | Database Optimizer | Migration application must use fresh Postgres 16 | §2 and §8 require it |
| Design | Kubernetes domain expert | NetworkPolicy must be chaos-tested, not template-only | §8.4 gates enforcement |
| Implementation | codex CLI implementer/reviewer | Verification scripts must run on the local macOS/Bash 3 environment, not just Linux CI | Replaced `mapfile`; used `cargo metadata --no-deps` for manifests without committed lockfiles |
| Implementation | codex CLI implementer/reviewer | KMS signing check must target control-plane local signing material, not canonical-ingest trust-store material | Narrowed production KMS check to the control-plane rendered section |
| Implementation | codex CLI implementer/reviewer | Demo regression remains a hard gate after verification fixes | `make demo-up DEMO_MODE=default` passed Step 8, outbox drain, and canonical_events verification |
| Review R1 | codex CLI adversarial reviewer | Migration checks must fail on missing state, not print diagnostics | Added `RAISE EXCEPTION` assertions for ledger, canonical_ingest, and control_plane required objects |
| Review R1 | codex CLI adversarial reviewer | Cargo verification must not depend on ignored local lockfiles and must run affected tests | Re-runs in a clean detached worktree and adds seven focused regression test commands |
| Review R1 | codex CLI adversarial reviewer | NetworkPolicy denial must be attributable to policy enforcement | Added unlabeled control pod external egress proof before enforced-pod deny |
| Review R1 | codex CLI adversarial reviewer | Helm security checks must inspect rendered objects, not global strings | Added YAML-level container security and database `secretKeyRef` assertions |

---

## §14. Merge checklist

- [x] Cargo verification passes
- [x] Helm profile matrix passes
- [x] Fresh Postgres 16 migration application passes
- [x] NetworkPolicy egress chaos test passes
- [x] Verification results committed
- [ ] AIT adversarial review passes or Staff+ arbitration is recorded

---

*Slice version: HARDEN_07_cargo_helm_migration_verification v1alpha1 | Spec ancestor: predictor-upgrade-hardening-spec-v1alpha1 | Branch: `harden/HARDEN_07_cargo_helm_migration_verification`*
