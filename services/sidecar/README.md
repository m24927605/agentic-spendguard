# spendguard-sidecar

Per-workload-instance sidecar binary for the SpendGuard Phase 1
first-customer (K8s SaaS-managed) POC. Runs as a sidecar container in
the customer's app pod; in-process Pydantic-AI / LangGraph adapters
connect via Unix Domain Socket (UDS) gRPC.

## Spec map

- Sidecar Architecture spec v1alpha1 §3 (in_process_adapter +
  local_sidecar responsibilities), §5 (UDS peer credentials handshake),
  §7 (signed fail-safe manifest), §8 (endpoint discovery), §9 (fencing
  token), §11 (lifecycle drain), §14 (resource sizing), §15 (key rotation).
- Contract DSL spec v1alpha1 §6 (decision transaction), §11 (effect
  schema), §14 (latency budget), §15 (trigger points).
- Trace Schema spec v1alpha1 §7 (span tree + canonical events),
  §13 (producer trust).
- Stage 2 POC Topology spec v1alpha1 §4 (audit transactional outbox),
  §11 (Provider Webhook Receiver — sidecar does NOT call this), §12.1
  (Helm trust bootstrap).

## Crate layout

```
src/
├── lib.rs                          re-exports + tonic-include_proto
├── main.rs                          binary: bootstrap + UDS server + drain
├── config.rs                        env-driven Config
├── server/
│   └── adapter_uds.rs               SidecarAdapter trait impl over UDS
├── clients/
│   ├── mod.rs
│   ├── mtls.rs                      ClientTlsConfig builder (cert-manager workload cert)
│   ├── ledger.rs                    Ledger gRPC client wrapper
│   └── canonical_ingest.rs          CanonicalIngest gRPC client wrapper
├── bootstrap/
│   ├── mod.rs
│   ├── trust.rs                     Helm root CA SPKI hash pin verify
│   ├── catalog.rs                   manifest pull + ed25519 verify + atomic swap
│   └── bundles.rs                   contract / schema bundle load + cosign verify (POC stub)
├── decision/
│   ├── mod.rs
│   └── transaction.rs               Contract §6 stages 1-4 (reserve via Ledger atomic outbox)
├── fencing/
│   └── mod.rs                       Sidecar §9 fencing scope cache
├── drain/
│   └── mod.rs                       Sidecar §11 lifecycle drain
└── domain/
    ├── mod.rs
    ├── error.rs                     DomainError → tonic::Status mapping
    └── state.rs                     SidecarState (cached catalog + bundles + fencing)

build.rs                             tonic-build proto codegen
Cargo.toml                           tonic 0.12 + tokio + ed25519-dalek 2 + reqwest 0.12
```

## What's implemented in this skeleton

- Trust bootstrap: Helm root CA bundle PEM → SPKI hash pin verify. Sidecar
  refuses to start on mismatch.
- Endpoint catalog refresh loop: HTTPS pull manifest → ed25519 verify
  against pinned key → fetch versioned catalog → sha256 verify body →
  atomic swap into `SidecarState`.
- Critical-stale gate: every decision checks
  `last_verified_critical_version_age <= critical_max_stale_seconds`
  before accepting the request.
- Adapter UDS gRPC server: Handshake (capability advertise + bundle refs +
  active key epochs), RequestDecision (Contract §6 stages 1-4),
  ConfirmPublishOutcome (POC ack), StreamDrainSignal (edge-trigger).
- Contract §6 decision transaction: snapshot → evaluate (POC stub returns
  CONTINUE for quickstart shadow contract) → prepare_effect (effect_hash
  hash chain) → reserve via Ledger.ReserveSet (atomic with audit_decision
  in ledger.audit_outbox per Stage 2 §4).
- Ledger client: ReserveSet, Release, QueryDecisionOutcome,
  ReplayAuditFromCursor over mTLS.
- Canonical Ingest client: AppendEvents (used for non-audit observability
  events; audit events flow via ledger transactional outbox).
- Drain protocol: SIGTERM handler → mark draining → window wait →
  serve_with_incoming_shutdown completes.

## What's deferred to vertical slice expansion

- Real CEL evaluator (currently CONTINUE-only; the cel-interpreter
  dependency is wired but not invoked).
- Effect lattice composition + same-type merge (Contract §10).
- Lifecycle commit_or_release (LLM_CALL_POST → CommitEstimated /
  ProviderReport pathways).
- Sub-agent budget grant lifecycle (Contract §8 Issue / Revoke / Consume).
- Approval workflow (require_approval decision kind).
- SO_PEERCRED enforcement in handshake (POC accepts any local UDS peer).
- Adapter announcement signature in HandshakeResponse.
- Bundle cosign verify (POC checks .sig file existence + hash; needs
  real sigstore verify against Helm-pinned trust root).
- Real Ledger.AcquireFencingScope RPC + lease renewal background task
  (POC pre-installs ActiveFencing from env / static bootstrap).
- Trace event LLM_CALL_POST routing (drives ledger commit lifecycle).
- Resource limits / cgroup isolation (per Sidecar §14).
- Lambda Extensions runtime (`lambda-extension` Cargo feature).
- Chaos test suite: pod_eviction / rolling_restart / spot_interruption /
  fencing split-brain / catalog manifest sigverify failure / bundle pull
  failure / UDS peer credential mismatch.

## Audit invariant in code

The RequestDecision handler invokes `transaction::run_through_reserve`
which calls `Ledger.ReserveSet`. Per Stage 2 §4, the ledger inserts the
reservation entries AND the audit_decision row into `audit_outbox` in
the SAME Postgres transaction with `synchronous_commit=on` + sync replica
quorum. The handler returns `DecisionResponse` only after the ledger
commit acks. The adapter then performs `apply_mutation` (publish_effect)
using `effect_hash` for idempotency. If the sidecar crashes between
ReserveSet ack and adapter publish, the next sidecar owner queries
`Ledger.QueryDecisionOutcome(decision_id)` and replays publish via the
adapter's idempotent `effect_hash` apply. No effect is ever published
without a durable audit row (Stage 2 §4.3 + Sidecar §6.1).

## Building

`cargo build` (Rust toolchain not present in current workspace; use
Docker or install via rustup).

## Helm deployment (sketch)

The customer Helm chart injects this binary as a sidecar container in
the app pod with:
- `terminationGracePeriodSeconds: 60` for drain (Sidecar §11)
- mounted Secret `spendguard-trust` (Helm-pinned root CA + bootstrap
  token + manifest verify pubkey)
- mounted Secret from cert-manager external issuer (workload cert)
- env vars `SPENDGUARD_SIDECAR_*` populated from Helm values
- shared `emptyDir` volume for the UDS socket (`/var/run/spendguard/`)
- readinessProbe + livenessProbe on port 8080 (`/healthz`)
- preStop exec: `kill -TERM 1` (sidecar handles drain itself)
