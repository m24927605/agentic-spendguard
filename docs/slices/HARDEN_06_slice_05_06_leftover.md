# HARDEN 06 — SLICE_05/06 leftover closure

> **Branch**: `harden/HARDEN_06_slice_05_06_leftover`
> **Status**: draft
> **Spec ancestor(s)**: `predictor-upgrade-hardening-spec-v1alpha1.md`
> **Depends on prior slices**: HARDEN_01 through HARDEN_05
> **Blocks subsequent slices**: HARDEN_07, HARDEN_08
> **Estimated change size**: medium; CloudEvent envelope and control-plane signing wire

---

## §0. TL;DR

Close the two high-risk leftovers from SLICE_05 and SLICE_06: tokenizer shadow drift alerts must use the full `AppendEventsRequest` envelope, and control-plane audit_outbox events must actually flow through an Ed25519 signing and forwarding path rather than stopping at table insert.

---

## §1. Architectural context

SLICE_05 introduced tokenizer drift alerts; SLICE_06 discovered that canonical_ingest requires `producer_id`, `schema_bundle`, and `route`; SLICE_07 added control-plane audit_outbox writes but left the outbox forwarder and signing path unverified. This slice makes those audit-chain edges real.

---

## §2. Scope (must-do)

- Close #168 tokenizer shadow sink `AppendEventsRequest` envelope gap
- Ensure tokenizer sink sends `producer_id`, `schema_bundle`, and `route = Observability` to canonical_ingest
- Add tests proving canonical_ingest rejects missing envelope fields and accepts tokenizer drift alerts with the full envelope
- Wire control-plane audit_outbox forwarder
- Wire Ed25519 signing for control-plane audit events
- Verify plugin registered/updated/deleted/force_reset audit events reach canonical_ingest as signed CloudEvents
- Preserve `spendguard.audit.*` prefix routing for immutable audit storage

---

## §3. Out of scope

| Item | Pushed to |
|---|---|
| Replay protection | HARDEN_05 |
| Per-tenant SVID certs | HARDEN_08 |
| New control-plane event types | Future feature slices |

---

## §4. File-level change list

### 4.1 New files

- `services/control_plane/src/audit_forwarder.rs`
- `services/control_plane/tests/audit_forwarder_integration.rs`
- `services/tokenizer/tests/shadow_append_events_envelope.rs`
- `docs/reviews/hardening/HARDEN_06/audit-envelope-and-signing.md`

### 4.2 Modified files

- `services/tokenizer/src/shadow/sink.rs` and related config/main wiring
- `services/control_plane/src/main.rs`, audit modules, and handler wiring
- `services/control_plane/migrations/**` if outbox lease/status columns are needed
- `charts/spendguard/templates/control_plane.yaml` for signing key and canonical_ingest endpoint configuration
- `charts/spendguard/templates/tokenizer.yaml` if tokenizer sink envelope config is incomplete

---

## §5. Schema / proto changes

No public proto changes expected. Control-plane DB schema may gain outbox forwarder bookkeeping columns if the existing `audit_outbox` table cannot support reliable forwarding.

---

## §6. Audit-chain impact

- Tokenizer drift alerts become canonical_ingest-compatible by carrying the required envelope
- Control-plane plugin lifecycle events become signed and forwarded instead of table-local only
- ImmutableAuditLog receives all `spendguard.audit.plugin_*` and tokenizer drift events through the same verifier path
- Missing envelope fields remain reject conditions; this slice must not weaken canonical_ingest validation

---

## §7. Failure mode coverage

| Scenario | Expected behavior |
|---|---|
| Tokenizer sink omits producer_id | canonical_ingest rejects; test covers |
| Tokenizer sink omits schema_bundle | canonical_ingest rejects; test covers |
| Tokenizer sink omits route | canonical_ingest rejects; test covers |
| Control-plane signing key missing in production | service refuses to start or marks audit forwarder not ready |
| canonical_ingest unavailable | outbox retry preserves events |
| Duplicate forward after crash | idempotency/replay behavior follows HARDEN_05 dedup |

---

## §8. Acceptance criteria

### 8.1 Tokenizer envelope

- Full envelope is constructed in tokenizer shadow sink
- Integration test proves drift alert admission through canonical_ingest

### 8.2 Control-plane signing

- Ed25519 signing path runs for control-plane audit_outbox rows
- Forwarder retries safely and records status
- Plugin lifecycle event integration test reaches canonical_ingest

### 8.3 Verification gates

