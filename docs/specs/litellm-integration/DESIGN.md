# LiteLLM ⇄ Agentic SpendGuard Integration — DESIGN.md

> Status: Proposed (doc-first; no code lands until all 5 specs accepted)
> Owner: Platform / Integrations
> Related: `sdk/python/src/spendguard/integrations/agt.py` (shape we mimic),
> `auto-instrument-egress-proxy-spec.md` (the HTTP layer we may chain through),
> `cost-advisor-spec.md` (proposal write-path; orthogonal here).

---

## 1. Problem & Users

### 1.1 Who adopts this

LiteLLM is the most-deployed open-source LLM gateway: a single `litellm.completion()`
surface that fronts 100+ providers and ships a proxy server (`litellm --config
proxy_config.yaml`) for multi-tenant API-key fanout. Two adopter shapes:

- **App developers** using `litellm.completion()` / `litellm.acompletion()`
  directly inside an agent. They want a fail-closed dollar budget that survives
  process crashes, retries, and parallel `asyncio.gather()` fanout.
- **Platform / ops teams** running the LiteLLM proxy as the per-team egress
  point. They want hard caps per `team_id` / `key_alias` that cannot be
  bypassed when a worker dies mid-request, and they want a cryptographic
  audit trail of "who spent what, when, against which budget, and which
  decision allowed it."

### 1.2 The gap LiteLLM alone leaves

LiteLLM has a `LiteLLM_BudgetTable` (key/team budgets, max parallel requests,
RPM/TPM limits) that is **best-effort**: tracking lives in-process behind a
DualCache (in-memory + optional Redis), counters are decremented after the
call returns, and multiple proxy workers race on the same key. A budget can
overrun by `N_workers × concurrent_inflight × per_call_cost` because there is
no two-phase reserve/commit. There is also no signed event chain — only
JSON rows in `LiteLLM_SpendLogs`, which the operator (or anyone with DB
write access) can rewrite.

SpendGuard fills four gaps:

| Gap | LiteLLM today | SpendGuard adds |
|---|---|---|
| Hard cap under concurrency | Race-prone post-call decrement | Pre-call **reserve**, post-call **commit/release** (Stripe auth/capture) |
| Multi-worker correctness | DualCache + Redis; eventual | Single-writer-per-budget Postgres ledger; serialised |
| Audit integrity | Mutable rows | `canonical_events` hash-chain, no rewrites |
| Approval / require-approval | Not a primitive | `REQUIRE_APPROVAL` outcome with frozen pricing |

We are explicitly **not** competing with LiteLLM's routing, fallbacks, or
provider abstraction. We layer on top.

---

## 2. Goals & Non-Goals

### 2.1 Goals (v1)

- **G1.** Drop-in budget enforcement for users already on LiteLLM, with at
  most three config changes (env var + `litellm_settings.callbacks` + a
  budget-id mapping).
- **G2.** Fail-closed: if the sidecar is unreachable, the LLM call is
  denied (configurable, default deny).
- **G3.** Cover async `litellm.acompletion()` (in-process) and the proxy
  server (multi-worker, multi-tenant). Sync `litellm.completion()` is
  explicitly out of scope per ADR-005 / NG6; sync users are directed to
  Shape A or async migration.
- **G4.** Audit-chain coverage: every LiteLLM call that hits the wire
  produces a `canonical_events` row that joins to LiteLLM's `request_id`.
- **G5.** A real end-to-end demo: `DEMO_MODE=litellm_real` runs the
  LiteLLM proxy + SpendGuard sidecar + Postgres against a real (or
  recorded) provider. **Codex ✅ is not enough; the demo must run.**

### 2.2 Non-goals (v1)

- **NG1.** We do **not** replace LiteLLM's routing, fallback, or model-alias
  resolution. Routing decisions stay in LiteLLM.
- **NG2.** We do **not** rewrite LiteLLM's `LiteLLM_SpendLogs` — those keep
  working; SpendGuard's ledger lives alongside.
- **NG3.** We do **not** ship a SpendGuard fork of LiteLLM. The integration
  is a pip-installable `spendguard-sdk[litellm]` extra.
- **NG4.** Tool-call budgets (per-tool BudgetClaim) are out of scope here —
  that is the `agt.py` / `openai_agents.py` surface. LiteLLM integration is
  scoped to **token + dollar** claims on the LLM call itself.
- **NG5.** Streaming token accounting at chunk granularity is deferred to
  v2 (we commit on stream completion, parity with egress-proxy v0.2 SSE).
- **NG6.** Sync `litellm.completion()` is out of scope (ADR-005). Sync
  users are routed to Shape A (egress-proxy) or instructed to migrate
  to `acompletion()`.

---

## 3. Integration Shapes — Trade-off Matrix

Three plausible shapes. Each is described, evaluated, and a recommendation
follows.

### 3.1 Shape A — LiteLLM client → SpendGuard egress proxy chain

```
litellm.acompletion(api_base="http://localhost:8088/v1")
        │
        ▼
SpendGuard egress proxy (existing)  ──► OpenAI / Anthropic
        │
        ▼
Postgres ledger + canonical_events
```

User sets `litellm.api_base = "http://localhost:8088/v1"` (or per-call
`api_base=...`). LiteLLM speaks OpenAI to our existing egress proxy, which
already handles reserve/commit/SSE.

**Pros**

- Zero new code path on the SpendGuard side; reuses `auto-instrument-egress-proxy`
  and `egress-proxy-v0.2-streaming-sse` work that already shipped.
- Works for **both** `litellm.completion()` and the LiteLLM proxy
  (proxy can be told to forward to our proxy as the upstream).
- One audit-chain row per HTTP call, no double-counting.

**Cons (named trade-offs)**

