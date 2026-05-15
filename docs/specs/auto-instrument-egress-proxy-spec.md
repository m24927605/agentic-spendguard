# Auto-Instrument Egress Proxy — Spec v7

> **Status**: codex r6 RED → v7 fixes 3 procedural TL;DR-vs-body inconsistencies (recurring pattern flagged across r1/r3/r4/r6). Awaiting r7 → GREEN trip count: 1.
> **Capability level**: `L1.5 partial-L2 egress_proxy_opt_in` — full L2 (egress_proxy_hard_block) requires k8s NetworkPolicy (deferred §13.5).
> **Goal**: drop the onboarding bar from "wrap your model object + 7 SDK params" to "set `OPENAI_BASE_URL=http://localhost:9000/v1`" — match Helicone/Portkey's 1-env-var bar while keeping enforcement-grade semantics (STOP truly blocks the upstream call).
> **Audience**: launch-target devs running OpenAI agents locally; want hard-cap + audit chain; do NOT need REQUIRE_APPROVAL / DEGRADE / multi-step approval workflows (those remain the wrapper SKU).
> **MVP scope**: OpenAI `POST /v1/chat/completions` non-streaming. CONTINUE forwards + commits real token usage. STOP returns 429. Approval / DEGRADE / streaming / Anthropic / multi-provider defer.

---

## 0aaaaa. TL;DR for codex r7 — what changed from v6

v6 → v7 fixes (all 3 procedural; Staff decisions already applied in v6):

| Codex r6 finding | v7 resolution | Section |
|---|---|---|
| P1-r6.1 §13 missing item 11 (HMAC audit-oracle defense) — TL;DR promised, body didn't deliver | §13 item 11 appended | §13 |
| P1-r6.2 §11 missing FIPS subsection — TL;DR promised, body didn't deliver | §11.1 subsection appended | §11.1 |
| P2-r6.3 §14.1 missing slug convention (Step 4) + dependent-slice freeze exception (Step 5) | §14.1 Step 4 + Step 5 patched | §14.1 |

Recurring root cause across r1/r3/r4/r6: **header-vs-body inconsistency in spec edits**. Identified by codex r6. Process improvement: spec-edit workflow should grep TL;DR `§N.M` references against actual headings before sending to codex. Add `make spec-lint` if r7+ continues to catch this.

---

## 0aaaa. TL;DR for codex r6 — what changed from v5 (post Staff escalation)

Per §14.1 Staff escalation triggered by codex r5 RED. 4 Staff sub-agents (distributed-systems / security / infrastructure / ledger-audit) returned in ~5 min; consensus doc at `docs/specs/auto-instrument-egress-proxy-staff-escalation-r5.md`. 3-of-4 majority on each axis:

| Axis | Staff consensus → v6 |
|---|---|
| Hash function | **blake2b-128** (replaces sha256) — matches all 3 SDKs |
| UUID flavor | **v4-shape (blake2b-masked)**, NOT RFC 4122 v5 — helper renamed in §4.1 wording |
| Canonicalization | **`serde_json` deterministic sorted-keys (BTreeMap)** — JCS is v0.2 aspirational |
| Port location | **NEW shared crate `services/ids/`** following `signing`/`policy` precedent — lands in **Slice 2** (was 4b) |
| Step_id discriminator | **Unified `:call:`** (drops `:proxy-call:`) — transport visibility lives in CloudEvent `source` (`egress-proxy://...` vs `sidecar://...`), NOT step_id |
| Operability | Rust port returns typed `Err(IdsError::Unserializable)` (no silent `repr()` fallback); counter `egress_proxy_unserializable_total` |
| Audit-oracle defense | Deferred §13.11 (HMAC-salt v0.2) — v0.1 stance: same-machine trust per §8 |
| FIPS | blake2b not FIPS; documented in §11 + slice 8 README; future `--hash-algo=sha256` flag |

---

## 0aaa. TL;DR for codex r5 — what changed from v4

v4 → v5 fixes:

| Codex r4 finding | v5 resolution | Section |
|---|---|---|
| P1-r4.1 fresh-UUIDv7 step_id/llm_call_id breaks cost_advisor agent-scope (verified against all 3 SDKs at langchain.py:248-250, openai_agents.py:176-178, pydantic_ai.py:439-440) | §4.1 step 5 + §4.1.5 use deterministic derivation: `signature = sha256(body)[..16]`; `step_id = f"{run_id}:proxy-call:{signature}"`; `llm_call_id = derive_uuid_from_signature(signature, scope="llm_call_id")` — mirrors the 3 SDK patterns 1:1 | §4.1, §4.1.5 |
| P2-r4.A Staff escalation undefined (consensus doc location, decision-maker, freeze rule) | NEW §14.1 Staff escalation playbook | §14.1 |
| P2-r4.B `egress_proxy_opt_in` documentation-only is operationally null | §11 explicitly says "documentation only; no code validation; informational" | §11 |
| P2-r4.C OpenAI 401 TLS-vs-body distinction | §4.4 adds explicit TLS handshake / connection failure row | §4.4 |

---

## 0aa. TL;DR for codex r4 — what changed from v3

v3 → v4 fixes (all 2 P1 + 4 P2-critical verified against actual code):

| Codex r3 finding | v4 resolution | Section |
|---|---|---|
| P1-r3.1 v3 cited openai_agents.py:222-234 + langchain.py:287-299 as evidence for post→confirm order; in reality those files DON'T call confirm_publish_outcome at all (verified by grep) | §4.1 wire-fidelity note rewritten: pydantic_ai is the SOLE complete reference; openai_agents/langchain skip Stage 6 ACK (separate audit gap, file follow-up). Proxy follows pydantic_ai (both calls). | §4.1 |
| P1-r3.2 §4.4 audit table claimed CLIENT_TIMEOUT / PROVIDER_ERROR reasons but ledger collapses both to RUNTIME_ERROR (verified `adapter_uds.rs:467-472`, `transaction.rs:1055`) | §4.4 table updated to show actual recorded reason (`RUNTIME_ERROR` for ProviderError/ClientTimeout/SSE-unexpected). Note v0.2 follow-up to thread original Outcome through to audit. | §4.4 |
| P2-r3.A §15 slice 5 still mandated SIGTERM enumeration (contradicts §9 fix) | §15 row 5 rewritten: per-handler commit/release; graceful_shutdown drain test (no enumeration) | §15 |
| P2-r3.B `egress_proxy_opt_in` advertising mechanism doesn't exist (HandshakeRequest has no string metadata field) | §11 + §15 row 8 clarified: advertising lives on **sidecar's** `enforcement_strength` env var (operator-set), NOT proxy's own handshake (proxy is a UDS client of sidecar, doesn't advertise its own capability_level upstream). config.rs gets a doc-comment-only update listing accepted strings. | §11, §15 |
| P2-r3.C §4.1.5 pricing cache refresh policy undefined | §4.1.5 specifies: mtime-check on every PRE (cheap, deterministic); cache invalidates when mtime advances; lock-free swap via `Arc<arc_swap::ArcSwap<PricingSnapshot>>`. Acceptance test in slice 4b. | §4.1.5 |
| P2-r3.D §4.1 step 5 DecisionRequest missing `ids.step_id` + `ids.llm_call_id` (required for `route=llm.call`) | §4.1 step 5 updated: explicit `ids.step_id` (fresh UUIDv7 per HTTP request) + `ids.llm_call_id` (fresh UUIDv7) | §4.1 |
| P2-r3.E (Layer 4 strip vs §3.4 byte-identical contradiction) | §8 Layer 4 narrowed to "RedactedRequest<B> with redacting Debug" only; strip option removed | §8 |
| P2-r3.F (86400 dropped vs emitted ambiguity) | §0a TL;DR row + §3.3 clarified consistently: 86400 IS still emitted as operator hint; it is NOT a retry-storm prevention claim. Real retry control is client `max_retries=0`. | §0a, §3.3 |
| P2-r3.G slice 4c acceptance missing "no re-read PRE→POST" test | §15 row 4c adds: integration test that overwrites runtime.env mid-request; assert LLM_CALL_POST payload carries OLD pricing hash | §15 |
| P2-r3.H §11 line 769 "validates" should be "documents" | §11 reworded | §11 |

v4 is 11 slices (unchanged structure from v3). r4 should re-verify against actual code (no new SDK references introduced).

---

## 0a. TL;DR for codex r3 — what changed from v2

v2 → v3 fixes (all 5 P1 + 4 P2 verified against actual code at file:line):

| Codex r2 finding | v3 resolution | Section |
|---|---|---|
| P1-r2.A Step 12a/12b ordering BACKWARDS from SDK | §4.1 order corrected to: 12a `EmitTraceEvents/LLM_CALL_POST` FIRST, 12b `ConfirmPublishOutcome(APPLIED)` SECOND. Verified `pydantic_ai.py:615-634`, `openai_agents.py:222-234`, `langchain.py:287-299`. | §4.1 |
| P1-r2.B `route` vs `trigger` conflation | §4.1 step 5: `trigger=LLM_CALL_PRE` (enum) AND `route="llm.call"` (string). Separate fields per `adapter.proto:222-230`; `pydantic_ai.py:535-541` precedent. | §4.1 |
| P1-r2.C §4.4 double-release pseudocode | Collapsed to single path: APPLY_FAILED for proxy-internal faults, `LLM_CALL_POST(PROVIDER_ERROR)` for OpenAI/network faults. Never both. Mirrors `safe_confirm_apply_failed` (client.py:264-301). | §4.4 |
| P1-r2.D PricingFreeze mirroring wrong model | §4.1.5 changed to "FROZEN at PRE": proxy reads runtime.env ONCE per ReservationContext, stores PricingFreeze in struct, REUSES verbatim at POST. Verified `pydantic_ai.py:624` uses `self._pricing` pinned at ctor. NEVER re-read between PRE and POST. | §4.1.5 |
| P1-r2.E `enforcement_strength` validation surface misrepresented | §15 slice 8 chooses approach (b): document accepted strings in doc-comment, no enum conversion. Sidecar code path unchanged. | §15 |
| P2-r2.A `Retry-After: 86400` silently clamped by openai-python | §3.3 elevates `max_retries=0` from "belt-and-suspenders" to Tier-1 launch caveat. 86400 is dropped as a retry-storm prevention claim; doc clarifies it's a hint to operators reading logs, not a retry-control mechanism. | §3.3, §11 |
| P2-r2.B Streaming-detection misses tools/json_schema/SSE-upgrade | §5.3 adds Content-Type check on upstream response: `text/event-stream` → 502 + `code: spendguard_unexpected_streaming_response`. | §5.3 |
| P2-r2.C tower_http TraceLayer redaction is per-handler choice | §8 adds (a) tracing lint rule + grep test (slice 4c acceptance), (b) request body wrapped in `RedactedRequest<T>` newtype with redacting Debug | §8 |
| P2-r2.D SIGTERM enumeration contradicts no-global-cache | §9 picks (a): axum graceful_shutdown drains each handler naturally; each handler calls its own commit/release path. No global registry. | §9 |

