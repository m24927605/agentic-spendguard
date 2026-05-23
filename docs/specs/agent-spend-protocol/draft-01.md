# Agent Spend Protocol — Draft 01

> **Status:** Draft-01, 2026-05-23.
> **Editor:** SpendGuard authors (m24927605@gmail.com).
> **License:** Apache-2.0. Public-domain protocol sketch — not a SpendGuard-specific binding.
> **Repository:** [github.com/m24927605/agentic-spendguard/blob/main/docs/specs/agent-spend-protocol/draft-01.md](https://github.com/m24927605/agentic-spendguard/blob/main/docs/specs/agent-spend-protocol/draft-01.md)

## Abstract

The Agent Spend Protocol (ASP) defines a wire-level contract between an LLM agent (or agent runtime) and a budget-enforcement authority, enabling **pre-call budget reservation**, **post-call usage reconciliation**, and **signed audit emission** for every provider call an agent attempts. ASP is provider-neutral and framework-neutral: any agent runtime that wants to gate spend before the provider clock starts — instead of after the bill arrives — can implement ASP against any enforcement authority that speaks it.

## 0. Why this exists

Three adjacent standards describe what happens around an LLM call. None of them describe what should be **allowed** before the call:

| Layer | Existing standard | What it covers | Gap |
|---|---|---|---|
| Tracing / observability | [OpenTelemetry GenAI Semantic Conventions](https://opentelemetry.io/docs/specs/semconv/gen-ai/) | Token counts, agent steps, span attributes — *after* the call | Cannot reject a call |
| Billing reconciliation | [FOCUS 1.0](https://focus.finos.org/) (FinOps Foundation) | Provider invoice schema — *days* after the call | Cannot reject a call |
| Identity & delegation | [APS / agent-governance-vocabulary](https://github.com/aeoess/agent-governance-vocabulary) | Who an agent is, what it's allowed to do | No transactional spend primitive |

An agent that hits a retry loop at 02:47am can consume $380 of provider tokens in 40 minutes. Detection-via-invoice arrives the next morning. Detection-via-spend-trace arrives in the post-mortem. **ASP is the protocol that makes detection arrive at the 11th call.**

The pattern is well-known outside LLMs — it is what Stripe calls "auth/capture": reserve the worst case before the operation, commit the real cost after, refund the overshoot, sign every step. ASP applies that pattern to LLM tokens.

## 1. Terminology and vocabulary alignment

This protocol reuses canonical terms from [`aeoess/agent-governance-vocabulary`](https://github.com/aeoess/agent-governance-vocabulary) (the multi-issuer neutral-ground governance vocabulary referenced by APS, SINT, AgentID, AgentGraph, MolTrust, ScopeBlind). A `crosswalk/asp.yaml` is anticipated for Draft-02; alignment markers are noted inline.

| ASP term | Definition | Vocabulary alignment |
|---|---|---|
| **Authority** | The entity that decides ALLOW / DENY / DEGRADE / REQUIRE_APPROVAL. May be a sidecar, a gateway, or an SaaS endpoint. | _new_ |
| **Budget** | A scoped capacity envelope: tenant + window + unit (e.g. `acme-team-3 / 2026-05 / output_token`). | _new_ |
| **Claim** | A signed amount the caller asserts against a Budget. Direction = DEBIT or CREDIT. | _new_ |
| **Reservation** | A held Claim with a TTL. Becomes a permanent Debit on Commit, or is released on Cancel/TTL expiry. | _new_ |
| **Commit** | Reconciles a Reservation against observed usage. Refunds overshoot, charges undershoot. Idempotent on `reservation_id`. | _new_ |
| **Decision** | The Authority's verdict on a Reserve request. One of ALLOW / DENY / DEGRADE / REQUIRE_APPROVAL. | aligns with `enforcement_class: binding` |
| **Decision Context** | The set of facts the Authority used (and the Decision is bound to via signature). | aligns with `context_facts` |
| **Audit Event** | A signed CloudEvent emitted for every Reserve / Commit / Reject, including the bound Decision Context. | aligns with `compliance_attestation` / `settlement_witness` |
| **Decision Lineage** | Ordered chain of Audit Events for a logical agent step. | uses canonical `decision_lineage` |

The protocol is intentionally **agnostic about identity**: who the caller is (`actor`), and which authority signed the receipt (`issuer`), are out of scope. ASP composes with APS, AgentID, x402 settlement, ERC-8004 attestation, and other identity-layer protocols by accepting them as inputs to Decision Context.

## 2. Transaction model

Every guarded provider call passes through five stages:

```
  ┌─────────┐       1. RESOLVE         ┌───────────┐
  │  Agent  │ ───────────────────────▶ │ Authority │
  └─────────┘                          └───────────┘
       │                                     │
       │           2. RESERVE                │
       │ ──────────────────────────────────▶ │
       │ ◀────────────────────────────────── │  Decision + reservation_id + ttl
       │                                     │
       │           3. CALL PROVIDER          │
       │   (proceed only if ALLOW)           │
       │                                     │
       │           4. COMMIT                 │
       │ ──────────────────────────────────▶ │  amount_atomic_observed
       │ ◀────────────────────────────────── │  refund_amount or charge_amount
       │                                     │
       │           5. AUDIT (asynchronous)   │
       │                                     │ ──▶ signed CloudEvent → audit log
```

**Stage semantics:**

1. **Resolve** — given an agent's identity + intended call context, the Authority resolves which Budget the call binds against. Optional if the caller knows its Budget binding statically.
2. **Reserve** — the caller submits a worst-case Claim. The Authority makes a Decision. On ALLOW it returns a Reservation with a TTL deadline by which Commit MUST arrive (else the Reservation is auto-released).
3. **Call provider** — only proceeds on ALLOW. DENY MUST short-circuit before any provider request is initiated.
4. **Commit** — after the provider responds, the caller reports observed `amount_atomic_observed`. The Authority reconciles: if observed < reserved, the difference is refunded; if observed > reserved, the excess is charged (or DENY-on-commit, depending on policy).
5. **Audit** — every Reserve and Commit emits a signed CloudEvent. The audit chain is the durable record; in-memory Authority state is recoverable from the chain.

**Atomicity and idempotency:**

- Reserve is atomic at the (Budget, window) granularity. Concurrent reserves never both succeed past the cap.
- Commit is idempotent on `reservation_id`. Retries are safe.
- Cancel is idempotent on `reservation_id`. Cancel-after-commit is a no-op.

**Failure modes:**

- **Authority unreachable** — caller MUST fail-closed (deny the provider call), MAY fail-open under an explicit operator override flag (development only).
- **TTL expiry without Commit** — Reservation auto-releases. Caller's provider call may have completed and tokens been billed; this is the **reconciliation gap** every implementation accepts and SHOULD instrument.
- **Commit observes more than reserved** — Authority policy: either (a) charge overage + alert, or (b) reject and quarantine the call. ASP exposes both; default is (a) under SLO ≤ 2× reservation, (b) above.

## 3. Reserve request

```protobuf
message ReserveRequest {
  // The caller is asking to spend at most `claim.amount_atomic`
  // against `claim.budget_id` under `claim.unit` and
  // `claim.window_instance_id`.
  BudgetClaim claim = 1;

  // Identity inputs — opaque to ASP, passed into Decision Context.
  // Bind whatever your identity layer produces (APS receipt,
  // AgentID JWT, ERC-8004 attestation, plain tenant string).
  google.protobuf.Struct identity = 2;

  // Runtime metadata bound into the signed Audit Event for forensics.
  // Authority SHOULD allowlist keys; values MUST be scalar
  // (string / bool / number / null). Allowlist defined by
  // governance policy, not by ASP.
  google.protobuf.Struct runtime_metadata = 3;

  // For idempotent retry under the same Reservation slot.
  // Authority dedupes on (caller_id, idempotency_key).
  string idempotency_key = 4;
}

message ReserveResponse {
  enum Decision {
    DECISION_UNSPECIFIED = 0;
    ALLOW = 1;
    DENY = 2;
    DEGRADE = 3;            // route to cheaper model / smaller context
    REQUIRE_APPROVAL = 4;   // human-in-the-loop hold
  }
  Decision decision = 1;
  string reservation_id = 2;        // empty if decision != ALLOW
  google.protobuf.Timestamp ttl_expires_at = 3;
  repeated string reason_codes = 4; // machine-readable rationale
  repeated string matched_rule_ids = 5;
  bytes audit_event_signature = 6;  // detached signature of the
                                    // emitted Audit Event for this
                                    // Reserve; lets caller pin
                                    // payloads to receipts
}
```

## 4. Commit request

```protobuf
message CommitRequest {
  string reservation_id = 1;
  string amount_atomic_observed = 2;  // decimal string, in claim.unit
  google.protobuf.Struct provider_response_facts = 3;
  // Optional: usage breakdown observed from provider
  // (e.g. completion_tokens, prompt_tokens, cached_tokens) for
  // richer audit + cost-attribution.
}

message CommitResponse {
  string refund_amount_atomic = 1;     // non-empty if observed < reserved
  string charge_amount_atomic = 2;     // non-empty if observed > reserved
  bytes audit_event_signature = 3;
}
```

## 5. Audit Event envelope

ASP emits one CloudEvent (v1.0.2) per Reserve and per Commit. Type discriminator:

```
type: org.agentspend.audit.decision    # Reserve decision
type: org.agentspend.audit.outcome     # Commit outcome
```

The `data` payload carries the signed Decision Context. Signing is detached (Ed25519 over the canonicalized payload, JCS canonical JSON form). Issuers SHOULD publish JWKS at a well-known URL; ASP does NOT prescribe issuer-key rotation, deferring to `signature_capability` discipline from the broader vocabulary.

**Minimum decision_context fields:**

| Field | Type | Required | Notes |
|---|---|---|---|
| `decision_id` | UUID | ✓ | Stable across retries via `idempotency_key` |
| `budget_id` | string | ✓ | The Budget the Claim hit |
| `unit` | string | ✓ | e.g. `output_token`, `usd_atomic`, `request` |
| `amount_atomic_reserved` | decimal string | ✓ | The Claim |
| `decision` | enum | ✓ | ALLOW / DENY / DEGRADE / REQUIRE_APPROVAL |
| `reason_codes` | string[] | recommended | machine-readable rationale |
| `runtime_metadata` | Struct | optional | allowlisted keys only |

Provider-specific extensions (e.g. for LiteLLM: `litellm_call_id`, `model`, `team_id`, `pricing_version`, `price_snapshot_hash_hex`, `fx_rate_version`, `unit_conversion_version`, `call_type`, `stream`, `mode`, `integration`) are valid `runtime_metadata` keys and are bound by the signature like any other context field.

## 6. Compatibility

- **OpenTelemetry GenAI** — ASP composes with OTel GenAI by emitting span events (`asp.reserve`, `asp.commit`, `asp.audit`) on the GenAI span. Concrete span event names + attribute mapping are the subject of [the parallel OTel SIG proposal](../../proposals/otel-genai-spend-extension.md).
- **FOCUS 1.0** — Commit observations SHOULD be exportable to FOCUS-compliant billing schemas for daily reconciliation against provider invoices. Mapping is one-way (FOCUS → ASP can't reconstruct decisions; ASP → FOCUS can produce a charge feed).
- **APS / AgentID / ERC-8004** — accepted as inputs to `identity`. ASP does NOT replace identity layers and does NOT prescribe how `actor` is established.
- **CloudEvents 1.0.2** — wire envelope of choice. JSON serialization.

## 7. Open questions for v0.2

1. **Cross-Authority settlement** — when a call spans Authorities (e.g. tenant A's agent calls tenant B's tool), does Reserve cascade, or does each Authority hold its own Reservation? Draft-01 punts.
2. **Multi-provider atomic budgets** — a Budget capped in `usd_atomic` that funds calls across OpenAI + Anthropic + Bedrock needs FX + pricing-version pinning that's currently implementation-defined. Should ASP standardize the freeze schema?
3. **DEGRADE semantics** — Draft-01 lists DEGRADE as a Decision but doesn't define how the Authority communicates the degraded route. Should ASP carry the routing hint, or is that out-of-band?
4. **Crosswalk to `agent-governance-vocabulary`** — formal `crosswalk/asp.yaml` PR, validating Decision Context fields against canonical terms.
5. **Quarantine on signature failure** — when an Audit Event's signature can't be verified by a downstream consumer, what's the protocol-mandated handling? (SpendGuard's reference implementation quarantines; not all consumers will.)

## 8. Reference implementation

[SpendGuard](https://github.com/m24927605/agentic-spendguard) (Apache 2.0) implements Draft-01 with a Rust sidecar Authority + Python SDK callers. SpendGuard ships adapters for LiteLLM, OpenAI Agents SDK, LangChain, LangGraph, Pydantic-AI, and Microsoft Agent Governance Toolkit. The 12-field LiteLLM `decision_context` extension is implemented and live-verified per [GH #77](https://github.com/m24927605/agentic-spendguard/issues/77).

A reference implementation is **not** the protocol. This document describes the protocol; alternative implementations are encouraged.

## 9. Acknowledgements

- The category framing draws on conversations and prior art from Tymofii Pidlisnyi's APS work and the broader `agent-governance-vocabulary` project — the existence of a multi-vendor neutral-ground vocabulary made it possible to write ASP without inventing parallel terminology for identity, attestation, and lineage.
- The pre-call-reservation pattern is borrowed wholesale from Stripe's auth/capture model for card payments.
- The audit-chain immutability discipline draws on prior work in financial-services double-entry bookkeeping and the CloudEvents conformance test suite.

## 10. Changelog

- **Draft-01** (2026-05-23) — initial public draft. Open for comment via GitHub issues at the repository above.

---

> Comments, corrections, and crosswalk PRs welcome at  
> [github.com/m24927605/agentic-spendguard/issues](https://github.com/m24927605/agentic-spendguard/issues).
