# Architecture

SpendGuard is a **fail-closed spend firewall** for LLM agents. It reserves
budget *before* a provider call is made, refuses the call when the budget is
gone, and records every decision in a tamper-evident, signed audit chain.

This document is the high-level map. The authoritative, versioned specs live
in [`docs/specs/`](docs/specs/) — read those before changing wire formats or
ledger invariants.

## The three layers

A request flows through three components. The proxy is the only thing your
client talks to; the other two are infrastructure.

```
agent ──HTTP──▶ egress-proxy ──UDS gRPC──▶ sidecar ──mTLS gRPC──▶ ledger
                     │                                              │
                     └── byte-identical forward to provider         │
                         ONLY on a CONTINUE decision                ▼
                                                      audit_outbox (signed, append-only)
                                                                    │
                                                  outbox-forwarder ─▶ canonical_ingest
                                                                    │
                                                                    ▼
                                                          your SIEM / data lake
```

### 1. Egress proxy (`services/egress_proxy`, Rust + axum)

- Speaks the OpenAI Chat Completions and Responses wire protocols.
- On a `CONTINUE` decision it forwards to the upstream provider
  byte-identically (streaming SSE is tee'd to the client while usage is
  parsed on the side for the commit lane).
- On `STOP` it returns a structured `spendguard_blocked` body (HTTP 429) and
  **the upstream provider request never fires**.

### 2. Sidecar (`services/sidecar`, Rust + tonic over a Unix domain socket)

- One per pod. Holds the Contract DSL evaluator and the mTLS gRPC client to
  the ledger.
- Decides `Continue` / `Stop` / `RequireApproval` / `Degrade` for every call.
- Signs each decision (Ed25519 locally, or AWS KMS ECDSA P-256 in
  production).

### 3. Ledger + audit chain (`services/ledger`, Postgres)

- An append-only **double-entry ledger**. A reservation is a balanced
  transfer: debit `available_budget`, credit `reserved_hold`. Commit and
  release move the holds. Budgets are funded by an opening deposit, so the
  `available_budget` balance is the remaining budget.
- The **hard cap is enforced in the ledger itself**: a reserve that would
  drive `available_budget` negative is rejected with `BUDGET_EXHAUSTED`
  (the ledger is the authority — not the sidecar's read-before-reserve).
- Every reservation, commit, release, and denied decision is an immutable
  row in `audit_outbox`. Postgres triggers refuse `UPDATE`/`DELETE`; each row
  carries a signature over a canonical hash. The chain is tamper-evident.
- `outbox_forwarder` and `canonical_ingest` close the loop into
  `canonical_events` for downstream consumers; `canonical_ingest` verifies
  signatures at ingest and quarantines failures.

## Core invariants

These are load-bearing. Breaking one is a release blocker.

1. **Fail-closed.** Any error, timeout, or ambiguity on the decision path
   results in *refusing* spend, never allowing it. A `DENY` must never
   silently become an `ALLOW`.
2. **The ledger is the hard cap.** Over-budget reserves are rejected inside
   the ledger transaction under a row lock, not by an advisory read.
3. **Append-only audit.** `audit_outbox` rows are never mutated or deleted;
   integrity rests on Postgres immutability triggers + per-row signatures.
4. **Single writer per budget (Phase 1).** A given budget is written by
   exactly one workload instance at a time, enforced via fencing leases.
   Multi-region writers are Phase 2.
5. **Additive wire evolution.** `proto/` and SQL `migrations/` are
   append-only. Schema changes land as additive, backwards-compatible
   migrations — never edits to applied history.

## Capability levels (L0–L3)

The trust model scales with how much the agent's code can be trusted not to
bypass the gate.

| Level | Mechanism | Residual bypass |
|---|---|---|
| **L0** advisory_sdk | SDK logs decisions; never blocks | Code that skips the SDK |
| **L1** semantic_adapter | SDK refuses the upstream call on `STOP` | Importing the provider client directly |
| **L2** egress_proxy_hard_block | Network proxy rejects un-gated egress (+ NetworkPolicy) | none — the agent must use the proxy |
| **L3** provider_key_gateway | Provider keys live server-side; the agent never sees them | none |

## Predictor subsystem

To decide *before* the provider returns usage, SpendGuard estimates output
cost up front:

- `services/output_predictor` — output-token prediction (strategies A/B/C,
  including a delegated customer-plugin mode over per-tenant SVID mTLS).
- `services/tokenizer` (+ `crates/spendguard-tokenizer`) — multi-vendor token
  counting with bounded input, encode timeouts, and a Tier-1 shadow path.
- `services/run_cost_projector` — projects run cost from the prediction.
- `services/stats_aggregator` — hourly aggregation + drift detection feeding
  calibration.

Predictions are advisory inputs to the decision; the ledger remains the
authority on the dollar.

## Service catalog

| Service | Responsibility | Port |
|---|---|---:|
| `ledger` | Double-entry ledger + audit transactional outbox | 50051 |
| `sidecar` | Per-pod UDS gRPC; contract evaluator; mTLS clients | (UDS) |
| `egress_proxy` | OpenAI-compatible HTTP proxy (1-env-var integration) | 9000 |
| `canonical_ingest` | Per-`decision_id` canonical ordering + storage classes | 50052 |
| `control_plane` | REST API for tenants / budgets / approvals | 8091 |
| `dashboard` | Read-only operator UI | 8090 |
| `outbox_forwarder` | Ledger → canonical_ingest loop | — |
| `signing` | Producer signing trait (local Ed25519 + KMS) | — |
| `ttl_sweeper` | Releases expired reservations | — |
| `webhook_receiver` | HMAC-verified provider webhooks → ledger ops | 8443 |
| `usage_poller` | Provider admin-usage API → usage records | — |

Other services (`auth`, `ids`, `leases`, `policy`, `endpoint_catalog`,
`retention_sweeper`, `cost_advisor`, `bundle_registry`, importers, codecs)
are supporting roles; see each service's `README.md`.

Every external surface is mTLS. Every service exposes Prometheus `/metrics`.
Every audit row is signed.

## Deployment

- **Local / demo:** `deploy/demo/compose.yaml` — full stack with PKI
  bootstrap, manifest signing, and internal mTLS. Bring it up with
  `make demo-up`.
- **Kubernetes:** [`charts/spendguard/`](charts/spendguard/) — a DaemonSet
  sidecar plus Deployments for the core services. `chart.profile=production`
  enforces required-input gates (bundle hashes, trust-root SPKI, real
  Postgres URL) at render time.
- **Signing modes:** `local` (Ed25519 PEM from a Secret), `kms` (AWS KMS
  ECDSA P-256 via IRSA), or `disabled` (demo profile only).

## Where to go next

- [`docs/specs/`](docs/specs/) — versioned, authoritative specs (ledger,
  contract DSL, trace schema, sidecar, predictor).
- [`CONTRIBUTING.md`](CONTRIBUTING.md) — build, test, and the demo gates.
- [`SECURITY.md`](SECURITY.md) — threat model and vulnerability reporting.