v3 is 11 slices (unchanged from v2 §15 split). r3 verifies textual + structural alignment with actual production code.

---

## 0. TL;DR for codex r2 — what changed from v1

v1 → v2 fixes:

| Codex r1 finding | v2 resolution | Section |
|---|---|---|
| P1.1 Pricing/unit threading between PRE and POST missing | NEW §4.1.5 reservation context retained in per-request task-local | §4.1.5, §5 |
| P1.2 ConfirmPublishOutcome vs EmitTraceEvents conflated | §4.1 step 11 split into 11a (ConfirmPublishOutcome) + 11b (EmitTraceEvents/LLM_CALL_POST); error path → APPLY_FAILED | §4.1, §4.4, §5.4 |
| P1.3 Decision-absent / partial fail-closed claim weak | NEW §4.2 invariant + slice 4 test: any non-Decision response (Status::Cancelled, partial frame, deadline-exceeded) → 502, NEVER forward | §4.2 |
| P1.4 openai-python ignores body `code` on 429 | §3.3 fixed: `Retry-After: 86400` on hard-cap STOP; launch docs recommend `OpenAI(max_retries=0)` | §3.3, §11 |
| P1.5 Default `sha256(body)[..16]` idempotency double-bills retries | §3.2 fixed: default flipped to per-attempt `sha256(body || nanos)[..16]`; explicit header for retry-collapse | §3.2, §7 |
| P1.6 Auth-in-logs claim structurally weak | NEW §8 RedactedAuth newtype; §10 acceptance tests for span field redaction | §8, §10 |
| P2.1 L2 claim without NetworkPolicy | Capability renamed to `L1.5 partial-L2` everywhere | header, §11 |
| P2.3 Streaming-501 cripples Agents SDK | §1.3 + §5.3 honest: 501 with launch-doc recommendation to set `stream=False`; SSE pass-through is v0.2 priority-1 | §1.3 |
| P2.5 Slice 4 overpacked | Split into 4a/4b/4c | §15 |
| P2.6 Slice 7 demo wiring overpacked | Split into 7a/7b | §15 |
| P2.7 "1-env-var" claim breaks w/ required headers | NEW §6.1 server-side default IDs via env; headers override | §6.1, §10 |

Spec is now 11 slices (was 8). Codex r2 should re-verify these against actual code at the cited file:line refs from r1.

---

## 1. Context + scope

### 1.1 The friction this fixes

Pre-CA-P3.8, the only path from "user has an OpenAI agent" to "SpendGuard is gating it" is one of 4 framework integrations (`langchain` / `pydantic_ai` / `openai_agents` / `agt`) that requires:

1. `pip install spendguard-sdk[<framework>]`
2. `SpendGuardClient(socket_path=..., tenant_id=...)` + `await connect()` + `await handshake()`
3. Wrap user's model object in `SpendGuard*Model(inner=..., budget_id=..., window_instance_id=..., unit=common_pb2.UnitRef(...), pricing=common_pb2.PricingFreeze(price_snapshot_hash=bytes.fromhex(...)), claim_estimator=...)`
4. Find + copy 8+ UUIDs from `deploy/demo/init/migrations/30_seed_demo_state.sh`
5. Find + copy `SPENDGUARD_PRICE_SNAPSHOT_HASH_HEX` from runtime.env

This is a **30-line setup, not 1**. Helicone's competing path is `export OPENAI_API_BASE=https://oai.hconeai.com/v1`. For the 80% of users who only want hard-cap + audit chain (NOT approval workflows or DEGRADE), the wrapper-mode friction is overkill and loses launch funnel.

### 1.2 The wedge angle this protects

Helicone / Portkey / Langfuse are **observability** — they proxy to count tokens, build dashboards, debug. None of them ENFORCE — a STOP decision means "log it red", not "kill the upstream HTTP request". Our wedge stays "true enforcement before the call".

The egress proxy must:
- Block the upstream HTTP request before any bytes leave the proxy, when SpendGuard says STOP
- Return 429 with structured reason codes so user code can branch
- Continue to record full audit chain (reservation → commit_estimated) on CONTINUE

It must NOT:
- Silently log + forward (that would make us Helicone-with-extra-steps)
- Strip the user's API key (that's L3 provider_key_gateway, separately scoped, deferred)
- Mid-stream cancellation (deferred — streaming defers to v0.2)

### 1.3 MVP scope cut

**IN**:
- `POST /v1/chat/completions` (OpenAI shape, non-streaming)
- `stream: true` returns **501** with `code: spendguard_streaming_unsupported`, body recommends `stream=False` until v0.2. **Launch docs MUST list this as the #1 caveat** because openai-agents `Runner.run` uses streaming by default.
- CONTINUE → forward request to `https://api.openai.com/v1/chat/completions` → return response → commit_estimated with real `usage.total_tokens`
- STOP → 429 + JSON body + `Retry-After: 86400` (hard-cap), or `Retry-After: <seconds-to-window-rollover>` (rolling-window) + DO NOT forward; launch docs recommend `OpenAI(max_retries=0)`
- REQUIRE_APPROVAL, DEGRADE → 503 + structured error (proxy mode unsupported; honest disclosure — see §4.4)
- Multi-tenant via either (a) **proxy-startup env vars** for the 1-env-var launch claim (`SPENDGUARD_PROXY_DEFAULT_TENANT_ID` etc., §6.1) OR (b) per-request `X-SpendGuard-Tenant-Id` / `X-SpendGuard-Budget-Id` / `X-SpendGuard-Window-Instance-Id` headers (override env)
- Single proxy port (default `9000`, bound to `127.0.0.1` by default; §8)
- UDS gRPC client to existing `sidecar` service (matches Python SDK wrapper-mode wire)
- Token accounting per OpenAI `usage` block in response; reservation context (reservation_id, unit_ref, pricing_freeze) retained per-request in task-local for the PRE→POST transition (§4.1.5)
- Two-step commit lane matching wrapper: `ConfirmPublishOutcome(APPLIED)` for Stage 6 publish_effect, then `EmitTraceEvents → LLM_CALL_POST` for Stage 7 commit (see §4.1)
- Audit chain identical to wrapper-mode (reservation → commit_estimated → audit_decision + audit_outcome events)

**OUT (deferred, document as `Future`)**:
- Streaming (`stream: true`) — returns 501 with `code: spendguard_streaming_unsupported` (v0.2: SSE pass-through with mid-stream cap; design in §12)
- Anthropic / Gemini / Bedrock multi-provider routing (v0.2: path-based or auth-sniff routing)
- REQUIRE_APPROVAL pause+resume (architectural mismatch with HTTP; remains wrapper SKU)
- DEGRADE mutation patches (requires request-body mutation per provider; remains wrapper SKU)
- API key proxying / vaulting (L3, separately scoped)
- Embeddings, completions (legacy), images, audio endpoints (v0.2 catch-up)
- Tool-call mid-stream gating (deferred with streaming)

---

## 2. Architecture

```
┌─────────────────────────────────────────────────────────────────────┐
│  user's agent process (host or container, no SDK install needed)    │
│                                                                       │
│   import openai                                                       │
│   client = openai.OpenAI(                                             │
│     base_url="http://localhost:9000/v1",  # ← only change             │
│     api_key=os.environ["OPENAI_API_KEY"], # ← user's real key         │
│     default_headers={                                                 │
│       "X-SpendGuard-Tenant-Id":           "...",                      │
│       "X-SpendGuard-Budget-Id":           "...",                      │
│       "X-SpendGuard-Window-Instance-Id":  "...",                      │
│     },                                                                │
│   )                                                                   │
│   client.chat.completions.create(model="gpt-4o", messages=[...])      │
└────────────────────────────┬────────────────────────────────────────┘
                             │ HTTP POST :9000/v1/chat/completions
                             ▼
       ┌─────────────────────────────────────────────────────────┐
       │ services/egress_proxy/   (Rust, axum, MVP this slice)   │
       │                                                          │
       │  1. parse: model, messages, est. tokens                  │
       │  2. SidecarAdapterClient.request_decision()  ──┐         │
       │  3a. CONTINUE → forward to OpenAI ─────────────┼──→ OPENAI │
       │      ← response → parse usage                  │           │
       │      → commit_estimated(actual_tokens) ────────┤           │
       │      ← 200 + body                              │           │
       │  3b. STOP → 429 + reason_codes                 │           │
       │      → NO forward                              │           │
       │  3c. REQUIRE_APPROVAL/DEGRADE → 503            │           │
       └────────────────────────────┬────────────────────┘         │
                                    │ UDS gRPC                    │
                                    ▼                             │
       ┌─────────────────────────────────────────────┐             │
       │ services/sidecar/ (existing, unchanged)     │             │
       │  • RequestDecision RPC                      │             │
       │  • ConfirmPublishOutcome RPC                │             │
       │  • Contract DSL evaluator                   │             │
       │  • Ledger client                            │             │
       └─────────────────────────────────────────────┘             │
                                                                    │
       ┌────────────────────────────────────────────────────────────┘
       │ HTTPS POST api.openai.com/v1/chat/completions
       ▼
       OpenAI
```

Key architectural decisions:

**(D1) Proxy is a SEPARATE service, not folded into sidecar.** Sidecar is the authoritative decision-maker (already handles wrapper-mode UDS adapter clients); proxy is just another adapter that happens to speak HTTP. Same Decision RPC contract. Same audit invariants.

- Reason: keeps the sidecar's UDS-only security model intact, lets the proxy crash-restart independently, allows proxy to be deployed separately (e.g., per-namespace ingress) while sidecar stays per-pod.

**(D2) Proxy uses UDS to talk to sidecar (NOT mTLS gRPC to ledger directly).** Same as how Python SDK wrapper-mode talks to sidecar.

- Reason: don't duplicate decision logic, don't widen the ledger's trust boundary, reuse contract eval + signing path.

**(D3) Proxy is per-pod / per-node, NOT shared SaaS.** User runs proxy alongside sidecar on the same host or pod (host-network for localhost dev, sidecar pod for k8s).

- Reason: API keys flow through the proxy — single-tenant trust boundary preserves wrapper-mode's security posture.

**(D4) Proxy is stateless. All state in ledger.**

- Reason: crash-restart is safe (idempotency on ledger side); horizontal scale is trivial; matches Phase 1 ledger constraint (single_writer_per_budget — proxy is a thin shim, ledger is authority).

---

## 3. API surface

### 3.1 HTTP endpoints

| Method | Path | Status |
|---|---|---|
| `POST` | `/v1/chat/completions` | MVP |
| `GET`  | `/healthz` | MVP (ok / 503-on-sidecar-down) |
| `GET`  | `/readyz`  | MVP (ready when sidecar handshake succeeded since startup) |
| `GET`  | `/metrics` | MVP (Prometheus text; per-route counters, decision-outcome histogram, latency p50/p95/p99) |
| `POST` | `/v1/embeddings` | Deferred v0.2 |
| `POST` | `/v1/messages` (Anthropic) | Deferred v0.2 |

