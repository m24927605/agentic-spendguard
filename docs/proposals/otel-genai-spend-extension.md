# OpenTelemetry GenAI extension proposal: spend-governance span events

> **Status:** Draft for discussion at the OpenTelemetry GenAI SIG.
> **Author:** SpendGuard authors (m24927605@gmail.com), 2026-05-23.
> **One-line summary:** Add four span events — `gen_ai.spend.reserve`, `gen_ai.spend.commit`, `gen_ai.spend.release`, `gen_ai.spend.audit` — and one attribute group (`gen_ai.spend.*`) so that pre-call budget-enforcement decisions become observable on the same GenAI span where the provider call already lives.

## Why this belongs in OTel GenAI semconv

The GenAI semantic conventions describe **what happened** during an LLM call (model, prompt tokens, completion tokens, span timing). A growing body of agent-runtime work — gateway-level budget controls (LiteLLM agent iteration budgets, Portkey budgets, Helicone limits), enterprise governance toolkits (Microsoft AGT), and pre-call enforcement protocols (the [Agent Spend Protocol draft](https://github.com/m24927605/agentic-spendguard/blob/main/docs/specs/agent-spend-protocol/draft-01.md)) — produces a distinct class of events that don't fit any existing OTel attribute group:

- A *decision* (ALLOW / ALLOW_WITH_CAPS / DENY) made **before** the provider call, with its own rationale and rule trace.
- A *reservation* (a held capacity claim) with a TTL deadline.
- A *commit* (reconciliation of observed usage against the reservation) emitted **after** the provider response.

These are operationally critical: when a tenant's budget hits a wall at 02:47 UTC, the SRE on call needs to find the DENY decisions on the same trace as the cascading retry attempts, not in a separate observability silo.

Without semconv coverage, every vendor's enforcement decisions go on the GenAI span as ad-hoc `attributes["my_vendor.budget.decision"] = "DENY"` strings, defeating the purpose of having semantic conventions in the first place.

## Proposed additions

### Four span events

| Event name | Emitted when | Required attributes |
|---|---|---|
| `gen_ai.spend.reserve` | Before the provider call, when an enforcement authority returns a decision | `gen_ai.spend.decision`, `gen_ai.spend.decision_id`, `gen_ai.spend.budget_id`, `gen_ai.spend.unit`, `gen_ai.spend.amount_atomic_reserved` |
| `gen_ai.spend.commit` | After the provider call, when observed usage is reconciled against the reservation | `gen_ai.spend.decision_id`, `gen_ai.spend.amount_atomic_observed`; plus `gen_ai.spend.refund_amount_atomic` (when observed < reserved) *or* `gen_ai.spend.charge_amount_atomic` (when observed > reserved). When observed equals reserved exactly, both fields are absent and the consumer infers exact-match from `amount_atomic_observed == amount_atomic_reserved` (which is on the reserve event for the same `decision_id`). |
| `gen_ai.spend.release` | When the provider call is aborted, the client times out, or the agent run is cancelled, and the held reservation is returned to the budget before TTL | `gen_ai.spend.decision_id`, `gen_ai.spend.reason_codes` |
| `gen_ai.spend.audit` | When the enforcement authority emits a signed audit record (typically asynchronous from the GenAI span) | `gen_ai.spend.decision_id`, `gen_ai.spend.audit_event_signature_hash` |

The provider call's own span attributes (`gen_ai.usage.input_tokens`, `gen_ai.usage.output_tokens`, `gen_ai.request.model`, …) remain unchanged. The new events sit alongside them on the same span; correlation between the provider call and the enforcement decision is by trace identity, not by ad-hoc key matching.

### Attribute group: `gen_ai.spend.*`

| Attribute | Type | Required | Brief |
|---|---|---|---|
| `gen_ai.spend.decision` | string enum | ✓ on `reserve` event | `allow` / `allow_with_caps` / `deny` (the canonical decision set from upstream `budget_reservation.yaml`). The "degrade" pattern is conveyed as `allow_with_caps` plus a `degrade.route_to` cap, not as a separate enum value. Human-in-the-loop approval is conveyed as `deny` with a `reason_codes` entry like `"approval_required"` and is otherwise out of scope for Draft-01 of the underlying ASP draft. |
| `gen_ai.spend.decision_id` | string | ✓ | Stable identifier tying `reserve` ↔ `commit` ↔ `audit` |
| `gen_ai.spend.budget_id` | string | ✓ on `reserve` | Opaque to OTel; e.g. `tenant-3:2026-05:output_token` |
| `gen_ai.spend.unit` | string | ✓ on `reserve` | e.g. `output_token`, `usd_atomic`, `request` |
| `gen_ai.spend.amount_atomic_reserved` | string | ✓ on `reserve` | Decimal string in `unit` |
| `gen_ai.spend.amount_atomic_observed` | string | ✓ on `commit` | Decimal string in `unit` |
| `gen_ai.spend.refund_amount_atomic` | string | conditional | Set only when observed < reserved; absent on exact-match or overage commits |
| `gen_ai.spend.charge_amount_atomic` | string | conditional | Set only when observed > reserved; absent on exact-match or under-reserved commits |
| `gen_ai.spend.reason_codes` | string[] | recommended | Machine-readable rationale for the decision |
| `gen_ai.spend.matched_rule_ids` | string[] | optional | Rule identifiers that fired (vendor-specific) |
| `gen_ai.spend.authority` | string | recommended | URL or opaque id of the enforcement authority |
| `gen_ai.spend.audit_event_signature_hash` | string | ✓ on `audit` | Hex-encoded SHA-256 of the detached signature, for pinning audit chains to traces |

`gen_ai.spend.decision_id` is the join key between the events on a span and any out-of-band audit log a vendor maintains. We deliberately do **not** prescribe how that audit log is structured — that's the job of [a separate spec](https://github.com/m24927605/agentic-spendguard/blob/main/docs/specs/agent-spend-protocol/draft-01.md).

## Worked example

```
span: "openai.chat.completions"
├── attributes:
│     gen_ai.system = "openai"
│     gen_ai.request.model = "gpt-4o"
│     gen_ai.usage.input_tokens = 412
│     gen_ai.usage.output_tokens = 87
├── event: gen_ai.spend.reserve (t+0ms)
│     gen_ai.spend.decision = "allow"
│     gen_ai.spend.decision_id = "01J5..."
│     gen_ai.spend.budget_id = "acme-eng:2026-05:output_token"
│     gen_ai.spend.unit = "output_token"
│     gen_ai.spend.amount_atomic_reserved = "200"
│     gen_ai.spend.authority = "https://sg.acme.internal"
├── event: gen_ai.spend.commit (t+1840ms)
│     gen_ai.spend.decision_id = "01J5..."
│     gen_ai.spend.amount_atomic_observed = "87"
│     gen_ai.spend.refund_amount_atomic = "113"
└── event: gen_ai.spend.audit (t+1842ms)
      gen_ai.spend.decision_id = "01J5..."
      gen_ai.spend.audit_event_signature_hash = "9f2c…"
```

DENY case:

```
span: "openai.chat.completions"
├── status: ERROR (the provider call was not made)
├── attributes:
│     gen_ai.system = "openai"
│     gen_ai.request.model = "gpt-4o"
│     (no gen_ai.usage.* — provider never called)
├── event: gen_ai.spend.reserve (t+0ms)
│     gen_ai.spend.decision = "deny"
│     gen_ai.spend.decision_id = "01J5..."
│     gen_ai.spend.budget_id = "acme-eng:2026-05:output_token"
│     gen_ai.spend.unit = "output_token"
│     gen_ai.spend.amount_atomic_reserved = "200"
│     gen_ai.spend.reason_codes = ["BUDGET_EXHAUSTED"]
│     gen_ai.spend.authority = "https://sg.acme.internal"
└── event: gen_ai.spend.audit (t+2ms)
      gen_ai.spend.decision_id = "01J5..."
      gen_ai.spend.audit_event_signature_hash = "1a8b…"
```

The SRE filter on a 02:47 incident dashboard becomes: `gen_ai.spend.decision = "deny" AND service.name = "checkout-agent"` — one trace query, no separate budget-events stream.

## Open questions for the SIG

1. **Naming** — is `gen_ai.spend.*` the right group prefix, or does `gen_ai.governance.spend.*` better future-proof for adjacent governance event groups (content moderation, rate gating, data classification)?
2. **Span vs. metric** — should `gen_ai.spend.commit` ALSO emit a metric (`gen_ai.spend.committed_total{decision=allow}`), or strictly stay event-level? Metrics give dashboards cheap; events give traces deep correlation. Both is possible.
3. **`decision_id` cardinality** — high (one per call). Acceptable per existing GenAI semconv precedent (`gen_ai.response.id`), but worth confirming.
4. **Degrade-cap routing hint** — the ASP draft folds the "DEGRADE" pattern into `ALLOW_WITH_CAPS` with a `degrade.route_to` cap. Should this OTel extension surface the routed model as a structured attribute (e.g. `gen_ai.spend.cap.degrade.route_to.model = "gpt-4o-mini"`), or leave it embedded in the cap payload and let the GenAI span pick up the routed model via the actual provider call's `gen_ai.request.model`?
5. **Stability tier** — propose **experimental** for the first release.

## Relationship to other emerging work

This proposal is the **observability extension** of a broader pattern:

- The [Agent Spend Protocol (ASP) Draft-01](https://github.com/m24927605/agentic-spendguard/blob/main/docs/specs/agent-spend-protocol/draft-01.md) defines the wire protocol between agents and authorities (Reserve / Commit / Audit semantics, transaction model, signature discipline).
- This OTel extension defines how those events surface on the GenAI span so existing OTel-based dashboards, traces, and SLOs can consume them without adopting ASP.

A runtime can implement OTel GenAI + this extension without implementing ASP — for instance, by emitting the events from a vendor-specific gateway-budget feature (LiteLLM iteration budgets, Portkey budgets). Conversely, an ASP-compliant runtime can choose to NOT emit OTel spans. The extension and ASP are decoupled by design.

## Asking the SIG for

1. Acknowledgement that this is in-scope for OTel GenAI semconv (vs. needs to be a sibling SIG).
2. Naming and stability-tier guidance.
3. A review window — happy to bring this to a SIG meeting once the agenda allows.

Comments + redirects welcome.
