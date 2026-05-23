# Agent Spend Protocol — Draft 01

> **Status:** Draft-01, 2026-05-23. Active revision.
> **Editor:** SpendGuard authors (m24927605@gmail.com).
> **License:** Apache-2.0. Public-domain protocol sketch — not a SpendGuard-specific binding.
> **Repository:** [github.com/m24927605/agentic-spendguard/blob/main/docs/specs/agent-spend-protocol/draft-01.md](https://github.com/m24927605/agentic-spendguard/blob/main/docs/specs/agent-spend-protocol/draft-01.md)
> **Upstream alignment:** SpendGuard binding of the [`crosswalk/budget_reservation.yaml`](https://github.com/aeoess/agent-governance-vocabulary/blob/main/crosswalk/budget_reservation.yaml) canonical-verb-set (`status: INCUBATING`, `crosswalk_type: domain_incubation`, two production implementations as of 2026-05-13). SpendGuard adoption is intended to surface the third production implementation that promotes the candidate verbs (`reserve`, `commit`, `query_budget`) toward canonical, and bring `release` + `refund` to the two-implementation threshold. Upstream `query_reservation` (also `proposed`) is out of scope for ASP Draft-01; SpendGuard does not implement a per-reservation query verb today.

## Abstract

The Agent Spend Protocol (ASP) defines a wire-level contract between an LLM agent (or agent runtime) and a budget-enforcement authority, enabling **pre-call budget reservation**, **post-call usage reconciliation**, and **signed audit emission** for every provider call an agent attempts. ASP is provider-neutral and framework-neutral: any agent runtime that wants to gate spend before the provider clock starts — instead of after the bill arrives — can implement ASP against any enforcement authority that speaks it.

This document is the **agent-runtime binding** of the upstream `budget_reservation` canonical-verb domain that is currently incubating in [`aeoess/agent-governance-vocabulary`](https://github.com/aeoess/agent-governance-vocabulary). The verb set (`reserve`, `commit`, `release`, `refund`, `query_budget`) is reused verbatim. The decision enum (`ALLOW`, `ALLOW_WITH_CAPS`, `DENY`) is reused verbatim with one agent-runtime extension (`REQUIRE_APPROVAL`) documented in §2; the common "DEGRADE" pattern is folded into `ALLOW_WITH_CAPS` rather than being a separate enum value, also per §2.

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
- Upstream `query_reservation` (`proposed`, one implementer with the verb multiplexed via `query`) is **deliberately out of scope** for Draft-01. SpendGuard does not yet expose a per-reservation read; if it adds one, Draft-02 will document the binding and the crosswalk will reflect it.

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

**Decision values.** The wire-level decision enum is the upstream canonical set verbatim plus one ASP-runtime-specific addition:

| Decision | Source | Meaning |
|---|---|---|
| `ALLOW` | upstream canonical | Reserve granted in full; provider call MAY proceed. |
| `ALLOW_WITH_CAPS` | upstream canonical | Reserve granted with structured caps (`{type, params}[]`) the caller MUST honor. ASP defers cap-type vocabulary to upstream v0.2 (`ALLOW_WITH_CAPS_structure`). |
| `DENY` | upstream canonical | Reserve refused. Provider call MUST NOT proceed. |
| `REQUIRE_APPROVAL` | **ASP extension** | Reserve held pending human-in-the-loop approval. Returns an `approval_request_id`; subsequent Commit MUST first poll approval status. Structurally distinct from ALLOW_WITH_CAPS because the caller is held rather than proceeding-with-constraints; therefore a distinct enum value rather than a cap type. Marked extension because it is agent-runtime-specific (rails don't hold for approval). |

**Cap-honoring contract.** When the Authority returns `ALLOW_WITH_CAPS`:

1. The caller MUST honor every cap in the returned `caps[]` list according to the cap's `type` and `params`.
2. The caller MUST treat the decision as `DENY` and refuse to proceed if **any** `caps[].type` is unknown to the caller, cannot be applied (e.g. the caller's request shape does not allow the requested modification), or is reported by the caller's local cap-registry as deprecated. Fail-closed is the default; there is no "ignore unknown caps and proceed" path.
3. Authorities MUST publish their supported cap-type registry (URL discoverable from the Authority's metadata endpoint, out of scope for Draft-01). Callers MUST publish or document the cap types they recognize.

**"DEGRADE" pattern.** The common agent-runtime case where the Authority refuses the requested call but offers a cheaper-route alternative (smaller model, reduced context) is **not** a separate decision value. It is the `ALLOW_WITH_CAPS` decision with `caps = [{type: "degrade.route_to", params: {model: "...", max_tokens: N, ...}}]` and `reason_codes` including `"degrade"`. Because of the fail-closed contract above, callers that don't recognize the `degrade.route_to` cap type correctly refuse the call instead of proceeding without honoring the constraint — that is the entire safety argument for collapsing DEGRADE into ALLOW_WITH_CAPS.

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

### 3.3 Failure modes and reservation lifecycle

A Reservation's lifecycle is one-shot. `reservation_id` MAY be used in at most one terminal state transition (commit, release, expired-beyond-grace, or quarantine). After the terminal state is reached:

- A subsequent `commit` against a committed reservation with the **same `(reservation_id, idempotency_key)` pair and identical request body** is treated as an idempotent retry: the Authority returns the original `CommitResponse` without re-running settlement and without emitting a new audit event. This is the standard network-retry safety net.
- A subsequent `commit` against a committed reservation with a **different `idempotency_key` or a conflicting request body** MUST be rejected with `RESERVATION_SETTLED`. The Authority emits `audit.replay_rejected` with reason `reservation_already_settled`.
- A subsequent `commit` against a released reservation MUST be rejected with `RESERVATION_RELEASED`. A commit against an expired-beyond-grace reservation MUST be rejected with `EXPIRED_BEYOND_GRACE`.
- A subsequent `release` against a committed reservation is a no-op (return success without state change). A release against an already-released reservation with the same `idempotency_key` returns the original response; with a different `idempotency_key` returns success-no-op as well — release-after-release is harmless.

The `idempotency_key` disambiguates retries **within** a single open reservation lifecycle, and lets the Authority distinguish "same caller retrying its own request" from "different caller attempting a fresh settlement". It does not unlock new terminal states for a settled reservation.

**TTL expiry is not strictly terminal during the grace window.** Per §3.2, a `ttl_expired` audit event MAY be followed by a `late_commit` within the grace window. The reservation's logical state during grace is `EXPIRED_IN_GRACE`; the truly terminal post-TTL state (`EXPIRED_BEYOND_GRACE`) is only reached after the grace window closes without a Commit. Implementations MUST emit `audit.ttl_expired` at the TTL boundary regardless of whether a late commit eventually arrives — the boundary is a real event in the audit chain.

Other failure modes:

- **Authority unreachable** — caller MUST fail-closed (deny the provider call). MAY fail-open under an explicit operator override flag (development only). The audit chain records nothing in fail-closed mode (no decision was rendered); fail-open mode emits an `audit.bypassed` event.
- **Replay attack on commit (same reservation, conflicting body)** — two distinct commits with the same `(reservation_id, idempotency_key)` pair but conflicting `amount_atomic_observed` or `provider_response_facts` are detected at the Authority. The Authority MUST reject the second commit with `REPLAY_CONFLICT` and emit `audit.replay_rejected` with reason `body_mismatch`. See §4 for the wire-level idempotency contract.
- **Double-spend across Reservations** — Budget atomicity at the (Budget, window) granularity prevents two `reserve` operations from both succeeding past the cap. This is an Authority-internal guarantee; ASP requires it but does not prescribe the locking mechanism.

## 4. Wire messages

```protobuf
// Common types referenced by RPC messages below.

message BudgetClaim {
  string budget_id = 1;             // opaque, scoped to issuer
  string window_instance_id = 2;    // billing window the claim hits
  string unit = 3;                  // e.g. "output_token", "usd_atomic", "request"
  string amount_atomic = 4;         // decimal string in `unit`
  enum Direction {
    DIRECTION_UNSPECIFIED = 0;
    DEBIT = 1;
    CREDIT = 2;
  }
  Direction direction = 5;
}

message AllowCap {
  string type = 1;                  // cap-type vocabulary (see §2)
  google.protobuf.Struct params = 2; // cap-type-specific parameters
}

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
    REQUIRE_APPROVAL = 4;   // ASP extension; see §2
    // The "DEGRADE" pattern is not a distinct decision value;
    // it is ALLOW_WITH_CAPS with a `degrade.route_to` cap.
  }
  Decision decision = 1;
  string reservation_id = 2;
  google.protobuf.Timestamp ttl_expires_at = 3;
  repeated string reason_codes = 4;
  repeated string matched_rule_ids = 5;
  repeated AllowCap caps = 6;       // populated when decision = ALLOW_WITH_CAPS
  string approval_request_id = 7;   // populated when decision = REQUIRE_APPROVAL
  bytes audit_event_signature = 8;  // detached signature of the emitted
                                    // audit.reserve event for this Reserve
}

message CommitRequest {
  string reservation_id = 1;
  string amount_atomic_observed = 2;     // decimal string, in claim.unit
  google.protobuf.Struct provider_response_facts = 3;

  // Idempotency contract (see §3.3 for full lifecycle):
  //   - Same (reservation_id, idempotency_key) pair + identical body
  //     → idempotent retry; Authority returns the original
  //       CommitResponse, no new audit event.
  //   - Same pair + conflicting amount_atomic_observed or
  //     provider_response_facts → REPLAY_CONFLICT,
  //     audit.replay_rejected with reason "body_mismatch".
  //   - Different idempotency_key against an already-committed
  //     reservation_id → RESERVATION_SETTLED, audit.replay_rejected
  //     with reason "reservation_already_settled".
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

**CloudEvent `type` discriminator:** issuer-prefixed, in the form `<authority-domain>.audit.<suffix>`. The prefix is a routing convenience for SIEMs subscribing to specific issuers. The suffix is one of two disjoint sets:

1. **Canonical-verb outcomes** (one event per successful verb invocation, suffix is the upstream `budget_reservation` verb name verbatim):
   - `<issuer>.audit.reserve`
   - `<issuer>.audit.commit`
   - `<issuer>.audit.release`
   - `<issuer>.audit.refund`

2. **ASP-defined outcome events** (suffixes registered in this section, NOT canonical verbs — they exist because real authorities have outcomes that are not 1:1 with the upstream verb verbs):
   - `<issuer>.audit.ttl_expired` — TTL reached without Commit or Release
   - `<issuer>.audit.late_commit` — Commit honored within grace window after TTL (§3.2)
   - `<issuer>.audit.overage_rejected` — Commit's `amount_atomic_observed` exceeded reservation, default policy
   - `<issuer>.audit.overage_charged` — Commit's overage charged under opt-in `CHARGE_OVERAGE` policy
   - `<issuer>.audit.replay_rejected` — Commit with conflicting body for same `(reservation_id, idempotency_key)` pair
   - `<issuer>.audit.bypassed` — fail-open mode let a call through without a decision (development only)
   - `<issuer>.audit.reconciliation_gap` — Commit rejected beyond grace; out-of-band accounting required

Examples:
- `org.agentspend.audit.reserve`  (vendor-neutral reference prefix)
- `spendguard.audit.reserve`      (the SpendGuard reference implementation's prefix; see §8)
- `goodmeta.audit.reserve`        (upstream `goodmeta` implementer post-AP2#252 rename; see §1)
- `spendguard.audit.ttl_expired`  (ASP-defined outcome under SpendGuard prefix)

Suffixes outside both sets are not valid ASP CloudEvent types. Adding a new outcome suffix requires a Draft revision.

**Issuer identity and JWKS discoverability.** The CloudEvent envelope's `source` attribute (a CloudEvents 1.0 normative field) MUST be set to a URL whose host is the issuer's domain. Verifiers derive the JWKS URL by appending `/.well-known/asp-jwks.json` to the `source` URL's origin. The `kid` field inside `data` selects the specific key in the JWKS. Example: `source = "https://sg.acme.internal/asp"` → JWKS at `https://sg.acme.internal/.well-known/asp-jwks.json`. Issuers whose prefix is not a domain name (e.g. the bare `spendguard` examples above) MUST still set `source` to a discoverable URL — the `type` prefix is a routing convenience, not an identity assertion.

**Signing.** The signed payload is the CloudEvent's `data` field. ASP RECOMMENDS Ed25519 over the JCS (RFC 8785) canonical-JSON form of `data` for cross-implementation verification. Implementations whose wire is natively protobuf MAY sign the canonical protobuf encoding of `data` instead; verification across mixed implementations then requires a documented re-canonicalization, which is the cost of choosing a non-JCS form. See §8 for the reference implementation's current choice.

**Key management.** Every CloudEvent envelope MUST carry a `kid` (signing key identifier) in the `data` payload or as a CloudEvent extension attribute. The issuer MUST publish a JWKS document at a well-known URL discoverable from the issuer's domain. After key rotation, **previous verification keys MUST remain published** for at least the retention period of the audit chain they signed (RECOMMENDED ≥ 1 year). Without this, historical audit chains become unverifiable.

**Minimum `data` fields per event type.**

Common to all event types (signed):

| Field | Type | Required | Notes |
|---|---|---|---|
| `decision_id` | UUID | ✓ | Stable across retries via `idempotency_key` |
| `kid` | string | ✓ | Signing key identifier; selects key in the issuer's JWKS |
| `event_time` | RFC 3339 timestamp | ✓ | Authority-clock time of event emission |
| `reason_codes` | string[] | recommended | machine-readable rationale |
| `runtime_metadata` | Struct | optional | allowlisted scalar keys |

Additional fields per suffix:

| Suffix | Required additional fields | Notes |
|---|---|---|
| `audit.reserve` | `budget_id`, `unit`, `amount_atomic_reserved`, `decision`, `ttl_expires_at` (if decision ∈ {ALLOW, ALLOW_WITH_CAPS}), `caps` (if decision = ALLOW_WITH_CAPS), `approval_request_id` (if decision = REQUIRE_APPROVAL) | Decision-context capture per §2 |
| `audit.commit` | `reservation_id`, `amount_atomic_observed`, `refund_amount_atomic` *or* `charge_amount_atomic` *or* `exact_match: true` | Exact-match commits where observed equals reserved set `exact_match: true` instead of `refund`/`charge` |
| `audit.release` | `reservation_id` | Reason for release goes in `reason_codes` |
| `audit.refund` | `reservation_id`, `amount_atomic_refunded`, `original_commit_event_id` | Post-commit reversal |
| `audit.ttl_expired` | `reservation_id`, `ttl_expires_at`, `capacity_returned_atomic` | Auto-release at TTL |
| `audit.late_commit` | `reservation_id`, `amount_atomic_observed`, `grace_window_ms_used`, `over_cap_amount_atomic` (if budget went over-cap) | Honored within grace window per §3.2 |
| `audit.overage_rejected` | `reservation_id`, `amount_atomic_observed`, `amount_atomic_reserved`, `overage_amount_atomic` | Default overage policy |
| `audit.overage_charged` | `reservation_id`, `amount_atomic_observed`, `amount_atomic_reserved`, `overage_amount_atomic`, `policy: "charge_overage"` | Opt-in overage policy |
| `audit.replay_rejected` | `reservation_id`, `idempotency_key`, `conflict_field` (which body field disagreed) | Per §3.3 |
| `audit.bypassed` | `bypass_reason` (`authority_unreachable`, `fail_open_override`), synthesized `reservation_id` permitted | Fail-open mode (dev only) |
| `audit.reconciliation_gap` | `reservation_id`, `amount_atomic_observed`, `time_past_grace_ms` | Out-of-band accounting required |

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

[SpendGuard](https://github.com/m24927605/agentic-spendguard) (Apache-2.0) is a **partial reference implementation** of Draft-01 — partial meaning it implements the protocol's transaction model and audit chain but with four known deltas from the wire shape above. Both directions of work (spec revision to match SpendGuard, or SpendGuard revision to match spec) are in scope for Draft-02.

| Aspect | Spec (Draft-01) | SpendGuard reference impl today | Resolution path |
|---|---|---|---|
| CloudEvent `type` discriminator | Per-verb / per-outcome under `<issuer>.audit.<suffix>` per §5 | Two-event legacy taxonomy: `spendguard.audit.decision` (covers Reserve outcomes) and `spendguard.audit.outcome` (covers Commit + Release outcomes); single types carry the verb in the `data` payload instead of the CloudEvent `type` suffix | SpendGuard migrates to per-suffix events under the `spendguard.audit.*` prefix in a future point release. Spec already permits issuer-prefixed names so the spec form is forward-compatible. |
| Audit signing format | JCS canonical JSON over `data`, Ed25519 | Canonical protobuf bytes over the proto event, Ed25519 | SpendGuard adds a JCS-form output alongside protobuf so cross-implementation verifiers don't need protobuf tooling. Tracked as a follow-up. |
| Commit lane | Single `Commit` RPC carrying `amount_atomic_observed` (provider-reported usage) | Only `CommitEstimated` is implemented today: `services/sidecar/src/decision/transaction.rs:run_commit_estimated` rejects non-empty `provider_reported_amount_atomic` with "ProviderReport path is deferred to a future slice". `adapter_uds.rs` routes all successful LLM post events through that estimated lane. So **SpendGuard does not yet implement the spec's observed-amount commit**; that is a SpendGuard backlog item, not a spec-vs-impl framing difference. | SpendGuard adds the observed-amount commit path. Until then, callers reconcile estimated reservations through the existing `CommitEstimated` RPC; this is an interop limitation against any future ASP-only consumer. |
| Release wire shape | `ReleaseRequest { reservation_id, idempotency_key, reason_codes }`, response carries only the signed audit event | SpendGuard's ledger-tier `ReleaseRequest` carries `reservation_set_id`, structured `Idempotency`, `Fencing`, `audit_event`, `decision_id`, `producer_sequence`. The adapter-facing Release path wraps this internally and returns success / replay / error. | SpendGuard exposes a Draft-01-shaped Release RPC at the adapter UDS boundary (the richer internal shape stays internal). Tracked as a follow-up. |

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
