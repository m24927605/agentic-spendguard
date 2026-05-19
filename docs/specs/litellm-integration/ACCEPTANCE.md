# LiteLLM Integration — ACCEPTANCE.md

> Status: Doc-first; lands before any implementation slice. Scope-lock document.
> Sibling docs: `DESIGN.md` (what we ship), `REVIEW_STANDARDS.md` (per-slice gate),
> `IMPLEMENTATION.md` (slice plan), `TEST_PLAN.md` (test pyramid + demo modes).
> Audience: Project owner (sign-off), implementers (target), reviewers (verification).

---

## 1. Acceptance philosophy

This doc defines the **scope-locked** answer to "what does *LiteLLM integration
is done* look like?" The doc owner is the project owner; once accepted, the
criteria here cannot be expanded mid-implementation without explicit owner
decision (`feedback_working_principles.md` Rule 3 — spec lock). Per-slice
progress lives in `REVIEW_STANDARDS.md` §6 (review logs) and `IMPLEMENTATION.md`
(slice plan). This doc is **the bar**: a slice that passes REVIEW_STANDARDS.md
still does not ship "the integration" — *all* criteria below must hold against
the merged diff before the project owner declares "ship it."

Criteria cite DESIGN.md goal IDs (G1–G5) and ADR IDs (ADR-001..005) by
reference so they are cross-checkable without re-reading.

---

## 2. Functional acceptance (maps to DESIGN.md G1–G5)

Every bullet is a capability the merged integration MUST demonstrate against
the live demo stack. Every criterion is objectively verifiable.

- **F1 (G1, drop-in).** Enable SpendGuard with at most three changes:
  install `spendguard-sdk[litellm]`, register the callback, supply a
  budget resolver. **Two registration forms exist depending on
  surface** (Round 3 P2.1 — pick one per surface, no ambiguity):
  - **Python (direct `acompletion()`):** `litellm.callbacks =
    [SpendGuardLiteLLMCallback(...)]` (list of instances).
  - **LiteLLM proxy (`proxy_config.yaml`):**
    `litellm_settings.callbacks: spendguard_litellm_proxy_callback.handler_instance`
    (string dotted-path to `handler_instance` in the
    operator-owned module — LiteLLM's required form for proxy-side
    callback loading).

  Verified by: the greenfield example in
  `docs/site/docs/integrations/litellm.md` (Slice 10 ships) runs
  end-to-end with no other code edits.
- **F2 (G2, fail-closed).** When the sidecar UDS socket does not exist,
  `litellm.acompletion(...)` raises `SidecarUnavailable` and
  the upstream provider is **never** contacted. The deny path for an
  over-budget call raises `DecisionDenied` (not `SpendGuardDenied` —
  P0.8 spec lock decision: the typed-deny exception is `DecisionDenied`
  everywhere, matching `client.py` + `agt.py` precedent). Verified by
  `DEMO_MODE=litellm_deny` with sidecar disabled (sub-step 2): (a) the
  **counting HTTP endpoint** counter stays at zero (P0.11: not
  `mock_response`), (b) typed exception raised, (c) no `canonical_events`
  row for the pre-call (sidecar never reached; SDK logs the attempt
  locally).
- **F3 (G3, in-process + proxy).** Both surfaces work against the same
  callback module: **(in-process)** `litellm.acompletion(...)` from a
  Python script with `litellm.callbacks = [callback]` set (always the
  list form — P2 consistency fix), verified by `litellm_real` step 1
  (Slice 6); **(proxy)** LiteLLM proxy started with the operator-owned
  `spendguard_litellm_proxy_callback.py` template (DESIGN §7.2 / built
  by Slice 8); a `POST /v1/chat/completions` with `team_id=...` gated
  by the matching SpendGuard budget, verified by `litellm_real` step 4
  (Slice 9). The proxy step is the **only** mode where
  `LiteLLM_SpendLogs` rows are written (DESIGN §8.3 — direct
  `acompletion()` mode does not populate that table).
