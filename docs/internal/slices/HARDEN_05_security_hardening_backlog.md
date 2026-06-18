# HARDEN 05 — Security hardening backlog

> **Branch**: `harden/HARDEN_05_security_hardening_backlog`
> **Status**: draft
> **Spec ancestor(s)**: `predictor-upgrade-hardening-spec-v1alpha1.md`
> **Depends on prior slices**: HARDEN_01 through HARDEN_04
> **Blocks subsequent slices**: HARDEN_06 through HARDEN_08
> **Estimated change size**: medium-large; security controls, migrations, and tests

---

## §0. TL;DR

Close the security backlog that was deferred across SLICE_03 through SLICE_06: CloudEvent replay protection, tenant opt-in for PII shadow sampling, count_tokens quota caps, explicit rustls crypto provider installation, and removal of unused tonic gzip feature surface.

---

## §1. Architectural context

Predictor upgrade introduced new signed CloudEvents, provider shadow calls, gRPC services, and tokenization dependencies. The security posture must now match production expectations: replay-resistance, tenant-controlled PII movement, quota isolation, explicit crypto provider setup, and minimized feature attack surface.

---

## §2. Scope (must-do)

- #144 CloudEvent replay protection: deduplicate at canonical_ingest by `(producer_id, event.id)` within a bounded replay window
- #142 PII per-tenant opt-in: tokenizer Tier 1 shadow path must consult a tenant-level allowlist before sending raw prompt text to provider count_tokens APIs
- #138 Cool-down quota cap: rate-limit count_tokens API calls to N/min per tenant and provider
- #106 rustls aws_lc_rs explicit `default_provider().install_default()` at process boot for Rust services using rustls
- #107 Drop unused tonic gzip feature from workspace crates unless a service demonstrably needs it
- Add adversarial tests for replay, opt-in enforcement, quota exhaustion, and crypto-provider boot

---

## §3. Out of scope

| Item | Pushed to |
|---|---|
| Per-tenant SVID client certs | HARDEN_08 |
| Full secrets-management redesign | Future platform hardening |
| Provider key rotation runbook | P2 unless needed by quota tests |

---

## §4. File-level change list

### 4.1 New files

- `services/canonical_ingest/migrations/00XX_event_replay_dedup.sql`
- `docs/internal/reviews/hardening/HARDEN_05/security-hardening-notes.md`
- Security integration tests for replay, PII opt-in, and quota cap under the existing test layout

### 4.2 Modified files

- `services/canonical_ingest/src/**` for replay dedup before immutable append
- `services/tokenizer/src/shadow/**` for tenant allowlist and quota cap
- `services/control_plane/**` if tenant allowlist or quota settings need API persistence
- `services/*/src/main.rs` and shared boot helpers for rustls provider installation
- Workspace `Cargo.toml` files and `Cargo.lock` for tonic gzip feature removal
- Helm values/templates for allowlist/quota configuration

---

## §5. Schema / proto changes

Expected schema additions:

- A canonical_ingest replay ledger keyed by producer, event ID, and expiry/ingest timestamp
- Tenant security settings for shadow PII opt-in and count_tokens quota, unless an existing control-plane table already covers them

No public proto changes are expected unless control-plane APIs require typed tenant settings.

---

## §6. Audit-chain impact

- Replay dedup must happen before appending duplicate CloudEvents to immutable audit storage
- Replayed duplicates should be rejected or idempotently acknowledged with a metric; they must not create second immutable rows
- Tenant PII opt-in changes should be operator-audited if exposed through control-plane APIs
- Quota-denied shadow samples must not affect hot-path reservation safety

---

## §7. Failure mode coverage

| Scenario | Expected behavior |
|---|---|
| Same `(producer_id, event.id)` arrives twice inside window | Second event is rejected or deduped without append |
| Same event ID arrives after window | Accepted only if outside documented replay horizon |
| Tenant has not opted into PII shadow | Raw prompt body is not sent to provider count_tokens |
| Tenant exceeds count_tokens quota | Shadow call skipped; circuit breaker and hot path remain healthy |
| rustls provider already installed | Boot helper is idempotent or handles AlreadyInstalled safely |
| tonic gzip removed | gRPC services still interoperate without compression |

---

## §8. Acceptance criteria

### 8.1 Security tests

- Replay dedup test proves duplicate CloudEvent cannot append twice
- PII opt-in test proves raw text is never sent for non-allowlisted tenant
- Quota cap test proves per-tenant limit blocks excess count_tokens calls
- rustls provider test or boot smoke proves explicit provider install path runs

