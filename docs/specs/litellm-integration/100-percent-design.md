# LiteLLM Integration → 100% Design Lock

**Status**: Proposed (design lock; no code) — 2026-05-20
**Scope**: 3 epics to close the production / onboarding / audit-chain
gaps from 85 / 80 / 95% to 100%.
**Constraints**: SDK is at HEAD on `main`; sidecar v1.0 wire format is
frozen; ADR-005 (sync deferred) remains in force.

---

## Epic A — `spendguard.integrations.litellm.acompletion()` async wrapper

**Problem.** DESIGN §3.4 Path A routes direct async callers via the
Shape A egress proxy. That works but (1) gates at the HTTP boundary,
losing structured callback context (`team_id`, `litellm_call_id`,
`stream`), and (2) forces operators to run a second proxy hop even
when they already use `litellm.acompletion()` directly. We add an
opt-in **in-process** async wrapper that gates direct callers with
the same fail-closed contract as Path B (proxy callback). Architectural
call: ship as a **per-call decorator wrapper** (not module-level
monkey-patch), because LiteLLM Slice 1 already burned us on monkey-
patching (`project_litellm_slice1_blocked.md`).

### Surface contract

```
spendguard.integrations.litellm.acompletion(
    *,
    client: SpendGuardClient,
    budget_resolver: BudgetResolver,
    claim_estimator: ClaimEstimator,
    claim_reconciler: ClaimReconciler,
    run_context: LiteLLMRunContext | None = None,
    fail_closed: bool = True,
    **litellm_kwargs: Any,
) -> Any  # forwards to litellm.acompletion(**litellm_kwargs)
```

- **Forwarded kwargs**: every `litellm_kwargs` key passes verbatim to
  `litellm.acompletion(...)`. SpendGuard kwargs are **keyword-only**
  and stripped before forwarding (mirrors `httpx.AsyncClient`
  pattern).
- **Configuration model**: **per-call kwargs**, not module-level. Per-
  call wins because operators with multi-tenant Python services need
  resolver state per request (not global), and module-level config
  re-introduces the global-state coupling that ADR-001 forbids.
- **File**: `sdk/python/src/spendguard/integrations/litellm.py`
  (append; do NOT new-file — same module so `__all__` exports stay
  one symbol set).
- **Reuse**: `acompletion()` instantiates a **transient inline
  callback** that shares 100% of the dataclasses + helpers
  (`_build_resolver_ctx`, `_build_decision_context`,
  `_validate_claim_against_binding`, `compute_prompt_hash`) with
  `SpendGuardLiteLLMCallback`. It does **not** subclass `CustomLogger`
  (no proxy needed) and does **not** use `_LoopBoundCallback` (caller
  already on the right loop — they `await`-ed us). Implementation
  shape: extract `_pre_call_core` / `_commit_core` / `_release_core`
  pure-functions from the existing callback class; both surfaces call
  the cores. ~120 LOC net new (helpers stay shared).

### Failure modes

| Sidecar/SDK signal | User-facing |
|---|---|
| `outcome.decision == STOP/REQUIRE_APPROVAL/...` | `DecisionDenied` (status 403); LiteLLM call never made |
| `outcome.decision == DEGRADE` | `SidecarUnavailable` (status 503); fail-closed per DESIGN §5 |
| Pre-call sidecar error (transport/handshake) | `SidecarUnavailable` (503); fail-closed |
| `litellm.acompletion()` raises | Reservation released best-effort via `emit_llm_call_post(outcome=FAILURE/CANCELLED)`; **original LiteLLM exception re-raised** (mirrors `async_log_failure_event` swallow contract) |
| Commit-time sidecar error | Logged WARN; TTL-sweep backstop; LiteLLM result still returned to caller (commit failure must not mask a SUCCESSFUL provider call) |
| `SPENDGUARD_LITELLM_FAIL_OPEN=1` | Allows pre-call errors past; DEV ONLY; logs WARN once per process (same env var as callback) |
| **Sync `litellm.completion()`** | **NOT exposed**. No `spendguard.integrations.litellm.completion()` symbol. ADR-005 confirmed. Sync callers continue to use Shape A egress (DESIGN §3.4 Path A). |