- **F4 (G4, audit-chain coverage).** Every LiteLLM call that reaches
  the wire produces a `canonical_events` row whose
  `decision_context_json` carries the 12 fields specified in DESIGN.md
  §8.2a (including `integration='litellm'` and `litellm_call_id`).
  For the **proxy demo step only** (Slice 9 step 4), the row joins to
  `LiteLLM_SpendLogs.request_id` via `litellm_call_id`. Verified by
  the cross-join query (Q2) in §5.1 — zero unmatched rows on the
  proxy step's commit row. Q2 is **NOT** asserted on direct
  `acompletion()` steps (Slice 6 + Slice 9 steps 1+2+3) because
  LiteLLM only writes `LiteLLM_SpendLogs` in proxy mode (DESIGN.md
  §8.3 — P0.5 fix from Phase 0 review).
- **F5 (G5, demo is the gate).** `DEMO_MODE=litellm_real` and
  `DEMO_MODE=litellm_deny` both reach the `[demo] PASS` line on a clean
  `make demo-down && make demo-up` cycle. Per
  `feedback_demo_quality_gate.md`: Codex review pass is necessary, not
  sufficient. See §5.
- **F6 (DESIGN §5, retry — ADR-002).** With `num_retries=3`, provider
  500 on first two attempts + success on third: exactly one
  `INVOICE_COMMITTED` lands; the two failed attempts each produce a
  `RESERVATION_RELEASED`. No double-charge. **Verified by the
  integration test
  `tests/integration/test_litellm_failure_integration.py::test_num_retries_three_releases_two_keeps_third`**
  (NOT a demo step — Round 2 Phase 0 review P0.3 fix: no slice ships a
  retry-injection demo step in v1; the integration test is the
  authoritative gate).
- **F7 (DESIGN §5, streaming — ADR-003).** A streaming
  `litellm.acompletion(..., stream=True)` reserves at start (estimator
  worst-case), streams chunks to the caller, and commits at
  end-of-stream with the **reconciler-computed** real totals **when
  `response_obj.usage` is present at end-of-stream** (the normal
  case). Verified in the stream step: (a) chunks delivered ≥1,
  (b) one `INVOICE_COMMITTED` with `amount_atomic` **not equal** to
  the estimator amount (proves reconciler ran on actual usage),
  (c) reservation TTL ≥ stream duration.

  **Estimator-fallback degraded path** (Round 4 P0.4 clarification —
  IS NOT F7 acceptance): if a provider does not emit `.usage` at
  end-of-stream, the callback logs a WARNING and commits the
  estimator value (DESIGN §6 `ClaimReconciler` fallback). This path
  is **explicitly non-F7**: it cannot satisfy the F7 demo gate
  (which requires reconciled-not-estimated amounts) and is allowed
  only as a degraded production fallback. The Slice 9 STREAM step
  uses a fixture that DOES emit `.usage` so F7 is satisfied; the
  estimator-fallback path is exercised by unit test
  `test_streaming_response_missing_usage_falls_back_to_estimator`
  (TEST_PLAN §2.4) and is not part of any demo gate.

---

## 3. Non-functional acceptance

- **NF1 (latency).** Pre-call hook median ≤ 10 ms, p99 ≤ 25 ms (sidecar
  healthy, same pod, UDS) over 100 sequential `litellm.acompletion()` calls
  in the tier-2 integration test. CI captures the histogram artefact.
- **NF2 (concurrency, marquee adversarial scenario).** 50 parallel
  `asyncio.gather()` of `litellm.acompletion()` against a budget that
  allows exactly 25 produces **exactly 25 ALLOWs and 25 DENYs** — no
  over-commit, no double-allow under contention. Per
  `feedback_codex_review.md` adversarial mode + `project_phase1_ledger.md`
  single-writer-per-budget.
- **NF3 (memory).** Callback registration does not leak coroutines on
  `litellm.proxy.shutdown()`. Verified by a proxy spin-up/spin-down loop
  ×5 with `tracemalloc` snapshots; no monotonic growth in the callback
  module's allocation set.