- **Latency:** adds one local HTTP hop per call (~1–3 ms loopback).
- **Routing fidelity:** only works for providers SpendGuard's egress proxy
  speaks (OpenAI + Responses API today). Anthropic, Gemini, Bedrock are
  **out of scope** until egress-proxy gains those surfaces.
- **Identity loss:** the egress proxy sees `team_id` / `key_alias` only
  if LiteLLM forwards them as headers — we do not get LiteLLM's rich
  `UserAPIKeyAuth` object.
- **Deny semantics:** HTTP 429 / 402 from the proxy is what LiteLLM sees;
  the LiteLLM client surface returns a `BadRequestError`, not a typed
  `DecisionDenied`. Operator UX is "what does 402 mean here?".

### 3.2 Shape B — LiteLLM CustomLogger callback (in-process gate)

```
litellm.acompletion(...)
        │
        ▼  async_pre_call_hook → SpendGuardCallback
SpendGuard sidecar (UDS gRPC) ── reserve ──┐
        │                                  │
        ▼                                  ▼
Upstream LLM provider          Ledger + canonical_events
        │
        ▼  async_log_success_event → commit
SpendGuard sidecar (UDS gRPC) ── commit ──┘
```

Implement `litellm.integrations.custom_logger.CustomLogger`:

- `async_pre_call_hook(...)` → call `SpendGuardClient.request_decision(
  trigger="LLM_CALL_PRE", ...)`. On DENY, **raise** `DecisionDenied`
  (LiteLLM treats raised exceptions as "block the call and surface to
  client").
- `async_log_success_event(...)` → `client.emit_llm_call_post(outcome=
  "SUCCESS", ...)` with the real `response_obj.usage` token counts.
- `async_log_failure_event(...)` → `client.emit_llm_call_post(outcome=
  "FAILURE" | "CANCELLED", ...)` to release the reservation.

**Pros (named trade-offs)**

- **Provider-agnostic:** works for Anthropic, Gemini, Bedrock, Cohere,
  anything LiteLLM speaks — we never touch the wire.
- **Rich identity:** `user_api_key_dict` gives us `team_id`, `key_alias`,
  `user_id` — we can map them directly to SpendGuard `budget_id` /
  `window_instance_id`.
- **Typed deny:** `DecisionDenied` propagates as a LiteLLM-recognised
  exception; clients see the actual `reason_codes`.
- **Native to LiteLLM:** registered via `litellm_settings.callbacks`,
  feels like a first-class plugin.

**Cons (named trade-offs)**

- **Streaming commit timing:** `async_log_success_event` fires only after
  the full response is consumed. For SSE this can be 30s+ later — the
  reservation TTL must cover the longest stream, and a client crash
  mid-stream leaks the reservation until TTL sweep.
- **Per-worker registration:** every proxy worker registers its own
  callback; the sidecar's single-writer-per-budget invariant is what
  prevents races, **not** LiteLLM. Wins iff sidecar is correct (it is).
- **No HTTP-layer view:** if a user bypasses LiteLLM (calls OpenAI
  directly), this shape misses it. Shape A catches that; this does not.

### 3.3 Shape C — Composite gateway (LiteLLM proxy → SpendGuard sidecar)

Run LiteLLM proxy with **both** Shape A (forward to SpendGuard egress
proxy as upstream) **and** Shape B (CustomLogger registered). Belt and
braces.

**Pros**

- Two independent fail-closed paths.
- Audit-chain redundancy.

**Cons**

- **Double-counting risk:** Shape A reserves at HTTP time, Shape B also
  reserves at callback time → 2× reservation on the same call unless we
  add an idempotency dedupe (existing `idempotency_key` should handle it,
  but it is one more invariant to test).
- **Operator complexity:** two integration surfaces to debug when something
  goes wrong.

### 3.4 Recommendation for v1

**Ship Shape B (CustomLogger) as the primary surface.** Reasons:

1. It is the most LiteLLM-native shape — operators expect to add an entry
   to `litellm_settings.callbacks` to plug in governance.
2. It is **provider-agnostic** — the existing egress proxy only handles
   OpenAI today; Shape B works for all 100+ LiteLLM providers from day 1.
3. The typed `DecisionDenied` exception surface is materially better
   operator UX than "HTTP 402 from somewhere upstream".

**Shape A is documented as a fallback** for users who already run the
egress proxy and do not want a second integration. We do not ship
new code for it — we ship a recipe in `IMPLEMENTATION.md` showing
how to point `litellm.api_base` at the existing proxy.

**Shape C is explicitly deferred to v2.** The composite story needs the
v1 callback shipped and battle-tested first.

---

## 4. Message Flow

### 4.1 Allow path (Shape B, non-streaming)

```
┌──────────┐   ┌──────────────┐  ┌──────────────┐  ┌──────────┐  ┌────────────┐
│ App / LL │   │ LiteLLM core │  │ SpendGuard   │  │ Sidecar  │  │ Provider   │
│  proxy   │   │              │  │   Callback   │  │ (gRPC)   │  │ (OpenAI…)  │
└────┬─────┘   └──────┬───────┘  └──────┬───────┘  └────┬─────┘  └─────┬──────┘
     │ acompletion(.) │                 │               │              │
     ├───────────────►│                 │               │              │
     │                │ async_pre_call_hook(data)       │              │
     │                ├────────────────►│               │              │
     │                │                 │ request_decision(LLM_CALL_PRE)│
     │                │                 ├──────────────►│              │
     │                │                 │   DecisionOutcome(ALLOW,     │
     │                │                 │       decision_id=…)         │
     │                │                 │◄──────────────┤              │
     │                │   data (unchanged, decision_id stashed in      │
     │                │           kwargs["spendguard"])                │
     │                │◄────────────────┤               │              │
     │                │ HTTP POST /v1/chat/completions                 │
     │                ├───────────────────────────────────────────────►│
     │                │                                  response_obj │
     │                │◄───────────────────────────────────────────────┤
     │                │ async_log_success_event(kwargs, response_obj)  │
     │                ├────────────────►│               │              │
     │                │                 │ emit_llm_call_post(          │
     │                │                 │   outcome="SUCCESS",         │
     │                │                 │   decision_id, reservation_id│
     │                │                 │   provider_reported_amount)  │
     │                │                 ├──────────────►│              │
     │                │                 │     ACK + invoice_id         │
     │                │                 │◄──────────────┤              │
     │  response_obj  │◄────────────────┤               │              │
     │◄───────────────┤                 │               │              │
```

### 4.2 Deny path

```
async_pre_call_hook → request_decision → DENY (reason_codes=["BUDGET_EXCEEDED"])
                                                      │
                              raise DecisionDenied    │
                                                      ▼
LiteLLM short-circuits, never calls upstream provider, surfaces exception
to the caller. No invoice; no release needed (no reservation was created).
```

### 4.3 Failure path (provider 5xx)

```
async_pre_call_hook → ALLOW + decision_id
HTTP call → provider 500
async_log_failure_event → emit_llm_call_post(outcome="FAILURE", decision_id, reservation_id)
```

### 4.4 Streaming (v1 = end-of-stream commit)

```
async_pre_call_hook → ALLOW (reservation = estimated max tokens)
stream chunks flow to client
on stream complete → async_log_success_event(kwargs, full_response)
                  → emit_llm_call_post(outcome="SUCCESS") with real token totals
```

Streaming claim **estimator** projects worst-case tokens; commit reconciles
to actual. Mid-stream client disconnect → no `async_log_success_event` →
reservation expires via TTL sweep. This is acceptable for v1 (parity with
egress-proxy v0.2). Chunk-by-chunk reconciliation deferred to v2.

---

## 5. Failure Modes & Contracts

| Failure | Behaviour | Contract |
|---|---|---|
| Sidecar UDS unreachable in pre-call | Raise `SidecarUnavailable` → LiteLLM blocks | Default fail-closed; opt-in `SPENDGUARD_FAIL_OPEN=1` for dev only |
| Postgres ledger down | **Sidecar returns `DEGRADED`; LiteLLM callback FAIL-CLOSED — raises `SidecarUnavailable` and the LLM call is denied.** Unlike `agt.py` where DEGRADED → ALLOW (tool calls don't spend money the same way), the LiteLLM integration spends real provider $ on each call. Allowing under DEGRADED would break F2 fail-closed AND F4 audit-chain coverage (no canonical_events row because ledger is down). Operators may opt out via `SPENDGUARD_LITELLM_FAIL_OPEN=1` (dev only). This is a **deliberate divergence from `agt.py`** documented here (Round 2 Phase 0 review P0.1 fix). | Fail-closed on ledger outage; metric exposed; alert recommended |
| Reservation TTL expires before commit | Sidecar auto-releases; commit becomes no-op idempotent | Long streams must set TTL ≥ stream timeout; default 300s |
| Partial commit (commit RPC times out after success) | Idempotency key dedupes; retry returns same `invoice_id` | `derive_idempotency_key(...)` matches existing SDK |
| Hot-reload mid-call | Frozen `PricingFreeze` carries through commit; new pricing takes next call | Already solved by `issue-59-approval-resume-frozen-pricing.md` |
| Double-spend across LiteLLM workers | Single-writer-per-budget ledger serialises; no client-side coordination needed | Phase 1 constraint per `project_phase1_ledger.md` |
| LiteLLM retries (built-in `num_retries`) | LiteLLM mints a **fresh `litellm_call_id`** for each retry attempt; the callback derives a **distinct `decision_id` per attempt** (via `derive_uuid_from_signature(f"litellm:{litellm_call_id}", scope="decision_id")`). Each attempt reserves; failed attempts release on `async_log_failure_event`; successful attempt commits. The **anti-pattern in REVIEW_STANDARDS §9.6** ("reserving on every retry without idempotency") refers to reserving with the *same* `decision_id` across attempts — which our derivation explicitly avoids because `litellm_call_id` is distinct per attempt. | Verified in demo; consistent with ADR-002 |
| Client cancels mid-call (Ctrl-C) | `async_log_failure_event` fires with `CancelledError` → release | Verified in demo |

Three SDK exceptions used (subclass `SpendGuardError`):

- `SidecarUnavailable` — sidecar UDS not reachable (NEW in
  `errors.py`, added by Slice 1)
- `DecisionDenied` — already exists in `errors.py` (raised by
  `SpendGuardClient.request_decision` on DENY); reused unchanged.
- `SpendGuardConfigError` — missing `budget_id` / `window_instance_id`
  mapping or `budget_resolver` returned `None` (NEW in `errors.py`,
  added by Slice 1)

Naming convention: typed-deny exception is `DecisionDenied`
**everywhere** (SDK, tests, demo, docs). The earlier name
`SpendGuardDenied` is REJECTED — it would diverge from
`client.py:request_decision` and from `agt.py` precedent.

---

## 6. API Surface (Python)

Module: `spendguard.integrations.litellm`. Mirrors `agt.py` style: one file,
~250 lines target, dataclass + class + `__all__`.

```python
# spendguard/integrations/litellm.py

from spendguard import SpendGuardClient
from spendguard._proto.spendguard.common.v1 import common_pb2

# Optional import; raises clear ImportError if litellm is not installed.
from litellm.integrations.custom_logger import CustomLogger

@dataclass(frozen=True, slots=True)
class LiteLLMRunContext:
    """Per-call identifiers. Set via run_context() async CM, read by callback."""
    run_id: str
    step_id: str | None = None

@asynccontextmanager
async def run_context(ctx: LiteLLMRunContext) -> AsyncIterator[LiteLLMRunContext]:
    ...

def current_run_context() -> LiteLLMRunContext | None: ...

@dataclass(frozen=True, slots=True)
class ResolverContext:
    """Inputs the BudgetResolver sees on every call.

    - `data` — the LiteLLM `kwargs` dict (model, messages, metadata, ...)
    - `user_api_key_dict` — LiteLLM's `UserAPIKeyAuth` object in proxy
      mode; `None` in direct `acompletion()` mode.
    - `call_type` — LiteLLM `call_type` enum string (e.g. `acompletion`,
      `aembedding`).

    Note: the hook constructs this context explicitly from the
    `async_pre_call_hook` arguments — resolver MUST NOT scrape
    `data["user_api_key_dict"]` because that key is not guaranteed
    present in LiteLLM kwargs (P0 fix from Phase 0 review)."""
    data: Mapping[str, Any]
    user_api_key_dict: Any | None  # litellm.proxy.UserAPIKeyAuth | None
    call_type: str


BudgetResolver = Callable[[ResolverContext], "BudgetBinding | None"]
"""Map a ResolverContext → which SpendGuard budget to charge against.

Operator-supplied. Typical implementation pulls `team_id`/`key_alias`
from `ctx.user_api_key_dict` (proxy mode) or
`ctx.data["metadata"]["spendguard_budget_id"]` (direct mode), and
returns a `BudgetBinding`. Returning `None` raises
`SpendGuardConfigError` at the callback boundary (no fallback to a
global default — see ADR-001). The `| None` in the type is the
canonical 'no budget found' signal (Round 3 P2.2 fix)."""

@dataclass(frozen=True, slots=True)
class BudgetBinding:
    budget_id: str
    window_instance_id: str
    unit: common_pb2.UnitRef
    pricing: common_pb2.PricingFreeze

ClaimEstimator = Callable[[ResolverContext], list[common_pb2.BudgetClaim]]
"""Project BudgetClaims from ResolverContext (pre-call, no response yet).

For non-streaming: usually `ctx.data["messages"]` token count × input
price + estimated output. For streaming: worst-case output tokens
(reservation must cover the worst case; reconciler refunds the
difference at commit). v1 contract: returns **exactly one** BudgetClaim
(single-unit token-or-dollar claim per call). Multi-claim is v2."""

ClaimReconciler = Callable[
    [ResolverContext, Any], list[common_pb2.BudgetClaim]
]
"""Compute real BudgetClaims at commit time from ResolverContext +
response_obj.

Reads `response_obj.usage.prompt_tokens` + `completion_tokens` and
produces the canonical commit claims. Called from
`async_log_success_event`. v1 contract: returns **exactly one**
BudgetClaim, same unit as the estimator. The callback raises
`SpendGuardConfigError` if the reconciler returns 0 or ≥2 claims."""

class SpendGuardLiteLLMCallback(CustomLogger):
    """LiteLLM CustomLogger that reserves/commits via the SpendGuard sidecar.

    Registration::

        import litellm
        from spendguard import SpendGuardClient
        from spendguard.integrations.litellm import (
            SpendGuardLiteLLMCallback, BudgetBinding,
        )

        client = SpendGuardClient(socket_path=..., tenant_id=...)
        await client.connect(); await client.handshake()

        callback = SpendGuardLiteLLMCallback(
            client=client,
            budget_resolver=lambda ctx: BudgetBinding(
                budget_id=ctx.data.get("metadata", {}).get("spendguard_budget_id"),
                window_instance_id="...",
                unit=common_pb2.UnitRef(...),
                pricing=common_pb2.PricingFreeze(...),
            ),
            claim_estimator=lambda ctx: [common_pb2.BudgetClaim(...)],
            claim_reconciler=lambda ctx, resp: [common_pb2.BudgetClaim(...)],
            fail_closed=True,  # default; can be overridden by env
        )
        litellm.callbacks = [callback]

        # Now any litellm.acompletion(..., metadata={"spendguard_budget_id": "b1"})
        # is gated by SpendGuard."""

    def __init__(
        self,
        *,
        client: SpendGuardClient,
        budget_resolver: BudgetResolver,
        claim_estimator: ClaimEstimator,
        claim_reconciler: ClaimReconciler,
        fail_closed: bool = True,
    ) -> None: ...

    # CustomLogger overrides
    async def async_pre_call_hook(
        self,
        user_api_key_dict,  # UserAPIKeyAuth | None (None in non-proxy)
        cache,
        data: dict,
        call_type: str,
    ) -> dict | None: ...

    async def async_log_success_event(
        self, kwargs: dict, response_obj, start_time, end_time
    ) -> None: ...

    async def async_log_failure_event(
        self, kwargs: dict, response_obj, start_time, end_time
    ) -> None: ...

# Convenience for non-proxy users who do not want to subclass.
def install(
    *,
    client: SpendGuardClient,
    budget_resolver: BudgetResolver,
    claim_estimator: ClaimEstimator,
    claim_reconciler: ClaimReconciler,
    fail_closed: bool = True,
) -> SpendGuardLiteLLMCallback:
    """Construct the callback and append to litellm.callbacks. Returns it
    so the caller can detach later."""

__all__ = [
    "BudgetBinding",
    "BudgetResolver",
    "ClaimEstimator",
    "ClaimReconciler",
    "LiteLLMRunContext",
    "ResolverContext",
    "SpendGuardLiteLLMCallback",
    "_LoopBoundCallback",  # proxy-template helper (Round 2 P0.5 fix)
    "current_run_context",
    "install",
    "run_context",
]
```

Notable shape decisions:

- **No global state.** Callback holds the `SpendGuardClient`; user creates it.
- **Resolver pattern** (not a hard-coded dict) so multi-tenant proxy setups
  can derive `budget_id` from `team_id` / `key_alias` at runtime.
- **Reconciler separate from estimator.** Pre-call estimator may guess;
  reconciler uses actual `response_obj.usage`. Keeps the two concerns honest.

---

## 7. Configuration Surface

### 7.1 Environment variables

| Var | Default | Meaning |
|---|---|---|
| `SPENDGUARD_LITELLM_FAIL_OPEN` | `0` | If `1`, sidecar errors → allow. **Dev only.** Read **once** at callback construction; flipping mid-process has no effect. The callback **must log a `WARNING` at construction** when the env var is `1`, and again **on every fail-open path taken** at runtime (per ACCEPTANCE.md S6). |
| `SPENDGUARD_LITELLM_TTL_SECONDS` | `300` | Reservation TTL passed to sidecar. Stream slices (Slice 4 + Slice 9) require TTL ≥ longest stream wall-clock; default 300s is sufficient for most use cases but operators with multi-minute streams must tune. |
| `SPENDGUARD_SIDECAR_SOCKET` | (required) | UDS path the callback connects to. Canonical name across SDK + operator templates (P2 fix from Phase 0 review). |
| `SPENDGUARD_TENANT_ID` | (required) | Tenant scope for sidecar handshake. |

(All read **once** at callback construction; **never** re-read mid-process. The `SPENDGUARD_LITELLM_DEFAULT_BUDGET_ID` env var from the pre-Phase-0 draft is REMOVED — there is no global default budget; `budget_resolver` returning `None` raises `SpendGuardConfigError`. See ADR-001 + P0.10 fix from Phase 0 review.)

### 7.2 LiteLLM proxy `proxy_config.yaml` integration

Operator-facing recipe (shipped in `IMPLEMENTATION.md`; no schema change in
LiteLLM):

```yaml
# proxy_config.yaml
litellm_settings:
  callbacks: spendguard_litellm_proxy_callback.handler_instance
  # ^ a small Python module the operator drops next to proxy_config.yaml
  #   that constructs SpendGuardLiteLLMCallback and assigns it to
  #   `handler_instance`. Pattern is standard for litellm custom callbacks.

general_settings:
  master_key: sk-...
  database_url: postgresql://...    # LiteLLM's own DB
  # SpendGuard's DB is separate; configured via SPENDGUARD_DATABASE_URL.

model_list:
  - model_name: gpt-4o-mini
    litellm_params:
      model: openai/gpt-4o-mini
      api_key: os.environ/OPENAI_API_KEY
```

The companion module looks like:

```python
# spendguard_litellm_proxy_callback.py (operator-owned, ~70 lines)
"""SpendGuard LiteLLM proxy callback — operator example.

The SpendGuard `SpendGuardClient` uses async gRPC over a UDS channel
which is **event-loop affine**: a channel created on loop L1 cannot
be safely reused on loop L2. LiteLLM imports this module
synchronously during `litellm --config proxy_config.yaml` boot, but
the LiteLLM proxy then starts its own ASGI event loop to serve
requests. We must therefore bootstrap the SpendGuard client **on
that serving loop**, not on a temporary one created via
`asyncio.run()`. The SDK ships `_LoopBoundCallback` which handles
this — operator template just imports + instantiates it (Round 3
P0.3 fix: previously this class lived inline in this template, now
it lives in `spendguard.integrations.litellm` so it is versioned +
tested with the SDK).
"""
from __future__ import annotations
import logging, os
from spendguard.integrations.litellm import (
    BudgetBinding, ResolverContext,
)
from spendguard._proto.spendguard.common.v1 import common_pb2

log = logging.getLogger("spendguard.litellm.proxy")


#
# _LoopBoundCallback now lives in the SDK (Round 3 P0.3 fix). The
# operator template just imports + instantiates it.
from spendguard.integrations.litellm import _LoopBoundCallback


def _resolve(ctx: ResolverContext) -> BudgetBinding:
    """Map LiteLLM proxy identity → SpendGuard BudgetBinding.

    Proxy auth flow that yields `user_api_key_dict.team_id` (Round 2
    Phase 0 review P0.6 fix — was under-specified before):

    1. Operator runs `litellm --config proxy_config.yaml`.
    2. Master key is set via `general_settings.master_key`.
    3. Operator creates a team and a key via the proxy's
       `/team/new` and `/key/generate` endpoints (LiteLLM proxy admin
       API), assigning the key to the team. The key encodes
       `team_id` server-side.
    4. Caller HTTP request: `POST /v1/chat/completions` with
       `Authorization: Bearer sk-<that-key>` header.
    5. LiteLLM proxy validates the key, populates
       `kwargs["user_api_key_dict"]` with a `UserAPIKeyAuth` object
       whose `.team_id` matches the team assigned in step 3.
    6. This resolver reads `.team_id`, looks up the budget by env
       var. Header-only `team_id` (without the auth flow) is IGNORED
       because `user_api_key_dict` is `None` in that case.
    """
    uak = ctx.user_api_key_dict
    team_id = getattr(uak, "team_id", None) if uak else None
    if not team_id:
        raise RuntimeError(
            "SpendGuard proxy callback requires team_id via authenticated "
            "API key; see operator setup at "
            "docs/specs/litellm-integration/PROXY_RECIPE.md#team-seed."
        )
    budget_id = os.environ.get(f"SPENDGUARD_BUDGET_FOR_TEAM_{team_id}")
    if not budget_id:
        raise RuntimeError(
            f"No SPENDGUARD_BUDGET_FOR_TEAM_{team_id} env var set; "
            "operator must define one per team.")
    return BudgetBinding(
        budget_id=budget_id,
        window_instance_id=os.environ["SPENDGUARD_WINDOW_INSTANCE_ID"],
        unit=common_pb2.UnitRef(...),
        pricing=common_pb2.PricingFreeze(...),
    )


def _estimator(ctx: ResolverContext) -> list:
    """Worst-case estimate: budget-anchored 5000 atomic units.
    Operator override for real deploys: use a token counter."""
    binding = _resolve(ctx)
    return [common_pb2.BudgetClaim(
        budget_id=binding.budget_id,
        unit=binding.unit,
        amount_atomic="5000",
        direction=common_pb2.BudgetClaim.DEBIT,
        window_instance_id=binding.window_instance_id,
    )]


def _reconciler(ctx: ResolverContext, response_obj) -> list:
    """Real cost: completion_tokens × output_price (atomic).
    `response_obj.usage` shape is consistent across OpenAI-compatible
    providers including Anthropic and Bedrock via LiteLLM's
    normalization."""
    binding = _resolve(ctx)
    tokens = int(getattr(response_obj.usage, "completion_tokens", 0))
    # Pricing-frozen output-token price multiplied by token count.
    # `binding.pricing` carries unit conversion + price snapshot.
    return [common_pb2.BudgetClaim(
        budget_id=binding.budget_id,
        unit=binding.unit,
        amount_atomic=str(tokens),  # atomic units; sidecar applies pricing
        direction=common_pb2.BudgetClaim.DEBIT,
        window_instance_id=binding.window_instance_id,
    )]


handler_instance = _LoopBoundCallback(
    socket_path=os.environ["SPENDGUARD_SIDECAR_SOCKET"],
    tenant_id=os.environ["SPENDGUARD_TENANT_ID"],
    budget_resolver=_resolve,
    claim_estimator=_estimator,
    claim_reconciler=_reconciler,
)
```

---

## 8. Audit-Chain Story

### 8.1 Correlation IDs

LiteLLM stamps every call with a `litellm_call_id` (UUID, also available as
`kwargs["litellm_call_id"]` in callbacks). SpendGuard uses its own
`llm_call_id` (UUID7). We derive ours **from** theirs:

```
llm_call_id = derive_uuid_from_signature(
    f"litellm:{kwargs['litellm_call_id']}", scope="llm_call_id"
)
```

This gives a deterministic, joinable identifier in both `canonical_events`
and `LiteLLM_SpendLogs`. Operators can `JOIN ON` for reconciliation.

### 8.2 Event shape

Each LiteLLM call produces:

- One `DECISION_REQUESTED` event (pre-call)
- One `DECISION_ALLOWED` or `DECISION_DENIED`
- One `RESERVATION_CREATED` (on allow)
- One `INVOICE_COMMITTED` (on success) or `RESERVATION_RELEASED` (on
  failure / cancel / TTL)

All hash-chained per existing canonical_events schema. No new event types
needed — LiteLLM is just another caller of the existing decision/invoice
RPCs.

**Naming convention.** Always use the full event-type names in
SQL/JSON/log assertions: `DECISION_ALLOWED`, `DECISION_DENIED`,
`DECISION_REQUESTED`, `RESERVATION_CREATED`, `RESERVATION_RELEASED`,
`INVOICE_COMMITTED`. Abbreviated forms (`ALLOWED`/`COMMITTED`) MUST
NOT appear in spec docs, tests, or demo stdout — they create false
mismatches in grep-based audits (P2 fix from Phase 0 review).

### 8.2a `decision_context_json` fields (MANDATORY shape)

The callback MUST pass the following fields into `request_decision` so
they land in `canonical_events.decision_context_json` for every event
emitted by this integration. The fields are the source of truth for
ACCEPTANCE.md S1–S2 and join query Q2.

| Field | Value | Source |
|---|---|---|
| `integration` | literal `"litellm"` | constant in callback |
| `litellm_call_id` | LiteLLM-stamped UUID | `kwargs["litellm_call_id"]` |
| `model` | model alias | `kwargs["model"]` |
| `pricing_version` | frozen pricing version | `binding.pricing.pricing_version` |
| `price_snapshot_hash_hex` | frozen pricing hash | `binding.pricing.price_snapshot_hash_hex` |
| `fx_rate_version` | FX version | `binding.pricing.fx_rate_version` |
| `unit_conversion_version` | unit-conv version | `binding.pricing.unit_conversion_version` |
| `prompt_hash` | blake2b 16-byte hash of `messages` JSON | computed by SDK helper |
| `call_type` | LiteLLM `call_type` (`acompletion`/etc) | `call_type` arg of hook |
| `stream` | `bool` | `kwargs.get("stream", False)` |
| `mode` | literal `"direct"` if `user_api_key_dict is None` else `"proxy"` | derived in `_build_decision_context` (Round 2 P0.2 fix: gives ACCEPTANCE.md Q2 a real field to filter on for proxy step) |
| `team_id` | proxy mode only: `user_api_key_dict.team_id`; otherwise `null` | `user_api_key_dict` |

Forbidden: `messages` content verbatim, response text verbatim,
provider API keys. Per ACCEPTANCE.md S4 the row is hashed/redacted.

### 8.3 LiteLLM_SpendLogs interplay

We do not write to `LiteLLM_SpendLogs`. LiteLLM continues to write its
own row per call **when running as the LiteLLM proxy** — that is the
only mode where LiteLLM populates the SpendLogs table.

**Direct `litellm.acompletion()` mode does NOT create `LiteLLM_SpendLogs`
rows** (LiteLLM's SpendLogs writer is gated on the proxy DB connection;
direct in-process calls bypass it entirely). The `LiteLLM_SpendLogs ⨝
canonical_events` join (ACCEPTANCE.md §5.1 Q2) is therefore meaningful
**only for the proxy-mode demo step** (Slice 9 step 4 PROXY). The
direct-mode steps (1 ALLOW, 2 DENY, 3 STREAM) assert canonical_events
chain integrity but do NOT assert SpendLogs presence.

(P0.5 fix from Phase 0 review.)

---

## 9. Open Questions / ADRs

### ADR-001: Where does the budget identifier come from?

**Context.** LiteLLM has no native concept of "SpendGuard budget_id". The
callback needs a way to know which budget to charge.

**Options.**
1. Read from `kwargs["metadata"]["spendguard_budget_id"]` (caller-owned).
2. Derive from `team_id` / `key_alias` via operator-supplied resolver.
3. Hard-code a single budget per process.

**Decision.** Resolver callback (Option 2), with metadata override (Option
1) as the resolver's fallback. Option 3 is too inflexible for proxy use.

**Consequences.** Operator must write ~10 lines of resolver code. We
trade dead-simple onboarding for multi-tenant correctness; multi-tenant
is the LiteLLM proxy's primary use case.

### ADR-002: How do we handle LiteLLM's built-in retries?

**Context.** `litellm.acompletion(..., num_retries=3)` retries on 5xx /
rate-limit. Each retry fires `async_pre_call_hook` again. If we reserve
on every attempt, we triple-book the budget.

**Options.**
1. Reserve on every attempt; release the prior on failure event.
2. Reserve once, share decision_id across retries.
3. Detect retry via `kwargs["num_retries"]` and skip the hook.

**Decision.** Option 1 (reserve every attempt, release on failure). The
failure event reliably fires between attempts in LiteLLM's retry loop;
reservations are cheap; idempotency key prevents accidental dedupe across
attempts because each attempt has a distinct `litellm_call_id`.

**Consequences.** Brief over-reservation during retry windows. Acceptable;
worst case is `num_retries × per_call_estimate` held for ~1s. Documented
in `FAILURE_MODES.md`.

### ADR-003: Streaming commit granularity

**Context.** SSE streams emit tokens over seconds-to-minutes. When do we
commit?

**Options.**
1. Commit on stream completion (end-of-stream).
2. Commit incrementally per chunk.
3. Commit periodically (e.g. every 100 tokens or every 5s).

**Decision.** Option 1 for v1 (parity with egress-proxy v0.2 SSE). Option 3
for v2. Option 2 rejected: too many sidecar RPCs per call.

**Consequences.** Reservation must cover worst-case stream cost; mid-stream
client crash relies on TTL sweep. Operators with very long streams must
tune `SPENDGUARD_LITELLM_TTL_SECONDS`.

### ADR-004: Fail-closed default

**Context.** Sidecar unreachable → block call (deny) or allow?

**Options.** Deny by default; allow by default; configurable.

**Decision.** Deny by default (`fail_closed=True`); env override
`SPENDGUARD_LITELLM_FAIL_OPEN=1` for dev. Production users explicitly
opt-out, not opt-in. Aligns with SpendGuard's overall fail-closed
posture.

**Consequences.** A sidecar outage takes the LLM offline. Operators
must run sidecar with redundancy. Documented in `OPERATIONS.md`.

### ADR-005: Sync `litellm.completion()` support

**Context.** LiteLLM still has many users on synchronous
`litellm.completion()`. `CustomLogger` exposes sync hooks
(`log_pre_api_call`, `log_success_event`).

**Options.**
1. Implement sync hooks too; bridge via `asyncio.run_coroutine_threadsafe`.
2. Async-only; document that sync users must use Shape A.
3. Async-only; loudly raise from the sync hooks.

**Decision (Round 2 Phase 0 review P0.7 fix — revised).** **Option 3
for v1**: async-only at full enforcement, sync `log_pre_api_call` hook
overridden to **raise `SpendGuardConfigError` BEFORE the provider HTTP
request**. The earlier "Option 2 + doc" was insufficient: if a user
installs the callback and calls `litellm.completion()` (sync), the
async hooks never fire and the provider is billed without budget
enforcement — silent fail-open. By overriding the sync pre-wire hook
to raise loudly, the integration stays fail-closed even when used
incorrectly.

Slice 1 adds the `log_pre_api_call` override raising
`SpendGuardConfigError("Sync litellm.completion() is not supported;
use litellm.acompletion() or Shape A egress proxy. See ADR-005.")`.

The sync log_success / log_failure hooks remain unimplemented (they
fire post-wire; raising there would only mask the real error and
double-bill operationally).

**Consequences.** Sync users get a typed error at call time instead
of silent unbilled requests. Still smaller v1 surface area than full
sync support; one extra paragraph in README documenting the new
exception path.

---

## 10. Out of Scope for v1 (Roadmap for v2+)

- **Streaming chunk-level commit** (ADR-003 deferred path).
- **Sync `litellm.completion()` first-class support** (ADR-005 deferred).
- **Tool-call sub-budgets** — when LiteLLM routes a tool call through
  function-calling, gate each tool invocation separately. Today handled
  via `agt.py` integration; LiteLLM-side tool gating is v2.
- **LiteLLM SDK auto-instrumentation** (a one-liner `spendguard.instrument_litellm()`
  similar to `auto-instrument-egress-proxy`). Designable, but requires
  monkey-patching LiteLLM's callback list, which we want to think about.
- **Composite gateway (Shape C)** — defer until v1 callback shipped and
  proven.
- **Cost Advisor proposal write-path integration** — Cost Advisor can
  observe LiteLLM patterns but cannot patch LiteLLM config in v1. v2 may
  add a "swap `gpt-4o` → `gpt-4o-mini` based on observed prompt
  complexity" patch path.
- **Multi-region proxy correctness** — single-writer-per-budget is per
  ledger; cross-region requires Phase 2 ledger work, orthogonal to this
  integration.

---

## 11. Demo as Quality Gate

Per `feedback_demo_quality_gate.md`: Codex ✅ is not enough. The
integration ships **two** demo modes and ACCEPTANCE.md §5 is the
authoritative shape (REVIEW_STANDARDS.md §7.3 and TEST_PLAN.md §3 must
mirror it verbatim; any drift is a P0 finding).

**`DEMO_MODE=litellm_real`** — 4-step happy/edge path
([ACCEPTANCE.md §5.1](./ACCEPTANCE.md#51-demo_modelitell_real-allow-path--audit-chain)):

1. **ALLOW** — direct `litellm.acompletion()` → DECISION_ALLOWED →
   INVOICE_COMMITTED. (Slice 6.)
2. **DENY** — over-budget direct call → `DecisionDenied` raised; no
   reservation, no invoice. (Slice 6.)
3. **STREAM** — `litellm.acompletion(stream=True)` → reservation
   created with worst-case estimate → chunks delivered → end-of-stream
   `async_log_success_event` → INVOICE_COMMITTED with real usage.
   (Slice 9 + Slice 4 streaming reconciler.)
4. **PROXY** — LiteLLM proxy subprocess + `proxy_config.yaml` +
   operator-owned `spendguard_litellm_proxy_callback.py` → HTTP
   `POST /v1/chat/completions` with `team_id` → DECISION_ALLOWED →
   INVOICE_COMMITTED → `LiteLLM_SpendLogs ⨝ canonical_events` join
   produces ≥1 matched row. (Slice 9 + Slice 8 proxy template.)

**`DEMO_MODE=litellm_deny`** — 3-step fail-closed coverage
([ACCEPTANCE.md §5.2](./ACCEPTANCE.md#52-demo_modelitell_deny-fail-closed-verification)):

1. **Budget exhausted** — sidecar returns DENY; provider HTTP request
   counter stays at 0. (Slice 7.)
2. **Sidecar offline** — UDS path unreachable; callback raises
   `SidecarUnavailable`; provider counter still 0. (Slice 7.)
3. **Resolver returns None** — explicit `SpendGuardConfigError`;
   provider counter still 0. (Slice 7.)

**Counting HTTP endpoint requirement (P0.11 fix).** The deny demo MUST
use a counting HTTP endpoint (in-process `aiohttp` mock server with a
hit counter) — `litellm.acompletion(mock_response="...")` is BANNED
for the deny demo because no HTTP endpoint exists in mock_response
mode, making the "provider counter == 0" assertion vacuously true.
TEST_PLAN.md §4.3 enforces this.

These demos are the integration-time contract — they expose anything
the SDK-level tests miss.

---

## 12. Slicing Hint for IMPLEMENTATION.md

The doc that comes after this one slices the implementation into 10
slices (expanded from the initial 7-slice draft to cover the 4-step
`litellm_real` demo + 3-step `litellm_deny` demo + ACCEPTANCE.md D1–D3
docs site coverage that were missing from the pre-Phase-0 draft).
To keep slices ≤250 lines, the cuts are:

1. SDK skeleton (`integrations/litellm.py` shell, dataclasses, imports,
   `__all__`, `ResolverContext`, `errors.py` additions).
2. Pre-call hook + reservation path (Slice 6 demo step 1 ALLOW unlocks).
3. Success-event commit + reconciler (non-streaming) (step 1 ALLOW
   completes end-to-end).
4. Streaming reconciler (worst-case estimator at pre-call; end-of-stream
   reconciler; TTL ≥ stream wall-clock). Unblocks Slice 9 step 3 STREAM.
5. Failure-event release + retry handling (ADR-002).
6. Demo `litellm_real` ALLOW + DENY (steps 1–2 of the 4-step demo).
7. Demo `litellm_deny` (3 fail-closed sub-steps: budget / sidecar
   offline / resolver None).
8. Proxy callback template + recipe (`spendguard_litellm_proxy_callback.py`,
   `proxy_config.yaml`, `PROXY_RECIPE.md`).
9. Demo `litellm_real` STREAM + PROXY (steps 3–4 of the 4-step demo;
   depends on Slices 4 + 8).
10. Docs site (`docs/site/docs/integrations/litellm.md` 3-path page +
    sibling Related footer updates + README + run the final
    whole-integration adversarial Codex pass per ACCEPTANCE.md C2).

Each is independently testable and Codex-reviewable. Dependency graph
in IMPLEMENTATION.md §1; line-budget rollup in §5 (includes tests, SQL
verify files, Makefile, docs — corrects the P1.1 finding from Phase 0
review).

---

## 13. Summary

- **Recommended v1 shape:** SpendGuard as a LiteLLM `CustomLogger`
  callback (Shape B). Provider-agnostic, native to LiteLLM, typed deny.
- **Documented fallback:** LiteLLM → SpendGuard egress proxy chain
  (Shape A). No new SpendGuard code; recipe only.
- **Deferred to v2:** Composite gateway (Shape C), streaming chunk
  commits, sync support, tool-call sub-budgets, Cost Advisor write-path.

The top three named trade-offs:

1. **Provider coverage vs HTTP-layer visibility** — Shape B covers all
   LiteLLM providers but does not catch direct-to-OpenAI bypass; Shape A
   inverts. We pick B and document A.
2. **Pre-call latency vs hard-cap correctness** — adding a sidecar RPC
   to every call costs ~5–10 ms; the alternative is post-hoc decrement
   that races across workers. We pay the latency for hard-cap.
3. **Streaming commit timing vs RPC volume** — end-of-stream commit
   leaks budget to TTL on client crash; per-chunk commit is N× RPC
   overhead. v1 picks end-of-stream + tunable TTL.