### Slice breakdown (≤300 LOC each)

| Slice | LOC | Files |
|---|---|---|
| **A1** — Core extraction (no behavior change) | 200 | `litellm.py` (+120 refactor / -0 deletion: extract `_pre_call_core`, `_commit_core`, `_release_core` from class methods to module funcs); `tests/integrations/test_litellm_callback.py` (+80: assert no regression) |
| **A2** — `acompletion()` wrapper | 240 | `litellm.py` (+140: `async def acompletion(...)`, decision-context building, retry-aware reservation lifecycle, snake-around `try/except/finally`); `tests/integrations/test_litellm_acompletion.py` (+100: ALLOW + DENY + DEGRADE + provider-raises + fail-open dev) |
| **A3** — Docs + export + smoke | 100 | `litellm.py` (`__all__` +1 symbol); `DESIGN.md` §3.4 Path C subsection (+40 lines); `examples/litellm-direct-acompletion/demo.py` (+50: minimal runnable example referenced from PROXY_RECIPE) |

### Test plan

- **Tier 1 unit**: monkeypatch `litellm.acompletion` with a stub
  AsyncMock; cover ALLOW happy path, DENY raises, DEGRADE raises,
  provider raises → release fires, commit raises → result still
  returned, fail-open allows through.
- **Tier 2 integration**: run against a real sidecar UDS in-process
  with the existing `tests/integration/` harness; exercise idempotency
  on retry (same `litellm_call_id` → same `decision_id`).
- **Tier 3 demo**: `examples/litellm-direct-acompletion/demo.py` —
  ALLOW + DENY 2-step demo (no proxy container). Not part of the
  litellm-proxy-composite Make target; standalone runnable.

### Spec dependencies

- **References**: DESIGN §3.4 (Path A vs Path B classification), §5
  (DEGRADE fail-closed), §6 (single-claim contract), §8.2a (12-field
  decision_context), ADR-001 (no global default), ADR-005 (sync
  deferred).
