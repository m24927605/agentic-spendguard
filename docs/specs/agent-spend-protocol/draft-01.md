# Agent Spend Protocol — Draft 01

> **Status:** Draft-01, 2026-05-23. Active revision.
> **Editor:** SpendGuard authors (m24927605@gmail.com).
> **License:** Apache-2.0. Public-domain protocol sketch — not a SpendGuard-specific binding.
> **Repository:** [github.com/m24927605/agentic-spendguard/blob/main/docs/specs/agent-spend-protocol/draft-01.md](https://github.com/m24927605/agentic-spendguard/blob/main/docs/specs/agent-spend-protocol/draft-01.md)
> **Upstream alignment:** SpendGuard binding of the [`crosswalk/budget_reservation.yaml`](https://github.com/aeoess/agent-governance-vocabulary/blob/main/crosswalk/budget_reservation.yaml) canonical-verb-set (`status: INCUBATING`, `crosswalk_type: domain_incubation`, two production implementations as of 2026-05-13). SpendGuard adoption is intended to surface the third production implementation that promotes the candidate verbs (`reserve`, `commit`, `query_budget`) toward canonical, and bring `release` + `refund` to the two-implementation threshold.

## Abstract

The Agent Spend Protocol (ASP) defines a wire-level contract between an LLM agent (or agent runtime) and a budget-enforcement authority, enabling **pre-call budget reservation**, **post-call usage reconciliation**, and **signed audit emission** for every provider call an agent attempts. ASP is provider-neutral and framework-neutral: any agent runtime that wants to gate spend before the provider clock starts — instead of after the bill arrives — can implement ASP against any enforcement authority that speaks it.

This document is the **agent-runtime binding** of the upstream `budget_reservation` canonical-verb domain that is currently incubating in [`aeoess/agent-governance-vocabulary`](https://github.com/aeoess/agent-governance-vocabulary). The verb set (`reserve`, `commit`, `release`, `refund`, `query_budget`) is reused verbatim. The decision shape (`ALLOW`, `ALLOW_WITH_CAPS`, `DENY`) is reused verbatim with two agent-runtime extensions (`DEGRADE`, `REQUIRE_APPROVAL`) documented in §2.

## 0. Why this exists

Three adjacent standards describe what happens around an LLM call. None of them describe what should be **allowed** before the call:

| Layer | Existing standard | What it covers | Gap |
|---|---|---|---|
| Tracing / observability | [OpenTelemetry GenAI Semantic Conventions](https://opentelemetry.io/docs/specs/semconv/gen-ai/) (experimental, March 2026) | Token counts, agent steps, span attributes — *after* the call | Cannot reject a call |
| Billing reconciliation | [FOCUS 1.0](https://focus.finops.org/) (FinOps Foundation) | Provider invoice schema — *days* after the call | Cannot reject a call |
| Identity & delegation | [APS / agent-governance-vocabulary](https://github.com/aeoess/agent-governance-vocabulary) | Who an agent is, what it's allowed to do, canonical names for governance primitives | The budget enforcement primitives exist (`crosswalk/budget_reservation.yaml`, INCUBATING) but the agent-runtime binding does not yet |

An agent that hits a retry loop at 02:47am can consume $380 of provider tokens in 40 minutes. Detection-via-invoice arrives the next morning. Detection-via-spend-trace arrives in the post-mortem. **ASP is the protocol that makes detection arrive at the 11th call.**

The pattern is well-known outside LLMs — it is what Stripe calls "auth/capture": reserve the worst case before the operation, commit the real cost after, refund the overshoot, sign every step. ASP applies that pattern to LLM tokens, reusing the verb set the upstream `budget_reservation` domain has already converged on.

## 1. Relationship to the upstream `budget_reservation` crosswalk

The `agent-governance-vocabulary` repository hosts a `crosswalk/budget_reservation.yaml` file with `crosswalk_type: domain_incubation`, declaring a canonical verb set for "the cumulative-spend enforcement layer between agent identity/delegation (APS) and payment-rail settlement (x402, Stripe, ACP, MPP, AP2)." As of 2026-05-13 it has two production implementations crosswalked — **goodmeta** (verb: `authorize`, renaming to `reserve` pending AP2#252 merge) and **Cycles** (`reserve`) — and a published promotion path: the file promotes from `domain_incubation` to canonical when (1) a third production implementation surfaces and (2) each `proposed` verb gains a second implementer.

ASP Draft-01 is the **agent-runtime binding** of that domain. Concretely:

- ASP's `Reserve` RPC corresponds to the upstream `reserve` verb (`candidate` status; goodmeta + Cycles).
- ASP's `Commit` RPC corresponds to upstream `commit` (`candidate`; goodmeta + Cycles).
- ASP's `Release` RPC corresponds to upstream `release` (`proposed`; Cycles only). SpendGuard adoption gives `release` its second implementer.
- ASP's `Refund` (optional, post-commit reversal) corresponds to upstream `refund` (`proposed`; goodmeta only). SpendGuard adoption gives `refund` its second implementer.
- ASP's optional `QueryBudget` RPC corresponds to upstream `query_budget` (`candidate`).

Crosswalk publication — `crosswalk/asp.yaml` in `agent-governance-vocabulary` — is anticipated for Draft-02 once the wire details below settle through public review.

## 2. Terminology and decision shape

Verbs use the upstream `budget_reservation` canonical names verbatim. Other ASP-specific terms below; broader governance terms (identity, attestation, lineage) defer to the wider `agent-governance-vocabulary` and are not redefined here.

| ASP term | Definition |
|---|---|
| **Authority** | The entity that decides the call. May be a sidecar, a gateway, an SaaS endpoint, or any process the agent runtime can RPC to. |
| **Budget** | A scoped capacity envelope: tenant + window + unit (e.g. `acme-team-3 / 2026-05 / output_token`). |
| **Claim** | A signed amount asserted against a Budget. Direction = DEBIT or CREDIT. |
| **Reservation** | A held Claim with a TTL. Becomes a permanent Debit on `commit`, returns to the Budget on `release`, or auto-releases on TTL expiry (with the late-commit semantics defined in §3). |
| **Decision** | The Authority's verdict on a `reserve`. |
| **Decision Context** | The set of facts the Authority used and the Decision is bound to via signature. |
| **Audit Event** | A signed CloudEvent emitted for every `reserve` / `commit` / `release` / `refund` outcome, including the bound Decision Context. |

**Decision values** (verbatim from upstream `budget_reservation.yaml` plus two extensions):

| Decision | Source | Meaning |
|---|---|---|
| `ALLOW` | upstream canonical | Reserve granted in full; provider call MAY proceed. |
| `ALLOW_WITH_CAPS` | upstream canonical | Reserve granted with structured caps (`{type, params}[]`) the caller MUST honor. ASP defers cap-type vocabulary to upstream v0.2 (`ALLOW_WITH_CAPS_structure`). |
| `DENY` | upstream canonical | Reserve refused. Provider call MUST NOT proceed. |
| `DEGRADE` | **ASP extension** | Reserve refused as-is; the Authority hints an alternative route (cheaper model, smaller context). The routing hint is conveyed as a cap of type `degrade.route_to` under `ALLOW_WITH_CAPS_structure.caps`, so this extension is compatible with the canonical shape. Marked extension because it is agent-runtime-specific (rails don't degrade). |
| `REQUIRE_APPROVAL` | **ASP extension** | Reserve held pending human-in-the-loop approval. Returns an `approval_request_id`; subsequent Commit MUST first poll approval status. Marked extension for the same reason. |

The protocol is intentionally **agnostic about identity**: who the caller is (`actor`) and which authority signed the receipt (`issuer`) are out of scope. ASP composes with APS, AgentID, x402, ERC-8004, and other identity-layer protocols by accepting them as inputs to Decision Context.

## 3. Transaction model

Every guarded provider call passes through this state machine. Stages `2` and `4` are the canonical verbs (`reserve`, `commit`); `5'` is the canonical `release`; `audit` events accompany every state transition.

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
       │   (proceed only if ALLOW or         │
       │    ALLOW_WITH_CAPS)                 │
       │                                     │
       │ ──── EITHER ────────────────────────│
       │           4. COMMIT                 │
       │ ──────────────────────────────────▶ │  amount_atomic_observed
       │ ◀────────────────────────────────── │  refund_amount or charge_amount
       │                                     │
       │ ──── OR ────────────────────────────│
       │           5'. RELEASE               │
       │ ──────────────────────────────────▶ │  (provider call aborted /
       │ ◀────────────────────────────────── │   client timed out / run cancelled)
       │                                     │
       │ ──── OR ────────────────────────────│
       │           ⌛  TTL EXPIRY              │  Authority auto-releases the
       │                                     │  Reservation per §3.2 grace rules
```

**Stage semantics:**

1. **Resolve** — given an agent's identity + intended call context, the Authority resolves which Budget the call binds against. Optional if the caller knows its Budget binding statically.
2. **Reserve** — the caller submits a worst-case Claim. The Authority makes a Decision. On `ALLOW` or `ALLOW_WITH_CAPS` it returns a Reservation with a TTL deadline by which `commit` or `release` MUST arrive.
3. **Call provider** — proceeds only when the Decision is `ALLOW` or `ALLOW_WITH_CAPS`. `DENY` MUST short-circuit before any provider request is initiated.
4. **Commit** — after the provider responds, the caller reports observed `amount_atomic_observed`. The Authority reconciles (see §3.1 for overage semantics).
5'. **Release** — if the provider call is aborted, the client times out, or the agent run is cancelled, the caller calls `release` to return the Reservation to the Budget before TTL. Emits a signed `audit.release` event.
6. **Audit** — every `reserve` / `commit` / `release` / `refund` outcome emits a signed CloudEvent. The audit chain is the durable record; in-memory Authority state is recoverable from the chain.

### 3.1 Commit and overage

Commit MUST be idempotent on `(reservation_id, idempotency_key)`. See §4 for the wire format.

If `amount_atomic_observed > amount_atomic_reserved`, the **default** behavior is:

> **REJECT** the commit. Authority emits an `audit.overage_rejected` event, the Reservation transitions to `QUARANTINED`, and the caller MUST treat the provider call as having occurred without budget coverage (operator policy: alert, page, or auto-credit from an overage budget). This default matches the pre-call reservation guarantee — an `ALLOW` decision cannot push a Budget past its cap.

Authorities MAY offer an opt-in `commit_overage_policy = CHARGE_OVERAGE` per-Budget that charges the excess instead of rejecting. When this policy is set, the Authority MUST emit `audit.overage_charged` and MUST surface the over-cap state immediately. This is an opt-in degradation of the pre-call guarantee and is documented per-Budget.

If `amount_atomic_observed < amount_atomic_reserved`, the difference is refunded (`refund_amount_atomic` set in the response). The signed audit event records both reserved and observed amounts.

### 3.2 TTL expiry and late commits

A Reservation has a TTL deadline. The window between `reserve` and `commit` is bounded.

When TTL is reached without `commit` or `release`:

1. The Authority emits `audit.ttl_expired`. The Reservation transitions to `EXPIRED`.
2. The capacity returns to the Budget — i.e. the Budget MAY grant new Reservations using that capacity from this point on.
3. A subsequent `commit` for the expired Reservation enters a defined **grace window** (Authority-configured, RECOMMENDED 30 s after TTL):
   - **Within grace:** Commit is honored, the spend is debited (even though the capacity was already returned, the Budget MAY go transiently over-cap; an `audit.late_commit` event is emitted). The over-cap state MUST surface to operator observability.
   - **Beyond grace:** Commit is rejected with `EXPIRED_BEYOND_GRACE`. The provider call has completed and the tokens have been billed by the provider, but the spend is not debited against this Budget. The caller is told to escalate via a separate `audit.reconciliation_gap` event so out-of-band accounting can record the gap.

The grace window is non-zero by design: at TTL the agent runtime usually has the provider response in hand and is microseconds away from commit. A zero-grace policy fails too many legitimate slow commits. A bounded grace prevents indefinite stretching.

Authorities MUST publish their grace window value via the Authority discovery endpoint (out of scope for Draft-01; SHOULD be ≤ 5 minutes).

### 3.3 Failure modes

- **Authority unreachable** — caller MUST fail-closed (deny the provider call). MAY fail-open under an explicit operator override flag (development only). The audit chain records nothing in fail-closed mode (no decision was rendered); fail-open mode emits a `audit.bypassed` event.
- **Replay attack on commit** — two distinct commits with the same `reservation_id` but conflicting `amount_atomic_observed` or `idempotency_key` are detected at the Authority. The Authority MUST reject the second commit with `REPLAY_CONFLICT` and emit `audit.replay_rejected`. See §4 for the wire-level idempotency contract.
- **Double-spend across Reservations** — Budget atomicity at the (Budget, window) granularity prevents two `reserve` operations from both succeeding past the cap. This is an Authority-internal guarantee; ASP requires it but does not prescribe the locking mechanism.

## 4. Wire messages

```protobuf
message ReserveRequest {
  BudgetClaim claim = 1;
  google.protobuf.Struct identity = 2;
  google.protobuf.Struct runtime_metadata = 3;
  string idempotency_key = 4;
}

message ReserveResponse {
  enum Decision {
    DECISION_UNSPECIFIED = 0;
    ALLOW = 1;
    DENY = 2;
    ALLOW_WITH_CAPS = 3;
    DEGRADE = 4;            // ASP extension; see §2
    REQUIRE_APPROVAL = 5;   // ASP extension; see §2
  }
  Decision decision = 1;
  string reservation_id = 2;
  google.protobuf.Timestamp ttl_expires_at = 3;
  repeated string reason_codes = 4;
  repeated string matched_rule_ids = 5;
  repeated AllowCap caps = 6;       // populated when decision = ALLOW_WITH_CAPS
  bytes audit_event_signature = 7;  // detached signature of the emitted
                                    // audit.reserve event for this Reserve
}

message CommitRequest {
  string reservation_id = 1;
  string amount_atomic_observed = 2;     // decimal string, in claim.unit
  google.protobuf.Struct provider_response_facts = 3;

  // Idempotency contract: the (reservation_id, idempotency_key) pair
  // is the dedup key. A second commit with the same pair but
  // conflicting amount_atomic_observed or provider_response_facts
  // is REPLAY_CONFLICT (see §3.3). A second commit with the same
  // pair and identical body is honored (returns the original
  // response).
  string idempotency_key = 4;

  // Hash of the canonicalized request body (excluding this field
  // and audit_event_signature). Authorities MAY require this to
  // detect tampering. RECOMMENDED but optional in Draft-01.
  bytes request_body_hash = 5;
}

message CommitResponse {
  string refund_amount_atomic = 1;
  string charge_amount_atomic = 2;
  bytes audit_event_signature = 3;
}

message ReleaseRequest {
  string reservation_id = 1;
  string idempotency_key = 2;
  repeated string reason_codes = 3;   // why released — provider_error,
                                      // client_timeout, run_cancelled, ...
}

message ReleaseResponse {
  bytes audit_event_signature = 1;
}
```

`Refund` (post-commit reversal) is defined symmetrically with the same idempotency contract; its shape is intentionally omitted from Draft-01 because the upstream `refund` verb is still at `proposed` and the SpendGuard binding wants two prior implementations to align with before committing wire details.

## 5. Audit Event envelope

ASP emits one CloudEvent (v1.0.2) per `reserve`, `commit`, `release`, `refund`, `ttl_expired`, `late_commit`, `overage_rejected`, `overage_charged`, `replay_rejected`, `bypassed`, and `reconciliation_gap` outcome.

**CloudEvent `type` discriminator:** issuer-prefixed, in the form `<authority-domain>.audit.<verb>`. Examples:

- `org.agentspend.audit.reserve`  (vendor-neutral reference prefix)
- `spendguard.audit.reserve`      (the SpendGuard reference implementation's prefix; see §8)
- `goodmeta.audit.authorize`      (an upstream implementer's prefix; see §1)

The prefix is purely a routing convenience for SIEMs that subscribe to specific issuers. The `verb` suffix (the part after the last `.audit.`) is the binding to the canonical `budget_reservation` verb set.

**Signing.** The signed payload is the CloudEvent's `data` field. ASP RECOMMENDS Ed25519 over the JCS (RFC 8785) canonical-JSON form of `data` for cross-implementation verification. Implementations whose wire is natively protobuf MAY sign the canonical protobuf encoding of `data` instead; verification across mixed implementations then requires a documented re-canonicalization, which is the cost of choosing a non-JCS form. See §8 for the reference implementation's current choice.

**Key management.** Every CloudEvent envelope MUST carry a `kid` (signing key identifier) in the `data` payload or as a CloudEvent extension attribute. The issuer MUST publish a JWKS document at a well-known URL discoverable from the issuer's domain. After key rotation, **previous verification keys MUST remain published** for at least the retention period of the audit chain they signed (RECOMMENDED ≥ 1 year). Without this, historical audit chains become unverifiable.

**Minimum decision_context fields:**

| Field | Type | Required | Notes |
|---|---|---|---|
| `decision_id` | UUID | ✓ | Stable across retries via `idempotency_key` |
| `budget_id` | string | ✓ | The Budget the Claim hit |
| `unit` | string | ✓ | e.g. `output_token`, `usd_atomic`, `request` |
| `amount_atomic_reserved` | decimal string | ✓ | The Claim |
| `decision` | enum | ✓ | per §2 |
| `kid` | string | ✓ | Signing key identifier |
| `reason_codes` | string[] | recommended | machine-readable rationale |
| `runtime_metadata` | Struct | optional | allowlisted scalar keys |

Provider-specific extensions (e.g. for LiteLLM: `litellm_call_id`, `model`, `team_id`, `pricing_version`, `price_snapshot_hash_hex`, `fx_rate_version`, `unit_conversion_version`, `call_type`, `stream`, `mode`, `integration`) are valid `runtime_metadata` keys and are bound by the signature like any other context field.

## 6. Compatibility

- **OpenTelemetry GenAI** — ASP composes with OTel GenAI by emitting span events on the GenAI span. The event names follow the [parallel OTel SIG proposal](../../proposals/otel-genai-spend-extension.md): `gen_ai.spend.reserve`, `gen_ai.spend.commit`, `gen_ai.spend.release`, `gen_ai.spend.audit`. Earlier drafts of this spec used the bare `asp.*` prefix; that has been retired in favor of the OTel-aligned names.
- **FOCUS 1.0** — `commit` observations SHOULD be exportable to FOCUS-compliant billing schemas for daily reconciliation against provider invoices. Mapping is one-way (FOCUS → ASP can't reconstruct decisions; ASP → FOCUS can produce a charge feed).
- **`crosswalk/budget_reservation.yaml`** — verbs and decision shape are the upstream canonical set. The `crosswalk/asp.yaml` crosswalk PR is planned for Draft-02.
- **APS / AgentID / x402 / ERC-8004** — accepted as inputs to `identity`. ASP does NOT replace identity layers and does NOT prescribe how `actor` is established.
- **CloudEvents 1.0.2** — wire envelope of choice. JSON serialization is normative; protobuf serialization permitted under the signing-format note in §5.

## 7. Open questions for v0.2

1. **Cross-Authority settlement** — when a call spans Authorities (e.g. tenant A's agent calls tenant B's tool), does Reserve cascade, or does each Authority hold its own Reservation? Draft-01 punts.
2. **Multi-provider atomic budgets** — a Budget capped in `usd_atomic` that funds calls across OpenAI + Anthropic + Bedrock needs FX + pricing-version pinning. The freeze schema is currently implementation-defined; should ASP standardize it?
3. **`ALLOW_WITH_CAPS` cap-type vocabulary** — upstream defers cap-type vocabulary to v0.2 of `budget_reservation.yaml`. ASP follows.
4. **`DEGRADE` routing-hint cap** — Draft-01 carries the route hint as a `degrade.route_to` cap inside `ALLOW_WITH_CAPS_structure.caps`. Open: should this be a standalone cap-type registered upstream?
5. **`Refund` wire details** — deliberately deferred until upstream `refund` has a second implementer.
6. **Quarantine across consumers** — when a downstream audit consumer can't verify an event's signature, what's the protocol-mandated handling? (SpendGuard's reference implementation quarantines; not all consumers will.)
7. **`request_body_hash` requirement level** — Draft-01 makes it RECOMMENDED; should it become MUST in Draft-02 once tooling for canonicalization exists?

## 8. Reference implementation — status and delta

[SpendGuard](https://github.com/m24927605/agentic-spendguard) (Apache-2.0) is a **partial reference implementation** of Draft-01 — partial meaning it implements the protocol's transaction model and audit chain but with three known deltas from the wire shape above. Both directions of work (spec revision to match SpendGuard, or SpendGuard revision to match spec) are in scope for Draft-02.

| Aspect | Spec (Draft-01) | SpendGuard reference impl today | Resolution path |
|---|---|---|---|
| CloudEvent `type` prefix | `<issuer-domain>.audit.<verb>` (e.g. `org.agentspend.audit.reserve`) | `spendguard.audit.decision`, `spendguard.audit.outcome` (legacy two-event taxonomy from before the per-verb event split) | SpendGuard will migrate to per-verb events under the `spendguard.audit.` prefix in a future point release; spec already permits issuer-prefixed names so the spec form is forward-compatible. |
| Audit signing format | JCS canonical JSON over `data`, Ed25519 | Canonical protobuf bytes over the proto event, Ed25519 | SpendGuard adds a JCS-form output alongside protobuf so cross-implementation verifiers don't need protobuf tooling. Tracked as a follow-up. |
| Commit lane | Single `Commit` RPC with `amount_atomic_observed` | Two lanes: `CommitObserved` (provider-reported usage) and `CommitEstimated` (callers without observed usage, e.g. failed streams). `CommitEstimated` is rejected if `estimated > original_reserved`. | SpendGuard's `CommitObserved` matches the spec; `CommitEstimated` is treated as a vendor extension for now. Spec may incorporate `CommitEstimated` as an optional fallback in Draft-02 if the use case generalizes. |

Adapters for LiteLLM, OpenAI Agents SDK, LangChain, LangGraph, Pydantic-AI, and Microsoft Agent Governance Toolkit ship today. The 12-field LiteLLM `decision_context` extension is implemented and live-verified per [GH #77](https://github.com/m24927605/agentic-spendguard/issues/77).

The reference implementation is **not** the protocol. This document describes the protocol; alternative implementations are encouraged and welcome to crosswalk against the upstream `budget_reservation` verb set.

## 9. Acknowledgements

- The category framing draws on conversations and prior art from Tymofii Pidlisnyi's APS work and the broader `agent-governance-vocabulary` project. The `crosswalk/budget_reservation.yaml` file made it possible to write ASP without inventing parallel terminology for `reserve`, `commit`, `release`, `refund`, and `query_budget`. goodmeta and Cycles are the two production implementers whose convergence put the verb set on the table.
- The pre-call-reservation pattern is borrowed wholesale from Stripe's auth/capture model for card payments.
- The audit-chain immutability discipline draws on prior work in financial-services double-entry bookkeeping and the CloudEvents conformance test suite.

## 10. Changelog

- **Draft-01** (2026-05-23) — initial public draft. Open for comment via GitHub issues at the repository above.

---

> Comments, corrections, and crosswalk PRs welcome at  
> [github.com/m24927605/agentic-spendguard/issues](https://github.com/m24927605/agentic-spendguard/issues).
