# POST_GA 01 - Ledger Release Replay Semantics

> **Branch**: `post-ga/POST_GA_01_ledger_replay_semantics`
> **Status**: draft
> **Spec ancestor(s)**: `post-ga-backlog-spec-v1alpha1.md`, `ledger-storage-spec-v1alpha1.md`, `proto/spendguard/ledger/v1/ledger.proto`
> **Issues**: #85, #86, #87
> **Estimated change size**: medium; ledger release semantics, tests, docs

---

## §0. TL;DR

Fix ReleaseReservation replay behavior so idempotent replay returns the
original audit signature, legitimate release replay after lease movement
is not blocked by stale fencing preflight, and idempotency conflicts map
to `FailedPrecondition` instead of `Internal`.

## §1. Architectural Context

Ledger release is part of the reservation lifecycle and audit chain. A
release retry may arrive after the writer lease has advanced; the server
must distinguish safe idempotent replay from stale mutation. The audit
signature produced by the first release is the durable identity of that
operation and must be returned consistently.

## §2. Scope

- #85: release replay branch returns the original audit event signature
- #86: fencing preflight permits legitimate replay after lease change
- #87: idempotency conflict maps to `FailedPrecondition`
- Add regression tests for release success, replay, stale mutation, and
  idempotency conflict mapping
- Update ledger docs and proto comments where semantics were ambiguous

## §3. Out of Scope

| Item | Pushed to |
|---|---|
| New reservation lifecycle states | Future ledger spec |
| Changing ReserveSet semantics | Not needed for release replay |
| Cross-service SDK API redesign | Separate SDK compatibility work |

## §4. File-Level Changes

- Modify ledger Release handler and idempotency helper code under `services/ledger/src/**`
- Add or update tests under `services/ledger/tests/**`
- Update `docs/ledger-storage-spec-v1alpha1.md`
- Update `proto/spendguard/ledger/v1/ledger.proto` comments only if needed
- Add evidence under `docs/reviews/post-ga/POST_GA_01_ledger_replay_semantics/`

## §5. Schema / Proto

No breaking proto field changes are expected. If the original audit
signature is not stored in an accessible release idempotency row, add a
forward-only migration that stores the replay response material without
changing existing audit rows.

## §6. Audit-Chain Impact

Replay must not append a second audit event. A safe replay returns the
first release result and original signature. A stale non-idempotent
mutation is rejected before audit append.

## §7. Failure Modes

| Scenario | Expected behavior |
|---|---|
| Same release request replayed | Return original response and audit signature |
| Replay after lease epoch advances | Permit replay if idempotency key and request fingerprint match |
| Same idempotency key with different payload | `FailedPrecondition` |
| Stale writer attempts new release | Fencing rejection |
| Audit signature unavailable | Fail closed; do not fabricate signature |

## §8. Acceptance Gates

- `cargo build && cargo test` for `services/ledger`
- Targeted release replay tests cover #85, #86, and #87
- Migration apply smoke if SQL is added
- `git diff --check`
- `helm template spendguard charts/spendguard --set chart.profile=demo`
- Evidence recorded in `docs/reviews/post-ga/POST_GA_01_ledger_replay_semantics/`

## §9. Review Checklist

1. Does replay return the original audit signature, not a regenerated one?
2. Is replay identified by stable request fingerprint and idempotency key?
3. Can stale lease holders mutate state through the replay path?
4. Are gRPC status mappings precise and tested?
5. Does the implementation avoid appending duplicate audit events?

## §10. Deferrals

| Item | Why deferred |
|---|---|
| Full ledger lifecycle redesign | Larger than #85-#87 |
| SDK ergonomic wrapper changes | Needs separate compatibility review |

## §11. Risk / Rollback

Risk is duplicate or missing release audit evidence. Keep changes narrow
to release replay and status mapping. Roll back by reverting handler
changes and any forward migration only with an equivalent compatibility
fix ready.

## §12. AIT Execution Notes

Reviewer: codex CLI via `ait run --adapter codex --review-mode adversarial`. Max 5 rounds. Staff+ panel arbitration if 5 rounds fail.

Reviewer must inspect idempotency, fencing, and audit append behavior
before accepting.

## §13. Adoption History

| Role | Decision | Outcome |
|---|---|---|
| Software Architect | Keep release replay isolated from ReserveSet | Scope limited to #85-#87 |
| Backend Architect | Replay identity is idempotency key plus request fingerprint | §7 and §9 require it |
| Security Engineer | Do not let replay bypass fencing for new mutations | §7 blocks stale mutation |
| Database Optimizer | Add schema only if response material is not already durable | §5 |
| Ledger Domain Expert | Original audit signature is the user-visible replay truth | §6 |

## §14. Merge Checklist

- [ ] #85 fixed and tested
- [ ] #86 fixed and tested
- [ ] #87 fixed and tested
- [ ] Ledger tests and evidence pass
- [ ] AIT review clean or Staff+ arbitration recorded
- [ ] Memory updated