### 8.2 Build and dependency gates

- `cargo build` and affected `cargo test` suites pass
- `cargo tree -e features` or equivalent notes prove tonic gzip is gone where unused
- `Cargo.lock` is consistent

### 8.3 Helm gates

- Demo and production templates render with secure defaults
- Production profile does not enable PII shadow by default

### 8.4 Demo-mode regression

- `make demo-up DEMO_MODE=plugin_c_synthetic` runs to cover rustls/gRPC paths where applicable

---

## §9. Slice-specific adversarial review checklist

1. Is replay dedup keyed by both producer and event ID?
2. Is the dedup window bounded and indexed?
3. Can a tenant smuggle PII shadow calls without allowlist?
4. Are count_tokens quotas per tenant and provider, not global only?
5. Does quota exhaustion skip shadow work without opening the hot path?
6. Is rustls provider installation performed before any TLS client/server construction?
7. Is provider installation safe across tests and multiple services?
8. Was tonic gzip removed without disabling required compression elsewhere?
9. Do Helm defaults preserve least privilege?
10. Are security findings fixed rather than deferred to GH issues?

---

## §10. Out-of-scope deferrals

| Item | Why deferred |
|---|---|
| SVID mTLS identity | HARDEN_08 owns it |
| Long-term replay retention analytics | Dedup window is enough for production blocker |
| UI for tenant PII opt-in | Control-plane/API is sufficient for hardening |

---

## §11. Risk / rollback plan

- Risk: replay dedup rejects legitimate retries with different payload. Mitigation: log hash mismatch and treat as security event.
- Risk: PII opt-in disables useful shadow data. Mitigation: safe default is no raw-text provider shadow; operators opt in explicitly.
- Risk: rustls provider install breaks tests due to global state. Mitigation: idempotent helper and serial boot test.
- Rollback: revert individual security control commits only with replacement mitigation documented.

---

## §12. Review Execution Notes

Reviewer: codex CLI via `codex review --base main`. Max 5 rounds. Staff+ panel arbitration if 5 rounds fail.

Reviewer should run security-focused grep checks for replay keys, raw prompt egress, quota scope, rustls boot ordering, and tonic feature drift.

---

## §13. Adoption history

| Round | Reviewer / panelist | Decision | Outcome |
|---|---|---|---|
| Design | Software Architect | Security backlog remains one slice because controls share shadow/audit surfaces | §2 scopes five issues |
| Design | Backend Architect | Replay dedup belongs at canonical_ingest before append | §4 and §6 place dedup there |
| Design | Security Engineer | PII shadow default must be opt-out by absence, not opt-in by chart profile | §8 requires production default off |
| Design | Database Optimizer | Replay table needs bounded indexed window | §7 and §9 require it |
| Design | Tokenizer domain expert | Quota cap must not affect Tier 2 hot path | §6 and §7 state shadow-only behavior |
| R1 | codex CLI adversarial review `review:01KSYF6Z3Z7FWTZE7Z9TE9RE9A` | Replay ledger needed runtime grants and quarantine-path replay hash checks | Fixed in commit `b2d269a` |
| R2 | codex CLI adversarial review `review:01KSYFSVJMXCSJXDW3GZAH2ZYS` | Quarantined event IDs needed global reservation; quota needed shared state | Fixed in commits `61a4c74` and `2115cfb` |
| R3 | codex CLI adversarial review `review:01KSYGXR55EMNA7P9HDVGQMENW` | Existing non-released quarantine rows needed migration-time replay reservations | Fixed in commit `6010333` |
| R4 | codex CLI adversarial review `review:01KSYHGGP0YHMTHYT5J1B5SA1V` | Tokenizer shadow runtime needed least-privilege DB credentials | Fixed in commit `61adfa3` |
| R5 | codex CLI adversarial review `review:01KSYJ7RMQWNPQ5WN0Y3PPZXNM` | No findings | Passed; no Staff+ arbitration required |

---

## §14. Merge checklist

- [x] Replay dedup migration and tests pass
- [x] PII opt-in and quota tests pass
- [x] rustls provider installed explicitly in affected services
- [x] tonic gzip feature removed where unused
- [x] Helm demo/production templates pass
- [x] Codex review passes or Staff+ arbitration is recorded

---

*Slice version: HARDEN_05_security_hardening_backlog v1alpha1 | Spec ancestor: predictor-upgrade-hardening-spec-v1alpha1 | Branch: `harden/HARDEN_05_security_hardening_backlog`*