- **NF4 (no global state).** Zero module-level mutable singletons in
  `spendguard.integrations.litellm` beyond the `_RUN_CONTEXT` ContextVar
  permitted by `agt.py` precedent. Other module-level mutable state is a
  P1 finding in the final Codex pass.
- **NF5 (sidecar back-pressure at commit boundary).** Under Shape B,
  the sidecar is contacted **only at pre-call and at end-of-stream**
  (DESIGN.md ADR-003). There is **no chunk-level sidecar interaction
  during the stream itself** — so "sidecar killed mid-stream" is
  silent from the SDK's perspective until end-of-stream commit. The
  acceptance criterion is therefore restated: if the sidecar is
  unreachable at the **commit boundary** (end-of-stream), the
  callback surfaces `SidecarUnavailable` from
  `async_log_success_event`; the reservation TTL-sweeps on the
  ledger side. Chunk-level sidecar interaction is deferred to v2
  with the chunk-commit work (ADR-003). Verified by
  `tests/integration/test_litellm_streaming_integration.py::test_streaming_sidecar_offline_at_commit_boundary`
  (Round 3 P0.8 fix — earlier NF5 was impossible under Shape B).

---

## 4. Security & audit-chain acceptance

- **S1 (correlation ID, MANDATORY).** Every `canonical_events` row from
  this integration carries the 12 `decision_context_json` fields
  specified in DESIGN.md §8.2a, including `integration='litellm'` and
  `litellm_call_id`. Verified by:
  `SELECT COUNT(*) FROM canonical_events WHERE decision_context_json->>'integration'='litellm' AND decision_context_json->>'litellm_call_id' IS NULL;`
  → expected `0`. Also verified row-by-row that all 12 fields are
  populated (P0.5 fix from Phase 0 review — `verify_step_litellm_real.sql`
  has the row-level predicate).
- **S2 (frozen pricing, MANDATORY).** Decision-context JSON includes the
  full frozen pricing tuple — `pricing_version`, `price_snapshot_hash_hex`,
  `fx_rate_version`, `unit_conversion_version` — same shape AGT and other
  integrations emit (`issue-59-approval-resume-frozen-pricing.md`).
  Verified by the analogous `IS NULL` predicate over all four columns.
- **S3 (no new secrets path).** The integration MUST NOT add a path that
  handles, logs, masks, or transports `OPENAI_API_KEY`, `ANTHROPIC_API_KEY`,
  or any other provider key. Provider keys remain LiteLLM's concern.
  Verified by `grep` over `sdk/python/src/spendguard/integrations/litellm.py`
  + operator template: zero matches for `OPENAI_API_KEY`,
  `ANTHROPIC_API_KEY`, `BEDROCK`, `GEMINI_API_KEY`, or
  `os.environ.get`/`os.getenv`+`*_API_KEY`. Final Codex pass C2 asks this
  question explicitly.
- **S4 (no log leakage of LLM payloads).** `messages`/`prompt`/response
  content from LiteLLM kwargs MUST NOT be written verbatim into
  `canonical_events.decision_context_json`. Only a blake2b 16-byte content
  hash and token-count metadata are committed. Verified by `grep` for
  unhashed `kwargs["messages"]` flows + sanity check
  `octet_length(decision_context_json::text) ≤ 4 KiB` per row.
- **S5 (SBOM, license).** The `spendguard-sdk[litellm]` extra pulls
  `litellm` and its deps. No new transitive brings in a license outside the
  existing allowlist (Apache-2.0, MIT, BSD-3-Clause, BSD-2-Clause, ISC,
  MPL-2.0). Verified by the project's existing SBOM-check job; exit 0 on
  the new extra.
- **S6 (fail-open dev escape hatch is loud).** Per ADR-004,
  `SPENDGUARD_LITELLM_FAIL_OPEN=1` is dev-only and MUST emit a
  `WARNING`-level log entry in **both** of these conditions
  (**per P1.3 fix from Phase 0 review** — single-WARNING-at-startup
  is insufficient):
  1. **At callback construction** — once, with message containing
     `SPENDGUARD_LITELLM_FAIL_OPEN=1`, regardless of whether any
     fail-open path is ever taken.
  2. **On every fail-open path taken at runtime** — each time a
     sidecar error is swallowed by the fail-open semantics, log a
     WARNING with the original exception repr.

  Env var is read **once** at construction; runtime mid-process changes
  have no effect. Verified by `tests/integration/test_litellm_precall_integration.py::test_fail_open_warning_loud`
  (caplog asserts both conditions) and by `grep`ing the
  demo log tail when the env var is set during demo bring-up.