**Out of scope for MVP path matrix**: `/v1/audio/*`, `/v1/images/*`, legacy `/v1/completions`, `/v1/files`, `/v1/assistants`. All non-MVP paths return `404 {"error":{"code":"spendguard_unsupported_endpoint","message":"egress-proxy v0.1 only supports /v1/chat/completions"}}`.

### 3.2 Request shape

Identical to OpenAI's `POST /v1/chat/completions` API. Proxy is a transparent OpenAI-compatible endpoint.

**Tenant / budget / window identification** — two paths (§6.1):
- **Path A (launch claim — 1-env-var)**: operator sets proxy-startup env vars (`SPENDGUARD_PROXY_DEFAULT_TENANT_ID`, `_BUDGET_ID`, `_WINDOW_INSTANCE_ID`); per-request headers are optional. User code is unchanged.
- **Path B (multi-tenant on one proxy)**: per-request `X-SpendGuard-Tenant-Id` / `X-SpendGuard-Budget-Id` / `X-SpendGuard-Window-Instance-Id` headers. Headers override env defaults when both present.
- Missing identification (both env unset AND headers absent) → 400.

**Additional OPTIONAL headers**:

```
X-SpendGuard-Run-Id:              <opaque string; defaults to a fresh UUIDv7 per request>
X-SpendGuard-Idempotency-Key:     <opaque string; defaults to per-attempt sha256(body || nanos)[..16] —
                                   see §7 for retry-collapse opt-in>
X-SpendGuard-Estimated-Tokens:    <int, optional, default: heuristic estimate from prompt tokens>
```

**Idempotency default flipped from v1 (codex r1 P1.5)**: default key is now **per-attempt** to prevent openai-python auto-retries from causing double-bills on OpenAI. Users wanting cross-process retry-collapse MUST send an explicit `X-SpendGuard-Idempotency-Key`. The launch docs MUST call this out alongside the `max_retries=0` recommendation.

The body is forwarded **byte-identically** to OpenAI on CONTINUE — proxy MUST NOT modify model/messages/tools/etc. Anything the proxy parses for SpendGuard purposes (model, est tokens, stream flag) is read-only.

### 3.3 Response shape

| Decision | Status | Body / Headers |
|---|---|---|
| CONTINUE + upstream 200 | 200 (or upstream's status, may be 2xx/4xx if OpenAI itself rejected) | OpenAI's response body forwarded byte-identically |
| CONTINUE + upstream 5xx / network | 502 (mapped from network errors), or forwarded upstream status | Includes structured `error.spendguard_upstream_failure` block. Reservation `Release(reason=PROVIDER_ERROR)` AND `ConfirmPublishOutcome(APPLY_FAILED)` fired before returning. |
| STOP (hard-cap, no retry helpful) | 429 + `Retry-After: 86400` | `{"error":{"message":"...","code":"spendguard_blocked","type":"insufficient_quota","details":{"decision_id":"...","reason_codes":["BUDGET_EXHAUSTED"],"matched_rule_ids":["..."]}}}` |
| STOP (window-rolling, retry-after-window) | 429 + `Retry-After: <seconds-to-window-rollover>` | Same body shape as above |
| REQUIRE_APPROVAL | 503 | `{"error":{"message":"...","code":"spendguard_unsupported_decision","details":{"decision_id":"...","reason_codes":[...],"hint":"use SDK wrapper for approval flows"}}}` |
| DEGRADE | 503 | Same shape, hint mentions DEGRADE not supported on proxy mode |
| Missing tenant/budget/window (env unset AND header absent) | 400 | `{"error":{"message":"...","code":"spendguard_missing_identification","details":{"missing":["X-SpendGuard-Tenant-Id (or set SPENDGUARD_PROXY_DEFAULT_TENANT_ID)"]}}}` |
| `stream: true` (v0.1 unsupported) | 501 | `{"error":{"code":"spendguard_streaming_unsupported","message":"set stream=False until v0.2"}}` |
| Sidecar unreachable / decision-absent / partial frame / Status::Cancelled | 502 | `{"error":{"code":"spendguard_sidecar_unavailable","fail_closed":true,"message":"..."}}` (NEVER fall through to forward — §4.2 invariant) |
| Idempotency conflict from ledger | 409 | Forwarded reason from ledger |

**Why 429 for STOP, with long `Retry-After`** (codex r2 P2-r2.A revised): OpenAI-python 1.x's retry logic checks `status == 429` and parses `Retry-After`, but the SDK internally clamps the wait via `min(retry_after, MAX_BACKOFF_SECS)` where `MAX_BACKOFF_SECS` is empirically ~60s. So `Retry-After: 86400` does NOT actually neuter retries — the SDK clamps to 60s and retries `max_retries` times (default 2 → 3 total attempts). Each retry burns a producer_sequence + audit_outbox row.

**Tier-1 launch caveat**: launch docs MUST list `OpenAI(max_retries=0)` as a required setting alongside `stream=False`. Without it, every STOP causes 3× the audit volume.

The `Retry-After: 86400` is still emitted for observability (shows up in operator dashboards as "hard-cap pause"), but it's NOT the retry-control mechanism. Real retry control is the client's `max_retries=0`.

| Variant | `Retry-After` | Semantics |
|---|---|---|
| Hard-cap STOP | `86400` (24h) | Dashboard hint; client must set `max_retries=0` |
| Window-rolling STOP | seconds-to-window-rollover | Same; advise client to set `max_retries=0` for clean UX |
| Sidecar unreachable | omit | Client SDK uses its own default backoff |
| Streaming-501 / missing-headers-400 | omit | Non-retryable error class |

**Why not 402**: considered (codex r1 P1.4 option (a)). Rejected because openai-python 1.x doesn't treat 402 specially either, and some user code patterns assert on 429 for "quota-exhausted-ish". 429 + `max_retries=0` is the least surprising shape AND properly bounded.

### 3.4 Authorization handling

The user's `Authorization: Bearer sk-...` header is **forwarded byte-identically** to OpenAI on CONTINUE. Proxy does NOT log it, persist it, or use it for anything other than verbatim forward. See §8 security model.

---

## 4. Decision flow

### 4.1 CONTINUE happy path (sequence)

```
 1. client → proxy: POST /v1/chat/completions {model, messages, ...}
                    + (Auth Path A) env defaults OR (Path B) X-SpendGuard-* headers
                    + Authorization: Bearer sk-... (user's OpenAI key)

 2. proxy: validate body parses + stream=false (501 if true)
 3. proxy: resolve (tenant_id, budget_id, window_instance_id) per §6.1:
           header overrides env default; 400 if neither.
 4. proxy: parse model + est. tokens (heuristic; X-SpendGuard-Estimated-Tokens overrides)
 5. proxy: build DecisionRequest (codex r2 P1-r2.B + r3 P2-r3.D fix: `route`/`trigger` separate fields; all 3 ids required for `route=llm.call`):
    - tenant_id (resolved per §6.1)
    - route="llm.call" (Contract scope; mirrors `sdk/python/src/spendguard/integrations/pydantic_ai.py:114`)
    - trigger=LLM_CALL_PRE (Trigger enum; mirrors pydantic_ai.py:541)
    - ids.run_id     (from `X-SpendGuard-Run-Id` header or fresh UUIDv7 per HTTP request)
    - signature = blake2b_128(canonicalized_body).hex()[..16]
                       // codex r5 Staff fix (3-of-4): blake2b-128, NOT sha256.
                       // Matches all 3 SDKs (langchain.py:149, openai_agents.py:125,
                       // pydantic_ai/ids.py:131,167). Canonicalization = serde_json::to_vec
                       // with BTreeMap (sorted keys; deterministic). JCS RFC 8785 is v0.2
                       // aspirational; v0.1 floor is sorted-keys.
    - ids.step_id    = format!("{}:call:{}", run_id, signature)
                       // codex r5 Staff fix (Staff #4 ledger-audit verdict): UNIFIED `:call:`
                       // discriminator, NOT `:proxy-call:`. Transport visibility lives in
                       // CloudEvent.source (`egress-proxy://...` vs `sidecar://...`), NOT
                       // step_id. cost_advisor groups by agent_id=step_id; transport-
                       // distinguished step_ids would split a single logical agent into
                       // two half-populated buckets across mixed/migrating deployments,
                       // silently breaking rules like failed_retry_burn_v1.
                       // Matches pydantic_ai.py:439 verbatim.
    - ids.llm_call_id = derive_uuid_from_signature(signature, scope="llm_call_id")
                       // codex r5 Staff fix (#2 security): renamed in commentary —
                       // helper is deterministic UUIDv4-shape (blake2b-masked), NOT
                       // RFC 4122 v5. Ported from `sdk/python/src/spendguard/ids.py:161-173`
                       // to new shared crate `spendguard-ids` (services/ids/) per Staff #3.
                       // Cross-language byte-equivalence locked via committed fixture at
                       // `services/ids/tests/fixtures/python_v1.json`.
    - ids.decision_id = fresh UUIDv7  // unique per attempt (no collapse semantics needed; ledger
                                       // idempotency_key handles cross-attempt collapse)
    - idempotency.key = X-SpendGuard-Idempotency-Key or sha256(body || nanos)[..16] (per-attempt default; see §7)
    - inputs.projected_claims = [BudgetClaim{budget_id, amount_atomic=est_tokens, unit_kind=token,
                                  window_instance_id, ...}]
    - inputs.projected_unit = UnitRef{token_kind=output_token, model_family=parsed_from_request_model}
 6. proxy → sidecar UDS: RequestDecision
 7. sidecar: contract eval → CONTINUE → ReserveSet (atomic with audit_decision)
 8. sidecar → proxy: DecisionResponse{decision=CONTINUE, decision_id, reservation_ids,
                                       ttl_expires_at, effect_hash, mutation_patch_json}
 9. proxy: store ReservationContext (see §4.1.5) in per-request task-local; this carries the
           reservation_id + unit + pricing-freeze tuple back to the POST step.
10. proxy → OpenAI: POST https://api.openai.com/v1/chat/completions (verbatim body, verbatim Authorization
                     unwrapped at the `RedactedAuth::expose_secret()` boundary; §8)
11. OpenAI → proxy: 200 + body with usage.total_tokens=actual
                     Proxy verifies Content-Type=application/json (text/event-stream → 502 + release;
                     codex r2 P2-r2.B fix); parses usage.completion_tokens + usage.prompt_tokens.