- Affected services build and test
- Helm demo/production templates include required signing and endpoint values
- No plaintext production shortcut is introduced

### 8.4 Demo-mode regression

- `make demo-up DEMO_MODE=plugin_c_synthetic` runs and produces a signed control-plane audit event

---

## §9. Slice-specific adversarial review checklist

1. Does tokenizer sink use the exact AppendEventsRequest envelope required by SLICE_06 R2 B5?
2. Are `producer_id`, `schema_bundle`, and `route` non-empty and tested?
3. Does canonical_ingest still reject missing envelope fields?
4. Are control-plane events signed before forwarding?
5. Is the Ed25519 private key masked in logs and not embedded in Helm values?
6. Does the outbox forwarder survive canonical_ingest downtime?
7. Are plugin lifecycle event types still `spendguard.audit.plugin_*.v1alpha1`?
8. Does the forwarder avoid double-send issues when HARDEN_05 replay dedup is active?
9. Does production profile fail fast on missing signing config?
10. Does demo mode prove the path end-to-end?

---

## §10. Out-of-scope deferrals

| Item | Why deferred |
|---|---|
| KMS-backed control-plane signing | Ed25519 local signer matches existing slice pattern |
| Control-plane audit dashboard | Not needed for production blocker closure |
| Cross-region outbox forwarding | Future scale work |

---

## §11. Risk / rollback plan

- Risk: forwarder creates duplicate signed events. Mitigation: idempotent outbox status plus HARDEN_05 replay dedup.
- Risk: production boot fails due to signing config. Mitigation: explicit Helm required values and clear error.
- Risk: request-serving RLS hides pending outbox rows from the worker. Mitigation: production requires a separate audit-forwarder database URL using `control_plane_audit_forwarder_role` RLS policies; no `BYPASSRLS` shortcut is used.
- Rollback: disable forwarder only in demo/dev; production rollback requires reverting this slice and reopening blocker issues.

---

## §12. AIT execution notes

Reviewer: codex CLI via `ait run --adapter codex --review-mode adversarial`. Max 5 rounds. Staff+ panel arbitration if 5 rounds fail.

Reviewer must inspect real integration tests for tokenizer envelope admission and control-plane signed forwarding.

---

## §13. Adoption history

| Round | Reviewer / panelist | Decision | Outcome |
|---|---|---|---|
| Design | Software Architect | Treat CloudEvent envelope and signing as one audit-chain slice | §2 scopes both |
| Design | Backend Architect | Do not weaken canonical_ingest validation to make producers pass | §6 and §9 require rejection tests |
| Design | Security Engineer | Production signing key config must fail fast | §7 and §8 require production gates |
| Design | Database Optimizer | Outbox forwarding must record durable status | §4 allows schema bookkeeping |
| Design | Audit-chain domain expert | `spendguard.audit.*` routing is mandatory | §6 and §9 enforce prefix |
| Implementation | Backend Architect | Keep tokenizer envelope as-is and harden canonical_ingest fail-fast tests | `append_events_rejects_*_before_storage` added |
| Implementation | Security Engineer | Use mTLS plus per-event Ed25519 signatures for control-plane forwarding | Helm/compose mount TLS and `control-plane.pem` |
| Implementation | Software Architect | Add a real Helm control-plane surface instead of compose-only wiring | `templates/control-plane.yaml` added |
| Review R1 | codex CLI adversarial reviewer | Demo Helm must not enable forwarder without schema hash; compose must source runtime schema hash and generate signing key | Fixed with `$forwarderEnabled`, control-plane entrypoint, and PKI signing key generation |
| Review R2 | codex CLI adversarial reviewer | Forwarder must not query RLS-protected outbox through the request-serving role | Added `control_plane_audit_forwarder_role`, production audit-forwarder DB URL, Helm secret key, and boot-time production validation |

---

## §14. Merge checklist

- [x] Tokenizer AppendEventsRequest envelope fixed and tested
- [x] Control-plane Ed25519 forwarder implemented and tested
- [ ] Plugin lifecycle audit event reaches canonical_ingest in demo
- [x] Helm production signing gates render correctly
- [x] Affected service tests pass
- [ ] AIT adversarial review passes or Staff+ arbitration is recorded

---

*Slice version: HARDEN_06_slice_05_06_leftover v1alpha1 | Spec ancestor: predictor-upgrade-hardening-spec-v1alpha1 | Branch: `harden/HARDEN_06_slice_05_06_leftover`*