---

## 5. Demo acceptance — the bar

Per `feedback_demo_quality_gate.md`: each service must really run the demo
before "done" can be declared. Two demo modes MUST pass on clean state.

### 5.1 `DEMO_MODE=litellm_real` (allow path + audit chain)

**Command (verbatim):**

```bash
cd /Users/michael.chen/products/agentic-spendguard/deploy/demo
make demo-down
DEMO_MODE=litellm_real make demo-up
```

**Stdout (in order — authoritative shape; REVIEW_STANDARDS.md §7.3
and TEST_PLAN.md §3.1 mirror this verbatim):**

```
[demo] DEMO_MODE=litellm_real → litellm proxy + sidecar + ledger ready
[demo] handshake ok session_id=...
[demo] step 1: ALLOW — litellm.acompletion → DECISION_ALLOWED → INVOICE_COMMITTED
[demo] step 2: DENY — over-budget → DecisionDenied raised
[demo] step 3: STREAM — sse complete → INVOICE_COMMITTED with real usage
[demo] step 4: PROXY — POST /v1/chat/completions team=t1 → INVOICE_COMMITTED
[demo] PASS — all 4 steps OK
```

Absent the literal `PASS — all 4 steps OK` line → demo gate **fails**
regardless of exit code.

**Ledger queries that MUST hold (templated for `$session_id`; event
types use the **full** canonical names per DESIGN.md §8.2 — no
abbreviations):**

```sql
-- Q1: event counts qualified by event_type (P0.6 fix from Phase 0 review).
-- Expected for the 4-step litellm_real demo:
--   DECISION_REQUESTED: 4 (one per step)
--   DECISION_ALLOWED:   3 (steps 1, 3, 4 — step 2 is the over-budget DENY)
--   DECISION_DENIED:    1 (step 2 only)
--   RESERVATION_CREATED:3 (one per allowed step)
--   INVOICE_COMMITTED:  3 (steps 1, 3, 4)
--   RESERVATION_RELEASED: 0 (no failures in happy path)
SELECT event_type, COUNT(*) FROM canonical_events
WHERE session_id = $session_id
  AND decision_context_json->>'integration' = 'litellm'
GROUP BY event_type;

-- Q2: cross-join. ONLY applied to the proxy step (step 4) because
-- LiteLLM_SpendLogs is only populated in proxy mode (DESIGN.md §8.3).
-- INNER JOIN (Round 4 P0.3 fix — earlier LEFT JOIN allowed a row
-- with NULL `ls.request_id` to satisfy ≥1 even when the join key
-- mismatched). Expected: ≥1 row.
SELECT ce.audit_decision_event_id, ls.request_id
FROM canonical_events ce
INNER JOIN "LiteLLM_SpendLogs" ls
  ON ls.request_id = ce.decision_context_json->>'litellm_call_id'
WHERE ce.event_type = 'INVOICE_COMMITTED'
  AND ce.decision_context_json->>'integration' = 'litellm'
  AND ce.decision_context_json->>'mode' = 'proxy';

-- Q2b: unmatched-count safety net. Expected: 0 (every proxy commit
-- must join). Catches the silent mismatch class even if a future
-- query refactor reverts Q2 to LEFT JOIN.
SELECT COUNT(*) FROM canonical_events ce
LEFT JOIN "LiteLLM_SpendLogs" ls
  ON ls.request_id = ce.decision_context_json->>'litellm_call_id'
WHERE ce.event_type = 'INVOICE_COMMITTED'
  AND ce.decision_context_json->>'integration' = 'litellm'
  AND ce.decision_context_json->>'mode' = 'proxy'
  AND ls.request_id IS NULL;

-- Q3: hash chain intact. canonical_events is a tenant-scoped chain
-- (not session-scoped), so a session's first event normally points
-- to a prior event in the same tenant. Q3 asserts that EVERY event
-- in this session has a valid prev pointer (event_id + hash matches
-- a real prior row in the same tenant). Expected: 0 broken pointers.
-- (Round 2 P1.6 fix — was asserting 1 genesis row, wrong for
-- tenant-scoped chain.)
SELECT COUNT(*) FROM canonical_events ce1
WHERE session_id = $session_id
  AND ce1.prev_event_id IS NOT NULL
  AND NOT EXISTS (
    SELECT 1 FROM canonical_events ce0
    WHERE ce0.tenant_id = ce1.tenant_id
      AND ce0.event_id = ce1.prev_event_id
      AND ce0.event_hash = ce1.prev_event_hash);
```