12a. proxy → sidecar UDS: EmitTraceEvents (single-element stream) {
              kind: LLM_CALL_POST,
              payload: LlmCallPostPayload {
                  reservation_id: <from §4.1.5 ReservationContext>,
                  estimated_amount_atomic: usage.total_tokens.to_string(),
                  unit: <from §4.1.5 — same UnitRef the reservation carried>,
                  pricing: <from §4.1.5 — same PricingFreeze, FROZEN at PRE time>,
                  outcome: SUCCESS,
                  decision_id, run_id, step_id, llm_call_id (from ReservationContext),
              }
       }  // Stage 7 commit_estimated; sidecar handler in transaction.rs::run_commit_estimated
12b. proxy → sidecar UDS: ConfirmPublishOutcome{
              decision_id, effect_hash, outcome: APPLIED  // or APPLIED_NOOP if no mutation
       }  // Stage 6 publish_effect ack — closes Contract §6 audit invariant
13. sidecar → ledger: CommitEstimated (atomic with audit_outcome; refunds reserved-committed delta)
14. proxy → client: 200 + OpenAI body (byte-identical)
```

**Wire fidelity note (codex r3 P1-r3.1 fix — only pydantic_ai is the complete reference)**: steps 12a and 12b are TWO separate sidecar RPCs and they MUST happen in this order:
1. **12a first**: `EmitTraceEvents/LLM_CALL_POST` — drives Stage 7 commit (audit_outcome row, ledger refund).
2. **12b second**: `ConfirmPublishOutcome(APPLIED)` — closes Stage 6 publish_effect ack.

Verified against production SDKs:
- `sdk/python/src/spendguard/integrations/pydantic_ai.py:615` (`emit_llm_call_post`) then `:630` (`confirm_publish_outcome`) — **complete reference**.
- `sdk/python/src/spendguard/integrations/openai_agents.py:222` — only calls `emit_llm_call_post`; **does NOT call confirm_publish_outcome**.
- `sdk/python/src/spendguard/integrations/langchain.py:287` — only calls `emit_llm_call_post`; **does NOT call confirm_publish_outcome**.

The openai_agents + langchain SDKs are INCOMPLETE per Contract §6.1 "no effect without audit" — they skip the Stage 6 publish_effect ack. This is a known gap in those SDKs (filed for follow-up; out of this spec's scope). **For this proxy spec, pydantic_ai is the SOLE complete reference and the proxy MUST do BOTH calls in order: LLM_CALL_POST first, then ConfirmPublishOutcome.** Skipping confirm-step would replicate the openai_agents/langchain gap, not close it. Slice 5 acceptance MUST verify both audit rows land in `audit_outbox`.

### 4.1.5 Reservation context retention — FROZEN-at-PRE (codex r2 P1-r2.D fix)

Between step 8 (DecisionResponse) and step 12a (LLM_CALL_POST), the proxy MUST retain a per-request struct holding:

```rust
struct ReservationContext {
    reservation_id: Uuid,              // from DecisionResponse.reservation_ids[0]
    decision_id: Uuid,                 // from DecisionResponse.decision_id
    effect_hash: [u8; 32],             // from DecisionResponse.effect_hash
    unit: UnitRef,                     // same UnitRef threaded into the original BudgetClaim
    pricing: PricingFreeze,            // FROZEN at PRE; never re-read after this point
    run_id: String,                    // from X-SpendGuard-Run-Id header OR fresh UUIDv7
    step_id: String,                   // DETERMINISTIC: format!("{run_id}:call:{}", blake2b_16(body))
                                       // codex r5 Staff fix — unified `:call:` discriminator (NOT
                                       // `:proxy-call:`); cross-mode convergence with wrapper SDKs.
    llm_call_id: String,               // DETERMINISTIC: derive_uuid_from_signature(sig, "llm_call_id")
                                       // codex r5 Staff fix — UUIDv4-shape (blake2b-masked),
                                       // NOT RFC 4122 v5. Ported from sdk/python/src/spendguard/ids.py:161.
    audit_decision_event_id: Uuid,     // from DecisionResponse (for STOP/error paths)
}
```

**Where does PricingFreeze come from?** — **Frozen at PRE time, not mirrored**.

Verified against production SDK at `sdk/python/src/spendguard/integrations/pydantic_ai.py:244,259,624`: the wrapper takes `PricingFreeze` as a **constructor argument** (pinned at process startup), stores it on `self._pricing`, and reuses it verbatim in the POST step. It does NOT re-read sidecar's `runtime.env` mid-flight.

The proxy mirrors this pattern:

1. At PRE (step 4-5), the proxy reads `/var/lib/spendguard/bundles/runtime.env` ONCE per request: parses `SPENDGUARD_SIDECAR_CONTRACT_BUNDLE_HASH_HEX` (NOT used for pricing — sidecar owns pricing validation), parses the sibling `bundles/contract_bundle/<id>.metadata.json` for the pricing 4-tuple (`pricing_version`, `price_snapshot_hash_hex`, `fx_rate_version`, `unit_conversion_version`). Stores the resulting `PricingFreeze` in the `ReservationContext.pricing` field.
2. At POST (step 12a), the proxy uses `ctx.pricing` UNCHANGED. **Never re-reads runtime.env between PRE and POST for the same ReservationContext.**

**Why frozen-at-PRE matters**: `services/sidecar/src/decision/transaction.rs:881-891` validates the LlmCallPostPayload pricing against the reservation's stored pricing (which was frozen at PRE time). If the proxy re-reads runtime.env at POST and the sidecar has hot-reloaded a new bundle in between, the proxy's POST-time pricing tuple no longer matches the reservation's stored tuple → `PricingFreezeMismatch` error → reservation rejected. The frozen-at-PRE design protects against the CA-P3.7 hot-reload race entirely.

**Concurrency / caching detail** (codex r3 P2-r3.C fix — refresh policy specified):

The proxy caches the parsed pricing tuple via `Arc<arc_swap::ArcSwap<PricingSnapshot>>` at process scope. The `arc_swap` crate provides lock-free atomic pointer swap (preferred over `RwLock` because PRE-time reads are the hot path).

Refresh trigger: **mtime check on every PRE request**. Each request at step 4 calls `std::fs::metadata(runtime_env_path).modified()`; if the mtime differs from the cached snapshot's stored mtime, the proxy re-parses the metadata.json + runtime.env and atomically swaps the cached `Arc<PricingSnapshot>`. Mtime read is ~1µs on Linux (single inode lookup); swap is a single atomic store. No measurable hot-path cost.

Invariant: the **value stored in a given ReservationContext MUST not change once the struct is constructed at step 4-5**. The handler:
1. At step 4: `let snapshot = cache.load_full();  // Arc<PricingSnapshot> at PRE time`
2. Stores `snapshot.pricing.clone()` into ReservationContext.pricing.
3. At step 12a: uses `ctx.pricing` directly (no re-read of cache).

If cache refreshes between step 4 and step 12a (because another concurrent request triggered mtime refresh), this request's `ctx.pricing` is already the stored snapshot at step 4 — unaffected.

Acceptance test (slice 4b): integration test (a) sidecar mock that does NOT hot-reload; (b) proxy receives 100 concurrent requests; (c) midway through the burst, test process `touch runtime.env` with new content; (d) assert each ReservationContext's pricing was internally consistent (PRE and POST byte-equal) AND that subsequent requests after the mtime change picked up new pricing.

Per-request retention mechanism: a struct passed by value (or Arc) through the handler's call chain. NOT `tokio::task_local!` (that's a different primitive — for per-task storage; axum handlers are tokio tasks but the pattern adds overhead with no benefit). NOT a global registry (memory leak + cross-request mixup risk).

**Failure handling**: if the proxy crashes between step 9 and step 12a, the reservation TTL-releases (60s default; see §9). The graceful SIGTERM handler (§9) does NOT enumerate in-flight contexts — instead axum's `graceful_shutdown()` drains each handler future naturally; each handler's own error-path code (§4.4) calls the appropriate release RPC.

### 4.2 STOP path + fail-closed invariant

```
1-7. same as CONTINUE up to DecisionResponse
8. sidecar → proxy: decision=STOP, decision_id, reason_codes=["BUDGET_EXHAUSTED"], matched_rule_ids=[...]
9. proxy → client: 429 + structured error body + Retry-After: 86400 (hard-cap) or rollover seconds
   NO call to OpenAI. The user's API key never leaves the proxy.
   The audit_decision is written by the sidecar (via the RecordDeniedDecision RPC + SP) before
   step 8 returns. Proxy does not call any additional sidecar RPC on STOP path.
```

**Hard fail-closed invariant** (codex r1 P1.3 fix): proxy MUST NEVER call OpenAI when the sidecar RPC did not return an explicit `decision = CONTINUE`. Specifically:

| Sidecar response | Proxy action |
|---|---|
| `Ok(DecisionResponse{decision: Continue, ...})` | Forward to OpenAI (only this branch) |
| `Ok(DecisionResponse{decision: Stop, ...})` | 429 + body (no forward) |
| `Ok(DecisionResponse{decision: RequireApproval | Degrade, ...})` | 503 + body (no forward) |
| `Ok(DecisionResponse{decision: Skip, ...})` | 429 + skip-specific body (proxy MVP treats Skip as STOP-like — non-actionable) |
| `Ok(DecisionResponse{decision: <unknown variant>, ...})` | 502 + body (proxy refuses to forward on unknown decision; logs warning) |
| `Err(tonic::Status::Cancelled)` | 502 + body (decision frame absent / partial) |
| `Err(tonic::Status::DeadlineExceeded)` | 502 + body (sidecar slow or unreachable) |
| `Err(other Status)` | 502 + body (log full status) |
| tokio task panic / cancellation in proxy handler | drop request → connection closes; reservation (if any) TTL-releases |

The Rust code layout MUST place the OpenAI call inside the `Decision::Continue` arm of a match expression, never as a fallthrough. Code review (slice 4c) verifies via grep + visual inspection.

**Test plan (slice 4 acceptance)**: integration test spawns a sidecar mock that returns each non-Continue variant; assert proxy returns the documented status code AND that an in-test counter of "OpenAI mock calls" is exactly zero in every non-Continue case.

### 4.3 Idempotent retry

**Default behavior in v0.1** (codex r1 P1.5 fix): the default idempotency key includes nanos so retries get fresh keys → each attempt is a fresh decision → no collapse → no double-bill risk (each attempt is independently gated).

**Explicit retry-collapse** (opt-in via `X-SpendGuard-Idempotency-Key`):

Client retries with same explicit `X-SpendGuard-Idempotency-Key`:

```
Retry → step 5-7 → sidecar's ledger sees idempotency_key collision → returns same DecisionResponse as first call.
- If first call was CONTINUE: proxy gets same decision_id back. Step 10 still forwards (idempotent at
  OpenAI level via its own request_id is NOT guaranteed). User accepts double-bill risk by using
  explicit collapse — typical use case: distributed-system "exactly-once" retry where the user has
  separately ensured at-most-one network attempt via their own request ID.
- If first call was STOP: proxy gets same DENIED response. Returns 429 again with NO upstream call.
```