- **Updates needed**: DESIGN §3.4 gains a **Path C: in-process async
  wrapper** subsection (~40 lines: surface, when to choose Path C vs
  Path A, idempotency contract). ACCEPTANCE.md §3 gains a Path-C
  acceptance bullet ("direct async wrapper exercised in
  `litellm_direct_acompletion` demo mode"). ADR-005 status unchanged.

### Out of scope

- Sync `litellm.completion()` wrapper (ADR-005 still NO).
- Auto-detect proxy-mode-vs-direct-mode at runtime (operator chooses
  explicitly).
- Streaming via `litellm.acompletion(stream=True)` returning an
  iterator — **deferred to a follow-up epic**; Slice A2 raises
  `SpendGuardConfigError("stream=True not yet supported in Path C; use
  Path B proxy")` if the caller passes `stream=True`.
- Per-call TTL override (matches callback Slice 4 R1 P1.3).
- Mid-flight cancellation cleanup beyond what `async_log_failure`
  already does.

---

## Epic B — `examples/litellm-proxy-composite/`

**Problem.** PROXY_RECIPE.md is operator-facing but is a template, not
a runnable demo. `openai-agents-composite/` (shipped) sets the bar:
one Make target, a docker-compose, a stripped resolver/estimator/
reconciler file, and an `app.py` that hits the local proxy. Operators
need the same shape for LiteLLM. Architectural call: **mirror the
openai-agents-composite layout exactly** (no novel topology), strip
all demo-mode branching from the operator callback file so a fork is
deletable down to ≤120 LOC.

### Surface contract

```
examples/litellm-proxy-composite/
├── README.md                  # 4-step quickstart (≤180 lines)
├── docker-compose.yaml        # 3 services: postgres, sidecar, litellm-proxy
├── proxy_config.yaml          # LiteLLM proxy config (callback wired)
├── spendguard_callback.py     # OPERATOR FORK TARGET — bare minimum
├── app.py                     # 4-step demo client (httpx → localhost:4000)
├── budgets.yaml               # Contract DSL: 2 budgets (team-a, team-b)
├── Makefile                   # demo-up, demo-run, demo-down, demo-verify
└── requirements.txt           # litellm[proxy], spendguard-sdk[litellm], httpx
```

- **`spendguard_callback.py`** (~120 LOC, **zero** demo branches):
  defines `budget_resolver`, `claim_estimator`, `claim_reconciler`,
  and instantiates a single `_LoopBoundCallback`. NO test-mode flags,
  NO `if os.environ.get("DEMO_MODE")`, NO mocked claims. Operators
  fork this file, swap budget IDs, ship.
- **`app.py`** (~180 LOC): drives the 4-step demo via **direct
  `httpx.AsyncClient` POST to `http://localhost:4000/v1/chat/
  completions`** with `Authorization: Bearer sk-<team-a-key>`. No
  `litellm.acompletion()` — the point is to prove the proxy path
  works for a plain OpenAI-API HTTP client. Steps: (1) team-a ALLOW
  non-stream, (2) team-a ALLOW stream, (3) team-a DENY (over budget),
  (4) team-b ALLOW (multi-tenant isolation).
- **Compose topology**: 3 containers on a `spendguard-net` bridge.
  `postgres:16-alpine` (ledger), `spendguard-sidecar:v1.0` (UDS at
  `/var/run/spendguard/sidecar.sock`, exposed via shared volume),
  `litellm-proxy` (mounts `proxy_config.yaml` + the shared sidecar
  socket volume). LiteLLM proxy listens on host port 4000. Env vars:
  `OPENAI_API_KEY` (host-provided), `LITELLM_MASTER_KEY=sk-master`,
  `SPENDGUARD_SOCKET_PATH=/var/run/spendguard/sidecar.sock`.

### Failure modes

| Failure | User-facing |
|---|---|
| `OPENAI_API_KEY` missing | `make demo-up` fails fast with explicit error before container start |
| Postgres slow to boot | sidecar `_ensure_client` 5s deadline absorbs; `make demo-run` retries proxy health-check 10× × 1s |
| Sidecar UDS not mounted into proxy container | proxy boot logs "spendguard handshake failed within 5s" — README §Troubleshooting points at compose volume mount |
| Step 3 DENY: caller receives HTTP 400 from LiteLLM proxy | This is correct (LiteLLM converts callback exception → 400). README documents `error.type == "BadRequestError"` is the success signal for step 3. |
| Q3 forensic SQL returns 0 rows | Means GH #77 (Epic C) hasn't landed — README links to Epic C status |

### Slice breakdown (≤300 LOC each)

| Slice | LOC | Files |
|---|---|---|
| **B1** — Topology + boot | 280 | `docker-compose.yaml` (+90), `proxy_config.yaml` (+50), `budgets.yaml` (+40), `Makefile` (+60: up/down/logs targets), `requirements.txt` (+6), `README.md` quickstart §1-2 (+34) |
| **B2** — Operator callback + app driver | 290 | `spendguard_callback.py` (+120), `app.py` (+180: 4-step asyncio.gather driver, httpx with explicit `Authorization` per step) — minus 10 shared imports |
| **B3** — Verify + docs | 220 | `Makefile` (+30: `demo-verify` runs psql Q1+Q2+Q3 against ledger), `README.md` §3-4 (+130: troubleshooting, fork-this-file checklist), `tests/test_composite_smoke.py` (+60: subprocess `make demo-up && demo-run && demo-verify && demo-down`) |

### Test plan

- **Tier 1 unit**: NONE. This is a demo, not a library.
- **Tier 2 integration**: `tests/test_composite_smoke.py` runs `make
  demo-up && make demo-run && make demo-verify && make demo-down` as
  one subprocess; asserts exit 0. Skipped on CI without Docker
  (`@pytest.mark.skipif(not docker_available)`); MANDATORY locally
  before each PR per `feedback_demo_quality_gate.md`.
- **Tier 3 demo**: `make demo-run` is itself the Tier 3 gate.
  Acceptance: all 4 steps print `[demo] PASS`, Q1 returns the right
  reserve/commit/denied counts, Q2 returns 3 joined rows, Q3 returns
  0 orphans.

### Spec dependencies

- **References**: PROXY_RECIPE.md (template source), DESIGN §3.4 Path
  B, §7.2 (proxy config shape), ACCEPTANCE.md §5.1 (Q1/Q2/Q3 SQL),
  ADR-001 (no global default).
- **Updates needed**: PROXY_RECIPE.md gains a "see also
  `examples/litellm-proxy-composite/` for a runnable reference"
  pointer (5 lines). ACCEPTANCE.md §3 adds bullet "composite demo
  Make target green" (mirrors openai-agents-composite acceptance).
- **Epic dependency**: B3's `demo-verify` SQL Q3 only returns
  meaningful forensics **after Epic C lands**. B3 can ship using Q1+Q2
  only; Q3 wiring goes in once Epic C is merged. Sequence: B1 → B2 →
  C → B3 (verify).

### Out of scope

- Kubernetes manifests (compose only; matches openai-agents-composite).
- Multi-region / fencing demo (single-region).
- Auth proxy in front of LiteLLM (LiteLLM virtual-key only).
- Live-reload bundle hot-swap (boot-time bundle load).
- Cost Advisor wiring (separate audit-pipe consumer; not the
  integration's job).

---

## Epic C — GH #77 sidecar `runtime_metadata` → CloudEvent enrichment

**Problem.** SDK ships the 12-field `decision_context_json` dict via
`client.request_decision(decision_context_json=...)` which is folded
into `DecisionRequest.inputs.runtime_metadata` (Struct). The sidecar
already extracts `prompt_hash` (`decision/transaction.rs::extract_
enrichment`) into the audit CloudEvent `data` payload for Cost
Advisor — but the other 11 LiteLLM fields (`integration`,
`litellm_call_id`, `model`, `pricing_version`,
`price_snapshot_hash_hex`, `fx_rate_version`,
`unit_conversion_version`, `call_type`, `stream`, `mode`, `team_id`)
are silently dropped. Operators querying `canonical_events.payload_
json->'data'->>'litellm_call_id'` see NULL → Q2/Q3 in ACCEPTANCE §5.1
collapse to 0 rows. Architectural call: **expand
`extract_enrichment` to capture a `spendguard.*` namespaced subset**
of `runtime_metadata`, persist into the existing `payload.data` JSON
under a new `"spendguard"` sub-object. **No schema change to
`canonical_events`**, no new column, no JSONB-path index (yet) —
forensics use existing JSONB query plans. Backward compat = trivially
empty subobject when SDK doesn't send.

### Surface contract

- **Sidecar Rust extraction**:
  - File: `services/sidecar/src/decision/transaction.rs`
  - Function: extend `extract_enrichment` (currently lines 97-138)
  - New struct field on `AuditEnrichment`: `pub spendguard_context:
    serde_json::Value` (defaults to `Value::Null` when not provided
    by SDK).
  - Logic: iterate `runtime_metadata.fields`, copy any key matching
    the `LITELLM_ENRICHMENT_KEYS` allowlist (12 fields above, plus
    `prompt_hash` for completeness) into a fresh `serde_json::Map`,
    coerce each `prost_types::Value::Kind` to its JSON form
    (StringValue → string, BoolValue → bool, NumberValue → number;
    NullValue → null; StructValue/ListValue → skip with WARN log —
    not part of v1 schema). The allowlist is the gate (sidecar
    refuses to copy unknown keys → fail-closed against operator-side
    SDK injecting arbitrary PII into audit chain).
- **CloudEvent emission**:
  - Files: `services/sidecar/src/decision/transaction.rs` —
    `build_audit_decision_cloudevent` (CONTINUE path, line 460) and
    the inline `payload = serde_json::json!({...})` in
    `run_record_denied_decision` (DENY path, line 565).
  - Change: add one key to each `payload` JSON literal: `"spendguard":
    enrichment.spendguard_context`. Goes inside the same `data` blob
    that already carries `snapshot_hash`, `matched_rules`,
    `agent_id`, etc. No envelope changes. Signing happens AFTER
    enrichment merges (existing `sign_cloudevent_in_place` call —
    already correct, just bigger payload).
- **Backward compat**:
  - Sidecar v1.0 wire: `runtime_metadata` field is OPTIONAL on
    `DecisionRequest`. When absent → `spendguard_context = null` →
    `payload.spendguard = null` → JSONB queries returning `NULL` on
    `->'spendguard'->>'litellm_call_id'` work without
    `coalesce()`. **Pre-existing canonical_events rows are
    unaffected** (never mutated — DESIGN NG2).
  - canonical_ingest **needs no change**. It already stores
    `payload_json` opaquely. The classify pipeline does not key off
    `spendguard.*` today; if Cost Advisor later does, it adds keys
    incrementally.
- **Canonical_ingest query convention** (lock):
  - **No new column.** Use JSONB path: `payload_json->'data'->
    'spendguard'->>'litellm_call_id'`. Justification: 4-step demo +
    cost-advisor queries run at hundreds of rows/sec, not millions
    — JSONB `->>` is fast enough. A new column would require a
    migration, double-write window, and backfill plan we don't yet
    need.
  - **Existing field migration**: `prompt_hash`, `agent_id`,
    `model_family` STAY at `payload_json->'data'->>'prompt_hash'`
    (no move under `spendguard` — Cost Advisor already queries the
    old path and Phase 5 GA rows depend on it). The `spendguard`
    sub-object is **additive** for LiteLLM-specific keys only.
  - ACCEPTANCE.md §5.1 line 173 query updates from
    `payload_json->'data'->>'integration'` (current, broken) to
    `payload_json->'data'->'spendguard'->>'integration'` (new path).
    Q2 and Q3 follow same rewrite.

### Failure modes

| Failure | User-facing |
|---|---|
| SDK doesn't send `decision_context_json` (Path A, older adapters) | `payload.spendguard = null`; Q2 returns 0 LiteLLM rows (correct — these aren't LiteLLM calls); other integrations unaffected |
| `runtime_metadata.fields` contains a key NOT on `LITELLM_ENRICHMENT_KEYS` | Silently dropped; WARN log once-per-process at sidecar (rate-limited) — prevents SDK from sneaking PII into audit chain |
| `runtime_metadata.fields[key].kind` is Struct/List | Skipped, WARN logged with key name; CloudEvent still emits with that key absent |
| canonical_ingest receives the bigger payload | Already stores opaquely as JSONB; no-op |
| CloudEvent signing | Happens AFTER payload assembly (unchanged ordering) — signature covers enrichment |

### Slice breakdown (≤300 LOC each)

| Slice | LOC | Files |
|---|---|---|
| **C1** — Extract + plumb (CONTINUE path only) | 200 | `services/sidecar/src/decision/transaction.rs` (+90: extend `AuditEnrichment` struct, `extract_enrichment` allowlist loop, `build_audit_decision_cloudevent` adds `"spendguard"` key); `services/sidecar/tests/integration/test_enrichment.rs` (+110: ALLOW path emits enrichment under correct JSONB path; missing-metadata path stays null; unknown-key gets dropped) |
| **C2** — DENY path parity + log gating | 180 | `transaction.rs` (+60: `run_record_denied_decision` payload literal); `services/sidecar/tests/integration/test_enrichment.rs` (+90: DENY path mirrors); WARN log throttle helper (+30) |
| **C3** — Acceptance SQL Q3 reanimation | 240 | `docs/specs/litellm-integration/ACCEPTANCE.md` (-15 / +25: rewrite Q2/Q3 to JSONB path); `examples/litellm-proxy-composite/Makefile` (+30: `demo-verify` calls); `services/canonical_ingest/tests/integration/test_litellm_query.rs` (+200: seed sample event, run Q3, assert orphan_outcomes == 0); GH issue #77 closeout note |

### Test plan

- **Tier 1 unit**: Rust unit tests in `transaction.rs` for the
  allowlist filter (12 keys pass; 13th drops; non-string kinds drop).
- **Tier 2 integration**: `services/sidecar/tests/integration/
  test_enrichment.rs` — sidecar UDS receives a `RequestDecision` with
  `runtime_metadata`, asserts emitted CloudEvent has
  `data.spendguard.litellm_call_id` set; back-compat assertion with
  no `runtime_metadata` provided.
- **Tier 3 demo**: `examples/litellm-proxy-composite/` `make demo-
  verify` runs Q1+Q2+Q3 against the live ledger after a 4-step run.
  Q3 returning 0 orphans = pass. This is the **ACCEPTANCE.md §5.1
  Q3 reanimation** — same SQL that was "BLOCKED on GH #77" goes
  green.

### Spec dependencies

- **References**: DESIGN §8.2a (12-field shape), ACCEPTANCE.md §5.1
  (Q1/Q2/Q3, especially the "Schema blocker (pivot R1 P0.1)" note
  flagging GH #77), `services/sidecar/src/decision/transaction.rs::
  extract_enrichment` doc-comment (already references
  `prompt_hash`).
- **Updates needed**:
  - DESIGN.md §8.2a — append "Sidecar persistence path: enrichment
    keys land under `canonical_events.payload_json->'data'->
    'spendguard'->*`. Allowlist enforced sidecar-side."
  - ACCEPTANCE.md §5.1 — rewrite Q2/Q3 JSONB path; remove the "BLOCKED
    on GH #77" note in Q3 expected output; update troubleshooting
    item 5 reference.
  - `transaction.rs::extract_enrichment` doc-comment — append: "12-key
    spendguard.* allowlist for LiteLLM forensics (GH #77)."
  - GH issue #77 — close with link to ADR-006 (new): "Sidecar
    enrichment uses JSONB sub-object, not new column. Reversible if
    query volume crosses 1M events/day."

### Out of scope

- New columns on `canonical_events` (deferred until query volume
  justifies it; current JSONB path scales to expected throughput).
- Backfill of historical rows (NG2 — never rewritten).
- GIN index on `payload_json->'data'->'spendguard'` (deferred; add
  when query plan shows seq-scan pain).
- Multi-integration enrichment unification (each integration owns its
  own allowlist; do not refactor `prompt_hash` into the new sub-
  object — pre-existing rows would break).
- Encryption-at-rest for enrichment fields (out of scope; same
  posture as the rest of `canonical_events.payload_json`).
- Auto-derivation of enrichment from `inputs.projected_unit` etc. —
  SDK is the source of truth; sidecar is a faithful extractor.

---

## Cross-epic sequencing

```
A1 (refactor) ──┐
                ├──> A2 (acompletion) ──> A3 (docs/smoke)
                │
B1 (topology) ──┤
                ├──> B2 (callback+app) ──┐
                │                        │
C1 (extract) ───┴──> C2 (DENY parity) ──┴──> C3 (Q3 + B3 verify together)
```

**Critical path**: B3 (`demo-verify`) cannot turn fully green until C3
ships, because Q3 reads the new JSONB path. Hold B3's `Makefile`
`demo-verify` target until C3 is merged; ship B1+B2 first with Q1
only.

**Reversibility**:
- Epic A: reversible — wrapper is opt-in; rolling back is deleting the
  symbol.
- Epic B: reversible — example directory is non-load-bearing.
- Epic C: **partially reversible**. The CloudEvent payload shape is
  load-bearing once emitted (signed rows are immutable per DESIGN
  NG2). New rows post-rollback would lack `spendguard` key, but old
  rows persist with it. Codex multi-agent review MANDATORY on C1
  before merge (per `feedback_codex_review.md` rule on irreversible
  audit-chain changes).