**Important: Q1/Q2/Q3 counts assume the 4-step demo. Slice 6 lands
steps 1+2 only; partial demo counts are: REQUESTED=2, ALLOWED=1,
DENIED=1, RESERVATION_CREATED=1, INVOICE_COMMITTED=1. Slice 9 adds
steps 3+4 to reach the full counts above.**

**Failure-mode investigation guide:**

1. **Boot failure (never reaches step 1).** `make demo-logs`: proxy startup
   errors → check `proxy_config.yaml` vs DESIGN §7.2 + `OPENAI_API_KEY`.
   Sidecar UDS missing → check `spendguard-adapter` container (likely
   Postgres; cite `project_known_demo_flakes.md` if seen). Handshake
   timeout → identical to AGT (`run_demo.py::run_agt_composite_mode`).
2. **Step 1: ALLOWED but no INVOICE_COMMITTED.** `async_log_success_event`
   never fired. Did the provider return? Is `litellm.callbacks` a **list**?
   Re-run with `LITELLM_LOG=DEBUG`.
3. **Step 3 (stream): commit `amount_atomic` matches estimator, not actual.**
   Reconciler does not consume `response_obj.usage` correctly — the wire-time
   bug class `feedback_demo_quality_gate.md` warns about (Pydantic-AI
   duck-typing precedent).
4. **Step 4 (proxy): DENIED on a budget that should allow.** Resolver did
   not see `team_id`. Per ADR-001: pull from `user_api_key_dict`, fall
   back to `kwargs["metadata"]["spendguard_budget_id"]`.
5. **Hash-chain query (Q3) returns >1.** Audit corruption. **HARD BLOCK.**
   P0 incident; do not declare PASS even if `[demo] PASS` printed.

### 5.2 `DEMO_MODE=litellm_deny` (fail-closed verification)

**Command (verbatim):**

```bash
cd /Users/michael.chen/products/agentic-spendguard/deploy/demo
make demo-down
DEMO_MODE=litellm_deny make demo-up
```

**Stdout (authoritative shape — TEST_PLAN.md §3.2 and REVIEW_STANDARDS.md
§7 mirror this verbatim):**

```
[demo] DEMO_MODE=litellm_deny → fail-closed scenarios
[demo] handshake ok session_id=...
[demo] step 1: budget exhausted — DecisionDenied raised (provider untouched)
[demo] step 2: sidecar offline — SidecarUnavailable raised (provider untouched)
[demo] step 3: resolver returns None + no default budget — SpendGuardConfigError raised
[demo] PASS — all 3 deny paths OK
```

**Ledger / fixture assertions:**

- **Step 2 (sidecar offline):**
  `SELECT COUNT(*) FROM canonical_events WHERE session_id=$step2_session_id;`
  → expected `0` (sidecar was never reached).
- **Step 1 (budget exhausted):** event types for this session are
  `{DECISION_REQUESTED, DECISION_DENIED}` only — no
  `RESERVATION_CREATED`, no `INVOICE_COMMITTED`.