**Body-cache idempotency** (deferred to v0.2; tracked §13.4): on explicit X-SpendGuard-Idempotency-Key replay, proxy could cache (idempotency_key → OpenAI response) for `reservation_ttl_seconds` to truly collapse on the OpenAI side too. Not in v0.1 because cache state contradicts proxy's stateless design (D4); needs Redis or similar. For v0.1, explicit collapse is "consistent decision, not necessarily consistent upstream call."

### 4.4 REQUIRE_APPROVAL / DEGRADE / proxy-side error during commit

```
1-7. same as CONTINUE
8. sidecar → proxy: decision=REQUIRE_APPROVAL (or DEGRADE)
9. proxy → client: 503 + structured error
   Hint body says "use SDK wrapper for approval workflows" + decision_id (the approval row is real,
   just inaccessible from proxy).
   NOTE: in REQUIRE_APPROVAL the sidecar's RecordDeniedDecision SP DOES write the approval row +
   audit_decision, so the audit chain is intact even though the proxy can't drive the resume.
```

**Honest disclosure**: proxy-mode is not enforceable for these decision kinds. Users with contracts that fire approval rules will see 503s on those requests. Document loud in proxy README; auto-add a contract-side rule check in the CLI (slice 6+) that warns if user's contract has any approval/degrade rule.

**Proxy-side error AFTER reservation but before commit** (codex r2 P1-r2.C fix — single path, never double-release): if a fault occurs in steps 9-11 (after CONTINUE reservation, before successful LLM_CALL_POST), the proxy emits **exactly one** release-class RPC. The choice depends on fault category:

```rust
async fn handle_error_after_reserve(
    ctx: &ReservationContext,
    err: ErrorClass,
) -> Result<()> {
    match err {
        // OpenAI returned a structured failure (4xx, 5xx, network timeout):
        // route through LLM_CALL_POST with the appropriate outcome enum.
        // The sidecar's adapter_uds.rs LLM_CALL_POST handler routes
        // PROVIDER_ERROR / CLIENT_TIMEOUT / RUN_ABORTED through run_release().
        ErrorClass::UpstreamHttpError | ErrorClass::UpstreamTimeout => {
            ctx.client.emit_llm_call_post(LlmCallPostPayload {
                reservation_id: ctx.reservation_id,
                decision_id: ctx.decision_id,
                outcome: match err {
                    ErrorClass::UpstreamTimeout => Outcome::ClientTimeout,
                    _ => Outcome::ProviderError,
                },
                unit: ctx.unit.clone(),
                pricing: ctx.pricing.clone(),
                // No estimated_amount_atomic — outcome != SUCCESS
                ..Default::default()
            }).await?;
        }

        // Proxy-internal fault (failed to parse upstream response, JSON
        // missing usage block, proxy logic bug, etc.): use
        // ConfirmPublishOutcome(APPLY_FAILED). This is the "we tried to
        // apply but couldn't" lane; sidecar's ConfirmPublishOutcome
        // handler routes APPLY_FAILED through run_release() with
        // reason=RUNTIME_ERROR. Mirrors `safe_confirm_apply_failed`
        // at sdk/python/src/spendguard/client.py:264-301.
        ErrorClass::ProxyInternal => {
            ctx.client.confirm_publish_outcome(ConfirmPublishOutcomeRequest {
                decision_id: ctx.decision_id.to_string(),
                effect_hash: ctx.effect_hash.to_vec(),
                outcome: Outcome::ApplyFailed as i32,
            }).await?;
        }
    }
    Ok(())
}
```

