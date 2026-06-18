# POST_GA 01 - Ledger Release Replay Semantics

> **Branch**: `post-ga/POST_GA_01_ledger_replay_semantics`
> **Status**: implementation complete; adversarial review round 1 clean
> **Spec ancestor(s)**: `post-ga-backlog-spec-v1alpha1.md`, `ledger-storage-spec-v1alpha1.md`, `proto/spendguard/ledger/v1/ledger.proto`, `proto/spendguard/common/v1/common.proto`, `proto/spendguard/sidecar_adapter/v1/adapter.proto`
> **Issues**: #85, #86, #87
> **Estimated change size**: medium; ledger/sidecar release semantics, shared error code, tests, docs

---

## §0. TL;DR

Fix ReleaseReservation replay behavior so idempotent replay returns the
original audit signature, legitimate release replay after lease movement
is not blocked by stale fencing preflight, and idempotency conflicts map
to `FailedPrecondition` instead of `Internal`.

## §1. Architectural Context

ReleaseReservation spans the adapter UDS surface, sidecar transaction
orchestration, and ledger idempotency procedure. The ledger procedure
already checks the release idempotency key before fencing, which is the
right ordering for safe replay, but the explicit sidecar RPC had a
preflight fencing gate that prevented that replay path. The sidecar also
receives no original release signature from the ledger replay response,
so it must preserve the signature it emitted for short-window adapter
retries without fabricating signatures for audit events that were not
persisted. Idempotency conflicts must use a stable shared proto error
code so sidecar maps them to `FailedPrecondition`.

## §2. Scope

- #85: same-process release replay returns the original audit event
  signature from a bounded sidecar replay cache keyed by tenant and
  ledger release idempotency key
- #86: explicit `ReleaseReservation` defers fencing enforcement to
  `transaction::run_release`, whose first-mutation path still checks
  fencing while replay can reach ledger idempotency-first handling
- #87: shared `Error.Code.IDEMPOTENCY_CONFLICT` maps ledger body errors
  to sidecar `DomainError::IdempotencyConflict` and gRPC
  `FailedPrecondition`
- Add regression tests for replay signature cache behavior, proto error
  mapping, and explicit release preflight semantics
- Update SDK, ASP Draft-01 status text, and proto comments where
  limitations are closed

## §3. Out of Scope

| Item | Pushed to |
|---|---|
| New reservation lifecycle states | Future ledger spec |
| Changing ReserveSet semantics | Not needed for release replay |
| Durable ledger storage of original release signatures in replay body | Later proto/schema evolution; POST_GA_01 uses sidecar short-window cache |
| Cross-service SDK API redesign | Separate SDK compatibility work |

## §4. File-Level Changes

- Add `IDEMPOTENCY_CONFLICT` to `proto/spendguard/common/v1/common.proto`
- Regenerate Python SDK proto stubs under `sdk/python/src/spendguard/_proto/**`
- Modify ledger and sidecar error mapping under `services/ledger/src/domain/error.rs` and `services/sidecar/src/domain/error.rs`
- Modify release orchestration under `services/sidecar/src/decision/transaction.rs`
- Modify explicit adapter handler under `services/sidecar/src/server/adapter_uds.rs`
- Add targeted unit tests in the same Rust modules
- Update SDK and ASP docs under `sdk/python/src/spendguard/client.py` and `docs/specs/agent-spend-protocol/draft-01.md`
- Update `proto/spendguard/sidecar_adapter/v1/adapter.proto` comments
- Add evidence under `docs/internal/reviews/post-ga/POST_GA_01_ledger_replay_semantics/`

## §5. Schema / Proto

Additive enum-only proto change:

- `spendguard.common.v1.Error.Code.IDEMPOTENCY_CONFLICT = 18`

No message fields change. The release signature fix is intentionally not
a ledger schema migration in this slice; the ledger replay response does
not currently carry the persisted CloudEvent signature, so sidecar caches
the first response signature for bounded retry windows and otherwise
fails closed by returning no fabricated signature.

## §6. Audit-Chain Impact

Replay must not append a second audit event. A safe replay returns the
first release result and, when the sidecar retry cache still contains the
first signature, the original signature bytes. A cache miss returns an
empty signature rather than a regenerated signature, because returning a
signature for an audit event the ledger did not persist would corrupt the
receipt chain. A stale non-idempotent mutation remains rejected by
`run_release` and the ledger procedure before audit append.

## §7. Failure Modes

| Scenario | Expected behavior |
|---|---|
| Same release request replayed in retry cache window | Return original response and audit signature |
| Same release request replayed after cache expiry or sidecar restart | Return original response and empty signature; never return a fabricated signature |
| Replay after lease epoch advances | Permit replay if idempotency key and request fingerprint match |
| Same idempotency key with different payload | `FailedPrecondition` |
| Stale writer attempts new release | Fencing rejection |
| Audit signature unavailable | Fail closed; do not fabricate signature |

## §8. Acceptance Gates

- `cargo build && cargo test` for `services/ledger`
- `cargo build && cargo test` for `services/sidecar`
- `make proto` in `sdk/python`
- Targeted tests cover #85, #86, and #87
- Migration apply smoke if SQL is added
- `git diff --check`
- `helm template spendguard charts/spendguard --set chart.profile=demo`
- Evidence recorded in `docs/internal/reviews/post-ga/POST_GA_01_ledger_replay_semantics/`

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

## §12. Review Execution Notes

Reviewer: codex CLI via `codex review --base main`. Max 5 rounds. Staff+ panel arbitration if 5 rounds fail.

Reviewer must inspect idempotency, fencing, and audit append behavior
before accepting.

## §13. Adoption History

| Role | Decision | Outcome |
|---|---|---|
| Software Architect | Keep release replay isolated from ReserveSet | Scope limited to #85-#87 |
| Backend Architect | Replay identity is ledger release idempotency key plus request fingerprint; signature recovery can be sidecar cache-scoped for this slice | §5-§7 require it |
| Security Engineer | Do not let replay bypass fencing for new mutations | §7 blocks stale mutation |
| Database Optimizer | Avoid ledger schema migration until replay response grows a durable signature field | §5 |
| Ledger Domain Expert | Original audit signature is the user-visible replay truth | §6 |
| Implementer | Added same-process release signature cache, moved explicit preflight fencing into `run_release`, and mapped `IdempotencyConflict` to `IDEMPOTENCY_CONFLICT` | Commits `2447887`, `064de5b`, `ea5bbc1`, `99cb23b` |
| Reviewer | Direct codex CLI review inspected diff and tests | Round 1 clean; no findings |

## §14. Merge Checklist

- [x] #85 fixed and tested
- [x] #86 fixed and tested
- [x] #87 fixed and tested
- [x] Ledger tests and evidence pass
- [x] Codex review clean or Staff+ arbitration recorded
- [ ] Memory updated after merge