**Provider-untouched assertion (the marquee G2 demonstration).**
The **counting HTTP endpoint** (in-process `aiohttp` mock server,
NOT `litellm.acompletion(mock_response=...)` — P0.11 fix) exposes
`GET /__counters` returning `{"requests_received": N}`. **Counter
semantics — single unified rule** (Round 4 P0.5 fix; aligns
ACCEPTANCE §5.2 / TEST_PLAN §4.3 / IMPLEMENTATION Slice 7):

1. **Positive control first.** Before each `litellm_deny` sub-step,
   the demo fires ONE deliberate ALLOW call through the same
   listener (via a fresh budget that permits it); snapshots the
   counter; asserts the counter incremented by exactly 1. This
   proves the listener is wired (no vacuous-zero risk).
2. **Sub-step delta == 0.** For each fail-closed sub-step,
   snapshot the counter immediately before the call, perform the
   deny scenario, snapshot again; **assert the delta is 0**. This
   is the wire-time fail-closed proof.

The "absolute zero across all 3 sub-steps" version is REPLACED by
this delta-with-positive-control rule because absolute zero
conflicted with the positive-control wiring check.

**Failure-mode investigation guide:**

1. **Step 2: provider counter > 0.** **HARD BLOCK.** Fail-closed inverted.
   P0; immediate fix; do not declare PASS.
2. **Step 3: no exception, call attempted with no budget.** Resolver
   fallback broken. Per ADR-001 (Round 2 P1.4 fix — env-var default
   was REMOVED in P0.10): the only valid fallback inside the
   resolver is `metadata["spendguard_budget_id"]`. If THAT is also
   missing/None, the resolver must return `None` →
   `SpendGuardConfigError`. There is NO `SPENDGUARD_LITELLM_DEFAULT_BUDGET_ID`
   env var anymore.
3. **Flake between PASS/FAIL across runs.** Race in prior-run cleanup.
   `make demo-down -v` then re-run. Treat as known flake only if cited in
   `project_known_demo_flakes.md`; otherwise P0.

---

## 6. Codex review acceptance

- **C1 (per-slice).** Every slice in `IMPLEMENTATION.md` satisfies
  `REVIEW_STANDARDS.md` §3.4 stopping rule (N≥2 rounds, zero unresolved
  P0, zero new P0 in last round, zero new P1 in critical path). Tracked in
  per-slice logs at `docs/specs/litellm-integration/review-logs/slice-NN.md`.
- **C2 (final whole-integration pass).** After the last slice merges, a
  final adversarial Codex pass runs against
  `base-before-slice-01..HEAD-of-slice-final`. Binary criterion:
  **zero new P0 findings on the final pass.** P1 findings are permitted
  only if (a) every one is logged in `review-logs/final-pass.md` with
  severity + status, AND (b) each unresolved P1 is either explicitly
  deferred-with-issue + approved by the project owner, or confirmed as a
  repeat of a prior-pass finding the slice owner already
  disputed-with-reason. New P1 surfacing only in the final pass signals
  cross-slice interaction bugs; resolve before ship.

The final-pass prompt uses REVIEW_STANDARDS.md §4.1 adapted for
whole-integration scope (substitute DESIGN.md §1–13 for the slice mini-spec).

---

## 7. Documentation acceptance

- **D1.** `docs/site/docs/integrations/litellm.md` exists and follows the
  `docs/site/docs/integrations/agt.md` shape, with: "Why you'd want this"
  intro; **three paths for existing LiteLLM users** — Path A (primary
  callback), Path B (proxy `proxy_config.yaml` + operator-owned callback
  module), Path C (Shape A fallback: `litellm.api_base` → SpendGuard
  egress proxy, recipe only, no new SpendGuard code); **prerequisites**
  table (sidecar deployed, Postgres reachable, tenant + budget seeded,
  contract bundle published, SDK installed); **operational gotchas**
  (reservation TTL for streams ADR-003, retry over-reservation window
  ADR-002, proxy-mode `team_id` resolution ADR-001, fail-open dev caveat
  ADR-004); **quickest validation** (the §5.1 command + expected output
  verbatim); **greenfield example** (~50–80 lines self-contained Python);
  **Related** footer linking the four sibling integration pages.