**Hard rule (verified against `transaction.rs:1078-1083` double-release guard)**: the proxy MUST call **exactly one** of the two RPCs per ReservationContext. Calling both triggers `ReservationStateConflict` on the second call (sidecar's `run_release` checks `current_state == "reserved"` and rejects when already released by the first call). The SDK's `safe_confirm_apply_failed` only calls one RPC for this exact reason.

**Audit-reason mapping** (codex r3 P1-r3.2 fix — ledger collapses both ProviderError + ClientTimeout to `RUNTIME_ERROR`):

The wire-level Outcome enum the proxy sends to sidecar's `LLM_CALL_POST` handler is one of {`PROVIDER_ERROR`, `CLIENT_TIMEOUT`, `RUN_ABORTED`, `SUCCESS`}, but the sidecar's `adapter_uds.rs:467-472` + `transaction.rs:1055` collapses the first two into a single `ReleaseReason::RuntimeError` enum → audit_outbox `audit_outcome.reason = "RUNTIME_ERROR"`. So audit consumers cannot distinguish provider 5xx vs client timeout at the audit row. **This is a v0.1 limitation, not a bug**; v0.2 follow-up tracked §13.10 to thread original Outcome through to a richer audit reason.

| Failure timing | Wire-level Outcome to sidecar | Audit row's `reason` (post-sidecar-collapse) |
|---|---|---|
| OpenAI 4xx (invalid model, auth fail, 401 on user's API key) | `LLM_CALL_POST { outcome: PROVIDER_ERROR }` | `RUNTIME_ERROR` |
| OpenAI 5xx / network | `LLM_CALL_POST { outcome: PROVIDER_ERROR }` | `RUNTIME_ERROR` |
| OpenAI request timed out | `LLM_CALL_POST { outcome: CLIENT_TIMEOUT }` | `RUNTIME_ERROR` (collapsed) |
| Upstream returned text/event-stream unexpectedly | `LLM_CALL_POST { outcome: PROVIDER_ERROR }` | `RUNTIME_ERROR` |
| TLS handshake / connection failure to OpenAI (cert pinning fails, root CA mismatch, DNS failure) | `LLM_CALL_POST { outcome: PROVIDER_ERROR }` (reqwest connection error maps here) | `RUNTIME_ERROR`; payload_json includes `upstream_error_class: "TRANSPORT_ERROR"` distinguishing from HTTP 5xx (codex r4 P2-r4.C fix) |
| Proxy-internal (JSON parse fail / no usage block / etc.) | `ConfirmPublishOutcome { APPLY_FAILED }` | `RUNTIME_ERROR` |
| Proxy panic / SIGKILL | (nothing emitted) | (reservation TTL-releases at 60s; no audit_outcome row written) |
| Proxy SIGTERM (graceful) | axum graceful_shutdown drains each handler; each handler runs its own happy/error path; no global enumeration (codex r2 P2-r2.D + r3 P2-r3.A fix) | natural commit or release per request |

**Note on 401 (user's OpenAI API key invalid)**: this is a CLIENT-side problem, but the proxy maps it to `PROVIDER_ERROR` because semantically the LLM call DID fail at the provider boundary. The audit reason is still `RUNTIME_ERROR` (collapsed). Operators inspecting `audit_outcome.payload_json` can see the original HTTP status code if the proxy embeds it (slice 5 acceptance: payload includes `upstream_status` + `upstream_error_class`). This gives the granularity audit consumers need without proto changes.

---

## 5. Token accounting

### 5.1 Pre-call estimate

`amount_atomic` in the ReserveSet claim comes from one of (priority order):

1. `X-SpendGuard-Estimated-Tokens` header (explicit override, trust the user)
2. `tiktoken`-equivalent: count messages.content tokens, multiply by completion-headroom (default 2x), add 50% buffer
3. Fallback: 1024 tokens (config knob `default_token_estimate`)

Estimate is **conservative-high**: better to over-reserve and refund the delta in commit_estimated than under-reserve and overflow. Refund delta is per the standard ledger ReserveSet → CommitEstimated lifecycle.

### 5.2 Post-call commit

OpenAI's response includes:
```json
{"usage": {"prompt_tokens": 18, "completion_tokens": 9, "total_tokens": 27}}
```

Proxy reads `usage.total_tokens` and calls CommitEstimated with `estimated_amount_atomic = total_tokens`. The ledger refunds `reserved - committed` automatically.

**Multi-unit budgets** (USD-denominated via `unit_conversion_version`): for v0.1 MVP, the reservation is in **token units**, and conversion to USD happens at commit time via the pricing snapshot loaded by the sidecar (same as wrapper mode). Proxy stays token-native.

### 5.3 Streaming detection (codex r2 P2-r2.B fix — request + response checks)

**Pre-reservation request check** (cheap, fail-fast):
- Parse request body's `stream` field. If `true`, return 501 + `code: spendguard_streaming_unsupported` BEFORE reserving.
- Don't reserve then fail — wastes a producer_sequence.

**Post-response response check** (defense for SSE-upgrades the request didn't declare):
- Modern OpenAI behavior: even with `stream: false` in the request, the API can return `Content-Type: text/event-stream` in edge cases (e.g., certain `tools` + `parallel_tool_calls=true` combos on some model versions, or future protocol upgrades).
- After step 11 (OpenAI response received), proxy MUST check `Content-Type` header:
  - `application/json` (or `application/json; charset=utf-8`): proceed to usage parse.
  - `text/event-stream`: treat as upstream error → emit `LLM_CALL_POST(PROVIDER_ERROR)` (release reservation per §4.4) → return 502 to client with `code: spendguard_unexpected_streaming_response` and a hint that v0.2 will support SSE pass-through.
  - Any other Content-Type: same as text/event-stream (502 + release).

**Acceptance test** (slice 5): integration test sends a request with `stream: false` to an upstream mock that returns `text/event-stream`; assert proxy responds 502 + the reservation is released (visible in ledger).

### 5.4 Upstream error handling

| Upstream outcome | Proxy action |
|---|---|
| 200 + usage block | CommitEstimated(usage.total_tokens) |
| 200 + missing usage | CommitEstimated(amount=estimate). Log warn. |
| 4xx (OpenAI rejected; e.g. invalid model, auth fail) | Release(reason=PROVIDER_ERROR). Forward 4xx to client. |
| 5xx (OpenAI / network) | Release(reason=PROVIDER_ERROR). Forward 5xx (or synthesize 502) to client. |
| Network timeout | Release(reason=CLIENT_TIMEOUT). 504 to client. |
| Proxy-side panic mid-response | Reservation TTL-releases (ledger default 600s) — recovery is graceful. |

---

## 6. Multi-tenant + multi-budget

### 6.1 Identification: env-default + per-request override (codex r1 P2.7 fix)

The launch story is **"1 env var: set OPENAI_BASE_URL and the proxy gates your calls."** That works only if the user's openai-python code can stay unchanged. Forcing per-request `X-SpendGuard-Tenant-Id` headers would require `OpenAI(base_url=..., default_headers={...})` — that's 4 lines, not 1, and breaks the wedge.

**Path A — Single-tenant proxy (the "1-env-var" launch claim)**:

Operator starts proxy with:

```bash
SPENDGUARD_PROXY_DEFAULT_TENANT_ID=00000000-0000-4000-8000-000000000001
SPENDGUARD_PROXY_DEFAULT_BUDGET_ID=44444444-4444-4444-8444-444444444444
SPENDGUARD_PROXY_DEFAULT_WINDOW_INSTANCE_ID=55555555-5555-4555-8555-555555555555
```

User's code stays:

```python
from openai import OpenAI
client = OpenAI(base_url="http://localhost:9000/v1", api_key="sk-...")
# no SpendGuard imports, no SpendGuard headers, no model wrapping.
client.chat.completions.create(model="gpt-4o-mini", messages=[...])
```

Proxy fills in `(tenant, budget, window)` from env on every request.

**Path B — Multi-tenant proxy (shared proxy across tenants/budgets)**:

User explicitly sets headers per call:

```python
client = OpenAI(
    base_url="http://proxy:9000/v1",
    api_key="sk-...",
    default_headers={
        "X-SpendGuard-Tenant-Id": "...",
        "X-SpendGuard-Budget-Id": "...",
        "X-SpendGuard-Window-Instance-Id": "...",
    },
)
```

Headers override env defaults. Use case: SaaS proxy serving multiple customers.

**Path C — Hybrid**: env-default tenant, per-request budget. E.g., one tenant has dev/staging/prod budgets selected per call.

**Validation rules** (all must hold or 400):
- All three IDs are valid lowercase-hyphen UUIDs
- No empty values (header explicitly set to "" → 400)
- Headers case-insensitive per HTTP; canonical form `X-SpendGuard-Tenant-Id`
- When env unset AND header absent → 400 with `code: spendguard_missing_identification`

### 6.2 Compatibility with wrapper-mode deployments (codex r1 P2.8 fix)

Proxy can coexist with Python SDK wrapper on the same budget. Ledger serializes writes per `(tenant_id, budget_id)` per Postgres SERIALIZABLE; per Phase 1 `single_writer_per_budget` constraint (memory `project_phase1_ledger.md`), the proxy is an additional writer for the same budget, not a parallel-writer per shard.

Contention scales with combined request rate. At Phase 1 SLO (decision-boundary p99 50ms, per Contract §14), the budget contention upper bound is ~20 decisions/sec/budget on a warm Postgres single-leader; mixing proxy + wrapper on one budget at high rate may push p99 above SLO. Not a v0.1 concern at launch traffic levels; document in operations runbook.

**Wire compatibility**: proxy uses same `RequestDecision` / `ConfirmPublishOutcome` / `EmitTraceEvents` RPCs as wrapper. No proto changes. Sidecar handler code unchanged.

### 6.3 Demo path

`DEMO_MODE=proxy` (slice 7) pre-seeds the same `(tenant=…001, budget=…444, window=…555)` tuple as other demo modes. Proxy starts with `SPENDGUARD_PROXY_DEFAULT_*` env vars pointing at these; demo container runs `OpenAI(base_url="http://egress-proxy:9000/v1", api_key=$OPENAI_API_KEY)` with **no headers**, validating the launch claim end-to-end.

### 6.4 Production tenant provisioning (deferred)

Real launch needs a `provision_tenant` CLI that creates budget + window + ledger accounts + outputs ready-to-use env vars. See §13.3. For v0.1, operators copy from `30_seed_demo_state.sh`.

---

## 7. Idempotency story

**Client-side retry safety**:
- Same `X-SpendGuard-Idempotency-Key` → same decision_id at ledger → ReserveSet replays → same response
- Default key when header absent: `sha256(canonicalized_body)[..16]` (canonicalization: sort JSON keys, strip whitespace)

**Ledger-side guarantees** (inherited from existing wrapper):
- ReserveSet UNIQUE on (tenant_id, operation_kind, idempotency_key) — first call wins
- CommitEstimated keyed on reservation_id — first commit wins; retries return same result
- DENIED decision UNIQUE on same tuple — first STOP "wins", retries return same 429

**OpenAI-side double-bill risk**:
- If client retries the proxy's CONTINUE response (because of e.g. network blip after proxy returned 200), the proxy MIGHT see the same idempotency_key and skip the OpenAI call on replay path (ledger returns same decision_id). But the proxy DOES still forward to OpenAI on each call by default — it relies on OpenAI's own idempotency if any (chat completions historically don't have it).
- Mitigation: proxy could cache (idempotency_key → OpenAI response) for the reservation_ttl_seconds window so a replay hits the cache. **DEFERRED to v0.2** — MVP documents the gap.
- For v0.1: user-side retry SHOULD generate a new `X-SpendGuard-Idempotency-Key` per attempt unless explicitly retry-safe. Document loud.

---

## 8. Security model

| Concern | v0.1 stance |
|---|---|
| User's `Authorization: Bearer sk-...` (their OpenAI key) | Forwarded byte-identical to OpenAI. NOT logged, NOT persisted, NOT used for SpendGuard tenant identification. Wrapped in `RedactedAuth` newtype (below). |
| SpendGuard auth (who can call the proxy) | v0.1 MVP: trust localhost (loopback only by default). The proxy binds 127.0.0.1:9000. To expose: operator deploys behind their own auth proxy. Document loud. |
| Cross-tenant header spoofing | v0.1 MVP: NO cross-check between Authorization owner and X-SpendGuard-Tenant-Id. Same trust model as wrapper-mode (the SDK trusts the calling process for tenant attribution). |
| TLS to OpenAI | reqwest with rustls + system root certs. Cert pinning is v0.2. |
| Mid-flight body modification | Forbidden. Proxy parses for read-only purposes. Test must verify byte-identity. |
| API key in logs | Structurally prevented via `RedactedAuth` newtype (below) + tracing filter + acceptance tests. |
| Sidecar UDS auth | Same as wrapper-mode: filesystem permission (0660) on `/var/run/spendguard/adapter.sock`. |

**RedactedAuth newtype** (codex r1 P1.6 fix — structural, not policy):

```rust
/// Wraps the user's bearer token. `Display` and `Debug` impls
/// MUST print "<redacted>" — never the underlying value. Compile-time
/// guarantee that a misplaced `{auth}` or `{auth:?}` in a tracing
/// macro cannot leak.
pub struct RedactedAuth(String);

impl std::fmt::Display for RedactedAuth {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "<redacted>")
    }
}

impl std::fmt::Debug for RedactedAuth {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "RedactedAuth(<redacted>)")
    }
}

impl RedactedAuth {
    /// Only path to the underlying value — must be called explicitly,
    /// typically only when building the upstream HTTP request.
    pub fn expose_secret(&self) -> &str { &self.0 }
}
```

`reqwest::header::HeaderValue::from_str(auth.expose_secret())` is the only call site. Audit at slice 2 + slice 4 review: grep for `expose_secret(` should show exactly one usage (upstream request construction).

**Tracing layer config — defense in depth** (codex r2 P2-r2.C fix):

`tower_http::trace::TraceLayer` defaults to NOT logging headers (good — verified against tower-http API docs), but `on_request` / `on_response` callbacks receive the full `http::Request<Body>` and the IMPL must choose what to log. A developer adding `?req` or `req.headers()` to a `tracing::info!` macro can break redaction silently. Defenses:

1. **Layer 1 — Tracing layer config** (must do):
   - `TraceLayer::new_for_http()` with custom `on_request` / `on_response` that explicitly log only allowlisted fields (method, path, status, latency). No `headers()`.
   - Drop the default `MakeSpan` impl which records all headers; use `DefaultMakeSpan::new().include_headers(false)` (the explicit default).

2. **Layer 2 — RedactedAuth newtype** (must do):
   - `Authorization` header extracted into `RedactedAuth` struct (above) BEFORE entering the tracing span.
   - Any `tracing::info!(?auth)` prints `RedactedAuth(<redacted>)`. Compile-time guarantee.

3. **Layer 3 — Lint / grep test** (must do, slice 4c acceptance):
   - CI grep: `grep -rE 'tracing::(info|warn|debug|trace)!.*[?!:]req|headers\(\)|"Authorization"' services/egress_proxy/src/` returns zero matches.
   - Or use clippy lint `disallowed_methods` with `http::HeaderMap::get("authorization")` flagged.

4. **Layer 4 — RedactedRequest<B> body newtype** (recommended; codex r3 P2-r3.E fix — strip-option removed because it breaks §3.4 byte-identical forwarding):
   - Custom newtype `RedactedRequest<B>` with redacting `Debug` impl that masks header values matching `^Bearer\s+sk-` regex.
   - The underlying header values remain accessible via `.expose_inner()` for the §3.4 byte-identical forwarding step. Display/Debug never leak.

5. **Layer 5 — Acceptance test** (must do, slice 2 + slice 4c):
   - Unit test: `RedactedAuth("sk-test-secret-1234").to_string() == "<redacted>"`.
   - Unit test: `format!("{:?}", RedactedAuth(...))` does NOT contain `sk-test-secret`.
   - Integration test: spawn proxy with `RUST_LOG=trace`, send `Authorization: Bearer sk-test-secret-decoy-1234567890abcdef`, assert captured stderr contains zero substring matches of `sk-test-secret-decoy` AND zero matches of `Bearer sk-`.
   - JSON-log inspection: `jq` filter on logs verifies no `authorization` field appears (case-insensitive). Test command in §10 acceptance.

**Acceptance tests** (§10):
1. Unit test: `RedactedAuth("sk-test-secret-1234").to_string() == "<redacted>"`.
2. Unit test: `format!("{:?}", RedactedAuth(...))` does not contain `sk-test-secret`.
3. Integration test: spawn proxy with `RUST_LOG=trace`, send `Authorization: Bearer sk-test-secret-decoy-XXXXXXXXXX`, assert captured stderr contains zero substring matches of the decoy AND zero matches of `Bearer sk-`.
4. JSON-log inspection: `jq` filter on logs verifies no `authorization` field appears (case-insensitive).

**Threat model for v0.1**: user runs proxy on their own machine / pod. Attacker on the same machine (root, or with `audit_outbox` read access) could see audit chain — but not the API key. NOT a public-internet-facing service in v0.1. README MUST say this in the security section of README and docs/site.

**Threat model for v0.1**: user runs proxy on their own machine / pod. Attacker on the same machine (root, or with `audit_outbox` read access) could see audit chain. NOT a public-internet-facing service in v0.1. README MUST say this in the security section of README and docs/site.

---

## 9. Failure modes

| Mode | Behavior | User sees |
|---|---|---|
| Sidecar UDS not connectable at startup | Proxy startup loop: retry connect+handshake with 1s backoff up to 30s; then exit 1 | Operator: 30s grace; fix sidecar |
| Sidecar handshake fails after connect | Same retry-then-exit | Operator: structured error in logs |
| Sidecar disconnect mid-flight | Proxy treats as 502 (per §4.2 fail-closed); APPLY_FAILED emitted best-effort | Client: 502 + `code: spendguard_sidecar_unavailable` |
| OpenAI 5xx / network | Forward upstream status; `EmitTraceEvents(LLM_CALL_POST, outcome=PROVIDER_ERROR)` releases reservation | Client: original 5xx; subsequent retry safe (default per-attempt keys) |
| Proxy SIGTERM (graceful) | axum `graceful_shutdown()` drains each handler future naturally; each running handler completes through its own happy-path (commit) or error-path (release) per §4.4. No global registry; no enumeration (codex r2 P2-r2.D fix). | Client: existing in-flight requests drain to completion (up to drain_window deadline, default 30s); new requests refused with connection close |
| Proxy SIGKILL / OOM / panic | OS kills proxy; in-flight reservations TTL-release after 60s (knob `SPENDGUARD_PROXY_RESERVATION_TTL_SECONDS`, default 60; lower than sidecar's 600 because proxy-driven calls complete fast) | Client: connection drops; budget freed in ≤60s; fallback to native OpenAI is the bypass risk (§11) |
| Malformed JSON body | 400 fail-fast; no reservation | Client: standard 400 |
| Body exceeds size limit (16 MB MVP) | 413; no reservation | Client: 413 |
| Sidecar contract bundle unloadable | Sidecar fails its own readyz; proxy /readyz reflects (502 on health) | Operator: fix bundle |
| Reservation TTL deadlock (codex r1 P2.2) | Default TTL lowered to 60s for proxy reservations vs 600s for wrapper. Plus graceful SIGTERM handler. | Budget unavailable for ≤60s on hard crash; document in operations runbook |

**Invariant**: every code path that has reserved on the ledger has a corresponding commit OR release path within the proxy. No "reserved, forgotten" path. TTL is the safety net (panic case) but normal-shutdown path explicitly releases via SIGTERM handler. Slice 5 acceptance verifies via fault-injection.

---

## 10. Acceptance criteria

For each slice the following must hold before merge:

1. **Compiles clean**: `cargo check -p spendguard-egress-proxy` returns 0.
2. **Unit tests pass**: `cargo test -p spendguard-egress-proxy` returns 0. Test count ≥ described in slice.
3. **Demo smoke**: `DEMO_MODE=proxy make demo-up` (after slice 7) exits 0 with assertions logged.
4. **No tonic CryptoProvider regression**: every `main()` installs aws_lc_rs default provider (per F1 backport).
5. **No SpendGuard key in logs**: `docker compose logs egress-proxy | grep -E 'sk-[a-zA-Z0-9-]{16,}'` returns empty.
6. **Codex review**: see §14.
7. **No file deleted outside slice scope**.
8. **No unrelated formatting churn**.

End-to-end acceptance (slice 7 + slice 8):

```bash
make demo-down -v
DEMO_MODE=proxy make demo-up
# expect: proxy alive on 9000, ledger reservation row, commit_estimated row,
#         audit chain visible in canonical_events; STOP test via curl returns 429
#         with structured body; latency-budget p99 < 50ms decision-only

# 1-line onboarding test (the launch claim):
docker run --rm -e OPENAI_API_KEY=sk-... \
  --network spendguard-net \
  python:3.12-slim \
  bash -c "pip install openai && python -c \"
from openai import OpenAI
c = OpenAI(base_url='http://egress-proxy:9000/v1', api_key=__import__('os').environ['OPENAI_API_KEY'],
           default_headers={'X-SpendGuard-Tenant-Id':'00000000-...001',
                            'X-SpendGuard-Budget-Id':'44444444-...444',
                            'X-SpendGuard-Window-Instance-Id':'55555555-...555'})
print(c.chat.completions.create(model='gpt-4o-mini', messages=[{'role':'user','content':'Hi'}]).choices[0].message.content)
\""
# expect: real OpenAI response, ~1-2 seconds, no SDK install, no Python code touching SpendGuardClient
```

---

## 11. Capability level — honest (codex r1 P2.1 fix)

**Spec-internal name**: `L1.5 partial-L2 egress_proxy_opt_in` for v0.1. Distinct from the existing config string `egress_proxy_hard_block` (L2 proper), which remains an aspirational future state requiring NetworkPolicy enforcement.

Honest matrix:

| Bypass attempt | v0.1 proxy-mode blocks? | What would block it |
|---|---|---|
| User imports `openai.OpenAI()` with default base_url (api.openai.com) | NO | NetworkPolicy `egress: only proxy:9000` (k8s; deferred §13.5); LD_PRELOAD shim (out of scope) |
| User imports `openai.OpenAI(base_url="proxy")` with no identification (path A env unset + no headers) | YES (400 fail-fast) | Already v0.1 |
| User imports `openai.OpenAI(base_url="proxy")` with spoofed tenant header (path B) | NO | JWT-claim tenant identification (deferred §13.8) |
| Containerized agent without network egress except via proxy | YES | NetworkPolicy ensures this is the only path |
| Process MITM (user replaces proxy binary or runs spoofed proxy) | NO | mTLS to proxy + Helm-pinned trust root — same as sidecar's adapter UDS |

**Naming convention going forward**:
- `L1 semantic_adapter`: wrapper-mode (today; SDK refuses on STOP, agent can `import openai` directly to bypass)
- `L1.5 partial-L2 egress_proxy_opt_in`: v0.1 of this spec (proxy enforces if user routes through it, but doesn't FORCE routing)
- `L2 egress_proxy_hard_block`: v0.1 + NetworkPolicy templates (deferred §13.5)
- `L3 provider_key_gateway`: agent never sees the API key (deferred entirely)

README + docs MUST advertise as **"drop-in onboarding for L1.5; L2 enforcement requires NetworkPolicy"**. Do NOT sell L2 unconditionally.

### 11.1 FIPS posture (codex r5 Staff #2 fix)

blake2b is **not** a FIPS-140-2 approved hash. v0.1 ships with blake2b because all 3 production SDKs use blake2b (`sdk/python/src/spendguard/ids.py:131,167`; `langchain.py:149`; `openai_agents.py:125`) and cross-mode ID convergence is the load-bearing property motivating the egress proxy. Operators with FIPS-compliance requirements:

- **v0.1**: not supported. README + slice 8 docs MUST loudly note "blake2b is the v0.1 hash; FIPS-compliant operators should defer adoption until v0.2."
- **v0.2**: planned `--hash-algo=sha256` build flag (Cargo feature) that swaps blake2b → sha256 across both the Rust port and the Python SDK in lockstep; cross-language byte-equivalence fixtures regenerate. Operators flip the flag at deploy time; mixed-flag deployments are not supported (would break cross-mode convergence).
- **L3 provider-key-gateway path** (deferred §13.x): the eventual key-gateway design will likely use HMAC-SHA256 by default since key derivation requires a FIPS-approved KDF.

The helper is NOT a cryptographic primitive (no MAC / KDF / signature semantics) — it is a content-addressing label. FIPS does not strictly require FIPS-approved hashes for content addressing. The §11.1 disclosure ensures regulated buyers can make an informed adoption decision rather than discover the algorithm choice after deployment.

---

**On `enforcement_strength` capability advertising** (codex r3 P2-r3.B + P2-r3.H + r4 P2-r4.B fix — DOCUMENTATION ONLY, no code path):

Verified `services/sidecar/src/config.rs:42-46`: `enforcement_strength` is a **free-form `String`** with default `"semantic_adapter"`. It is NOT validated, NOT typed, and only logged at startup (per `main.rs:36`). No code branches on its value. The doc-comment lists accepted strings `advisory_sdk / semantic_adapter / egress_proxy_hard_block / provider_key_gateway` but this is documentation, not validation.

**Operational reality** (codex r4 P2-r4.B): adding `egress_proxy_opt_in` to the doc-comment is **informational only**. The string flows through `Config::enforcement_strength` → handshake reply → user SDK without any code seeing it. No client today branches on this field's value. Marking it in the docs lets operators reading the source see the option; it doesn't activate any new code path. This matches the existing pattern for `egress_proxy_hard_block` (also a documented string with no validation).

Architecture clarification: the **proxy is a UDS client of the sidecar** (per §2 D2), so the proxy does NOT do its own adapter `Handshake` upstream of itself. The sidecar's `enforcement_strength` env var is the operator-set advertising knob; if the operator deploys the proxy alongside the sidecar AND wants honest capability claims, they should set `SPENDGUARD_SIDECAR_ENFORCEMENT_STRENGTH=egress_proxy_opt_in` on the sidecar. This propagates through the existing handshake to user code; user code reading the handshake response sees `enforcement_strength="egress_proxy_opt_in"` and can branch on it.

Slice 8 changes:
- `services/sidecar/src/config.rs:43` doc-comment: add `egress_proxy_opt_in` to the accepted-strings list. No code change (still free-form String).
- README + docs/site: operator runbook says "set `SPENDGUARD_SIDECAR_ENFORCEMENT_STRENGTH=egress_proxy_opt_in` if you deploy the proxy".

No proto change. No `HandshakeRequest` schema change. The proxy itself doesn't advertise anything new — the sidecar's existing capability advertising mechanism is reused.

---

## 12. Streaming (v0.2 design preview)

Not in MVP but locking the future direction so the v0.1 code doesn't paint into a corner:

- SSE pass-through: proxy reads upstream `text/event-stream` line-by-line, forwards each `data: ...` chunk to client immediately.
- Each chunk's `delta.content` token count is accumulated client-side.
- On EOF: commit_estimated(actual_tokens).
- Mid-stream cap (if a streaming response exceeds reservation): proxy sends a synthetic SSE event `{"error":{"code":"spendguard_streaming_cap_exceeded",...}}` and closes connection. Reservation released.
- Doesn't apply to v0.1 because non-streaming is the 80% target.

---

## 13. Deferred items (track as GitHub issues post-merge)

1. Multi-provider routing (Anthropic `POST /v1/messages`, Gemini, Bedrock) — header sniff or path prefix
2. Streaming SSE (see §12)
3. `provision_tenant` CLI — one-liner that creates budget + window + ledger accounts + outputs headers to copy
4. Body-cache idempotency for OpenAI double-bill safety (§7)
5. NetworkPolicy templates for k8s deployments (L2 enforcement)
6. Per-tenant rate limit at proxy (in addition to budget gate)
7. Cert pinning to OpenAI
8. JWT-based tenant identification (replace X-SpendGuard-* headers with verified claims)
9. Embeddings + completions + images endpoints
10. v0.2 contract DSL extension: rule's `matched_route` includes egress-proxy-only checks
11. **Audit-outbox oracle defense** — replace `signature = blake2b_128(canonicalized_body)[..16]` with `signature = HMAC(K_tenant, canonicalized_body)[..16]` where K_tenant is a per-tenant secret read from sidecar config. Closes the same-machine-attacker-with-audit_outbox-read-access vector flagged by codex r5 Staff #2 (an adversary that can SELECT step_id from audit_outbox can re-derive content body via prompt-corpus brute force). v0.1 stance: same-machine trust per §8 threat model; v0.2 raises the bar via tenant-keyed HMAC.

---

## 14. Codex review standards

Per `feedback_codex_review.md` + `feedback_codex_iteration_pattern.md`:

**Per-slice review loop**:
1. Adversarial review against the staged diff
2. Stopping rule: GREEN (no P1, no P2-critical-path) → ship; RED → fix → re-review
3. Max 5 rounds per slice
4. If still RED after r5: escalate to Staff-team consensus via parallel Agent dispatch (4 angles: distributed-systems, security, product, infrastructure); synthesize their answers into a decision; ship the consensus solution
5. Memory entry per slice if codex caught a non-obvious issue

**Per-spec review angles (this doc)**:
1. **Security**: API key trust boundary, header spoofing, log leakage, TLS, sidecar UDS auth
2. **Reliability / failure modes**: every reservation has commit/release; sidecar disconnect; partial response; idempotency double-bill
3. **Capability level honesty**: don't oversell L2 without NetworkPolicy
4. **Wedge protection**: STOP truly blocks (not just logs); 429 semantics; differentiation vs Helicone
5. **MVP scope cut realism**: streaming, multi-provider, approval/degrade — defer-or-ship judgments
6. **Acceptance criteria completeness**: every claim in spec has a test
7. **Backward compatibility**: doesn't break existing wrapper-mode, doesn't change sidecar wire contract

**Definition of "codex GREEN"**: zero P1 findings + zero P2 findings in critical-path categories (security / reliability / wedge / capability honesty). P2 docs-clarity and P3 hygiene allowed; folded as inline doc comments per CA-P3.7 precedent.

### 14.1 Staff escalation playbook (codex r4 P2-r4.A fix)

Triggered when a slice has 5 consecutive codex RED reviews. Operational steps:

**Step 1: Trigger conditions** — all of:
- 5 distinct codex review rounds completed for the slice (r1-r5)
- r5 verdict is RED (any P1 OR P2-critical)
- The same root-cause finding has been flagged in ≥3 of the 5 rounds (i.e., not new bugs introduced each round; a stubborn disagreement)

**Step 2: Dispatch Staff team — 4 parallel Agent invocations** (`superpowers:code-reviewer` subagent with role-specific prompts):
- **Distributed systems angle**: contention, idempotency, race conditions, partial failures
- **Security angle**: trust boundaries, key handling, injection vectors, log redaction
- **Product angle**: launch claim viability, friction, abandonment rate, user trust
- **Infrastructure angle**: deployability, observability, ops burden, debugging

Each Staff agent receives:
1. The current spec/diff under review
2. The full r1-r5 codex review history (verdicts + punch lists)
3. The specific contested finding(s)
4. Prompt: "Recommend a decision on this finding. Options: (a) accept the finding and rewrite; (b) reject the finding with justification; (c) split the difference. Explain trade-offs in <200 words. Include a concrete fix recipe for option (a) or (c)."

**Step 3: Synthesis** — Claude reads all 4 Staff outputs. Decision criterion (in priority order):
1. If 3 of 4 agree on (a), (b), or (c): take that path.
2. If split 2-2: tiebreaker is the user explicitly. Pause the slice; surface the 4 opinions + 1-paragraph synthesis; ask the user to decide. (This is the ONLY case in the autonomous loop where user input is requested.)
3. If 4-way disagreement: same as (2) — surface to user.

**Step 4: Consensus document** — written to `docs/specs/<spec-file-stem>-staff-escalation-rN.md` (where `<spec-file-stem>` is the spec filename without `.md`, e.g., `auto-instrument-egress-proxy` → `auto-instrument-egress-proxy-staff-escalation-r5.md`; N is the round number that triggered escalation). Codex r6 P2-r6.3 slug-convention clarification. Contains:
- Original finding (verbatim from codex)
- 4 Staff opinions (verbatim outputs)
- Synthesis (Claude's read)
- Decision (option a/b/c) + rationale
- Concrete fix applied (or "rejected because X")
- Codex rN+1 review verdict on the fix (e.g. r6 verdict for an r5 escalation)

**Step 5: Freeze rule** — during Staff deliberation (between rN RED and rN+1 review):
- Spec under review CAN be edited (to apply the Staff fix)
- **Slices that structurally depend on the spec section under deliberation** (e.g., a slice that imports a crate moved/renamed by the Staff fix) MUST wait for rN+1 sign-off. Codex r6 P2-r6.3 dependent-slice clarification: a slice depends structurally if its acceptance criteria reference a section the Staff fix is modifying.
- **Slices NOT structurally affected** by the Staff fix may proceed referencing the pre-Staff spec text. In practice for this 11-slice plan, dependency chains 2→4b→5 and 7a→7b are tight; freeze typically blocks all downstream slices.
- After rN+1, if Staff fix is GREEN: merge slice; subsequent slices reference the post-Staff spec.
- If rN+1 still RED after Staff fix: escalate to user as "Staff team could not reach actionable consensus; please decide".

**Step 6: Failsafe — abort condition** — if r10 (5 more rounds after Staff fix) still RED, the slice is structurally infeasible; surface to user with "this slice cannot be shipped autonomously; recommend redesign or scope cut".

**Authority hierarchy**:
- Claude (this agent): decision-maker for codex r1-r5 + Staff synthesis on 3-of-4 majority
- User: tiebreaker for Staff 2-2 / 4-way split / r10 abort
- Staff team (4 parallel sub-agents): recommendation only, never autonomous decision

This playbook applies to ALL 11 slices in this spec. Slice 1 (this file) has reached r4 RED → r5 will be the last codex round before Step 1 triggers if r5 is also RED.

---

## 15. Slice breakdown — v2 (11 slices after codex r1 P2.5/P2.6 splits)

| # | Title | Surface | Codex angles |
|---|---|---|---|
| 1 | Spec doc (this file) | Design | Security, reliability, capability honesty, wedge protection, scope cut, slice ordering |
| 2 | Crate skeleton + /healthz + RedactedAuth + tracing + `spendguard-ids` shared crate | (a) New `services/egress_proxy/` Cargo + main + Dockerfile + compose entry; rustls CryptoProvider; structured logger with header filter. (b) NEW shared crate `services/ids/` (mirrors `services/policy/` pattern) containing `default_call_signature_jcs` + `derive_uuid_from_signature` (blake2b-128 + v4-shape masking); cross-language byte-equivalence fixture at `services/ids/tests/fixtures/python_v1.json` (committed by Python SDK `tests/test_ids_fixtures.py`). Codex r5 Staff #3 moved this from Slice 4b. | Build correctness, structural log-redaction, no startup leak; `aws_lc_rs::default_provider().install_default()` invoked; **cross-language byte-equivalence test**: Rust port produces same UUIDs as Python helper for every fixture row |
| 3 | HTTP pass-through forwarder | reqwest → api.openai.com; byte-identity verify (no gating yet); stream-true → 501; body size limit | Body-mutation forbidden, header-pass-through (incl. Authorization unchanged), upstream error mapping (5xx → 502 with structured fail block) |
| 4a | Sidecar UDS gRPC client + handshake + /readyz | tonic UDS client; connect+handshake retry-with-backoff loop (1s × 30s); /readyz reflects handshake state | Retry budget, connect-then-exit semantics, readyz wait |
| 4b | DecisionRequest construction + per-request state | Parse model; estimate tokens; build BudgetClaim; mint ids via `spendguard-ids` crate (from Slice 2); ReservationContext threading; ARC-swap pricing cache with mtime refresh per §4.1.5 | DecisionRequest field correctness vs sidecar wire; enrichment threading (run_id, model_family); ReservationContext lifecycle; pricing cache refresh test (codex r3 P2-r3.C) |
| 4c | Fail-closed decision routing + no-runtime-env-re-read test | Match `Continue → forward` (only branch); all other variants + errors → 502/429/503/501 per §3.3; sidecar partial response → 502. NEW (codex r3 P2-r3.G): integration test that overwrites runtime.env between PRE and POST; assert LLM_CALL_POST payload carries OLD pricing hash (the frozen one from PRE) | Pre-vs-post forward ordering (zero-OpenAI-call test for non-Continue); fail-closed invariant test matrix; PRE→POST pricing freeze invariant |
| 5 | Commit lane: LLM_CALL_POST + ConfirmPublishOutcome + error release | Happy-path 12a (LLM_CALL_POST) + 12b (ConfirmPublishOutcome) in spec order; error paths per §4.4 (single RPC, never both); SIGTERM via axum graceful_shutdown drain (codex r3 P2-r3.A fix: each handler runs its own commit/release path — NO global enumeration of in-flight contexts) | Refund-delta correctness; release on every error path (unit + fault-injection test); graceful_shutdown drain test: 100 in-flight requests at SIGTERM; assert each ends in either commit or release within 30s |
| 6 | Identification: env-defaults + per-request header override | `SPENDGUARD_PROXY_DEFAULT_*` env vars; X-SpendGuard-* parsing; 400 on missing-everywhere; the "1-env-var" launch claim test | Header spoofing acknowledged; env-vs-header precedence; missing-identification fail-fast |
| 7a | DEMO_MODE=proxy bring-up | compose service + Makefile entry; proxy boots with sidecar; /readyz passes; healthcheck wired | Service ordering, healthcheck timing, port mappings, volume mounts for shared bundles-data + sidecar-uds |
| 7b | DEMO_MODE=proxy e2e smoke driver | Python container that does `from openai import OpenAI; OpenAI(base_url=...).chat.completions.create(...)`; pre-flight pre-check; verify reservation + commit_estimated + audit chain in ledger; STOP path test with budget-too-small | E2E PASS demo-quality-gate; the launch claim runs end-to-end; structured error body on STOP; no API key in logs (acceptance §10.5b+c) |
| 8 | Final adversarial sweep + spec lock + memory + README + sidecar config doc-comment | Codex final review on the whole slice; spec file marked LOCKED; memory `project_overview.md` updated; README repositioned (lead with 1-env-var claim); `services/sidecar/src/config.rs:43` doc-comment adds `egress_proxy_opt_in` to accepted-strings list (codex r3 P2-r3.B fix — no validation code change, no proto change; sidecar's enforcement_strength remains free-form String; operator sets `SPENDGUARD_SIDECAR_ENFORCEMENT_STRENGTH=egress_proxy_opt_in` to advertise via existing handshake) | Full surface; honest framing; cross-spec consistency (Phase 1 ledger constraints + Contract DSL invariants) |

Each slice ships independently. Slice N must not block on slice N+1 review.

**v2 acknowledgments of codex r1 P3 findings** (not blocking ship):
- §5.3 pseudo-code: explicit check ordering in §4.1 step list (now 14 steps, ordered).
- §3.2 idempotency key: kept at 16 hex chars (64 bits) — adversarial collision out of v0.1 threat model; doc clarifies.
- §12 streaming preview: out of scope for v0.1; v0.2 tracked.
- §13 deferred items #3 `provision_tenant` CLI: moved to launch-blocker list IF launch reception calls it out; v0.1 demo path still works without it.
- §10.4 CryptoProvider regression: slice 2 acceptance explicitly invokes `aws_lc_rs::default_provider().install_default()` per F1 backport.

---

**End of spec v1 strawman.**