- **D2.** Every existing integration page (`agt.md`, `langchain.md`,
  `openai-agents.md`, `pydantic-ai.md`) `Related` footer is updated to
  include the LiteLLM page. Verified by `grep -l "litellm.md"
  docs/site/docs/integrations/*.md` returning all four sibling files.
- **D3.** `README.md` (root) + `docs/site/docs/quickstart.md` mention
  LiteLLM in their integration list. Single line each.
- **D4.** `docs/specs/litellm-integration/{DESIGN,REVIEW_STANDARDS,
  IMPLEMENTATION,TEST_PLAN,ACCEPTANCE}.md` all present in the merged tree;
  spec set is internally consistent (final Codex pass C2 verifies this).

---

## 8. What is NOT acceptance (explicit non-criteria)

Spec lock requires the negative list to be as crisp as the positive list:

- **N1.** No 100% line coverage requirement. Targets live in TEST_PLAN.md.
- **N2.** No SpendGuard fork of LiteLLM (DESIGN NG3). Pip-installable only.
- **N3.** No Cost Advisor write-path integration in v1 (DESIGN §10). Cost
  Advisor may observe LiteLLM patterns; v2 territory for patching.
- **N4.** No real third-party provider calls required in CI. Recorded
  responses + mock provider sufficient. Real keys are operator-side only.
- **N5.** No sync `litellm.completion()` support in v1 (ADR-005 +
  DESIGN.md NG6). Sync users documented as Shape A or async migration.
  This negative criterion is reinforced from DESIGN.md G3 (which now
  explicitly says "async only") — P0.1 fix from Phase 0 review.
- **N6.** No streaming chunk-level commit (ADR-003). End-of-stream is the
  v1 commit point.
- **N7.** No composite Shape C (DESIGN §3.3). Deferred to v2.
- **N8.** No exhaustive per-provider demo. Shape B's provider-agnosticism
  is a design property verified by source reading, not by N demo modes.
- **N9.** No `LiteLLM_SpendLogs` rewriting (DESIGN NG2 + §8.3). LiteLLM
  keeps its own logs; SpendGuard writes alongside.
- **N10.** No `strong_global` / `eventual` consistency advertising. Only
  `single_writer_per_budget` per `project_phase1_ledger.md`. Cross-region
  linearizability is Phase 3+ territory; not tested here.

---

## 9. Sign-off process

Sequential; each gate must pass before the next is approached:

1. **Per-slice Codex (C1).** Each slice's review log shows
   STOPPING-RULE-MET (REVIEW_STANDARDS.md §3.4); log committed in the same
   PR as the slice code (H7); human reviewer verifies H1–H7 per
   REVIEW_STANDARDS.md §8.
2. **Per-slice demo gate** (Round 3 P1.5 clarification). Each slice's
   demo mode (REVIEW_STANDARDS.md §7.1) reaches `[demo] PASS` with
   log-tail evidence in §7.4 of the slice log. Slice→demo mapping:
   - Slices 1–5 (SDK-only): `DEMO_MODE=decision` regression — assert
     existing `[demo] PASS — handshake + decision + confirm OK`.
   - Slice 6: `DEMO_MODE=litellm_real` partial (steps 1+2; see
     REVIEW_STANDARDS §7.3 "Partial-completion note").
   - Slice 7: `DEMO_MODE=litellm_deny` (full 3-step).
   - Slice 8: `DEMO_MODE=decision` regression + proxy template
     YAML-parse smoke.
   - Slice 9: `DEMO_MODE=litellm_real` complete (all 4 steps; final
     `PASS — all 4 steps OK` line).
   - Slice 10: `DEMO_MODE=litellm_real` + `DEMO_MODE=litellm_deny`
     full sweep (regression against shipped integration).
3. **Documentation review (D1–D4).** Project owner (or designated doc
   reviewer) reads `docs/site/docs/integrations/litellm.md` against the
   `agt.md` template. Spot-check: copy-paste greenfield example into a
   clean venv with `pip install --pre 'spendguard-sdk[litellm]'` + demo
   sidecar; example prints the expected line.
4. **Final whole-integration Codex pass (C2).** Adversarial prompt against
   `base-before-slice-01..HEAD-of-slice-final`; criterion zero new P0;
   logged at `docs/specs/litellm-integration/review-logs/final-pass.md`.
5. **Demo acceptance — final clean cycle.** From clean checkout (or `git
   clean -xfd && make demo-down -v`), run **both** demos back-to-back:
   `litellm_real` PASS → `make demo-down` → `litellm_deny` PASS. Both
   `[demo] PASS` lines must land in the same session; a flake on either
   side blocks declaration.
6. **User explicit "ship."** Project owner reviews ACCEPTANCE.md
   side-by-side with the merged diff + the two demo runs and declares
   "ship." Per `feedback_working_principles.md` Rule 2: a single line in
   the owner's notes; artefacts are the merged code, review logs, demo
   logs. No ceremonial PR comment required.

If any gate fails: implementer (a) fixes within scope and re-attempts, or
(b) escalates per REVIEW_STANDARDS.md §3.5 with a concrete recommendation.

---

## 10. Rollback plan

If a critical bug surfaces post-merge:

- **R1 (user-side, immediate).** `pip uninstall spendguard-sdk` rolls
  back users on the LiteLLM extra only. Users on other SDK surfaces revert
  two lines: `litellm.callbacks = []` + remove
  `spendguard.integrations.litellm` imports. LiteLLM behaviour reverts to
  pre-integration baseline; LiteLLM's `LiteLLM_BudgetTable` keeps working
  (DESIGN NG2 — never rewritten). No data migration; `canonical_events`
  rows are immutable and queryable.
- **R2 (project-side).** Yank `spendguard.integrations.litellm` from the
  next minor SDK release; mark `spendguard-sdk[litellm]` as **broken** in
  PyPI release notes + docs. Existing installs keep working; new users
  blocked until fix lands. Same shape as `auto-instrument-egress-proxy`
  deprecations.
- **R3 (Shape A as fallback).** Per DESIGN §3.1, Shape A (LiteLLM →
  SpendGuard egress proxy chain) needs **zero new SpendGuard code**. Point
  `litellm.api_base` at the existing egress proxy → continue with degraded
  provider coverage (OpenAI / Responses API only). The docs page Path C
  carries the recipe.
- **R4 (data integrity).** Per `project_phase1_ledger.md` + DESIGN §8.2:
  rollback **never** mutates `canonical_events`. In-flight reservations
  release on TTL sweep (DESIGN §5, default 300s). No manual ledger surgery;
  hash chain stays intact across rollback boundaries.
- **R5 (communication).** Per `feedback_working_principles.md` Rule 2:
  single line in owner's incident notes + one-paragraph release-note entry
  pointing users at R1/R3. No long retro until the fix ships.

Rollback bar is **no permanent damage** — neither LiteLLM users' calls
nor SpendGuard's ledger nor the audit chain are degraded by reverting.
That property is owed to the integration being a **pip extra + callback
registration**, not a LiteLLM code mutation — the cost we paid in DESIGN
§3 (Shape B over Shape C) buys this rollback property for free.

---

## 11. References

- `DESIGN.md` — G1–G5 §2.1; failure modes §5; audit chain §8; demo gate
  §11; ADR-001..005 §9.
- `REVIEW_STANDARDS.md` — §3 Codex loop, §7 demo gate, §8 human reviewer
  check order.
- `IMPLEMENTATION.md` — slice plan (feeds per-slice acceptance).
- `TEST_PLAN.md` — test pyramid + demo-mode invariants this doc cites.
- `feedback_demo_quality_gate.md` — Codex ✅ 不夠;每個 service 必須真跑 demo.
- `feedback_working_principles.md` — Rule 3 spec lock; Rule 2 短 reply 長 doc.
- `project_phase1_ledger.md` — `single_writer_per_budget` only.
- `sdk/python/src/spendguard/integrations/agt.py` — shape we mimic.
- `deploy/demo/demo/run_demo.py::run_agt_composite_mode` — demo precedent.
- `docs/site/docs/integrations/agt.md` — shape template for D1.
