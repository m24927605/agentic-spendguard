# D23 — BeeAI Framework `Emitter` adapter — Review standards (R1-R5)

The `superpowers:code-reviewer` skill applies this checklist on every slice diff. Findings against any item below are blocking unless the reviewer explicitly downgrades with rationale recorded in the round summary. R5 → Staff+ panel arbitration if any item stays open.

## 1. Security

- **S1**: Reserve must fire **before** any control flow that could trigger a provider HTTP. Verify by tracing the `*.start` handler: `await client.request_decision(...)` is the first awaited call, and any path that returns from the handler without raising must have `inflight.put(...)` called (so the eventual commit lands). Demo `e8` is the cross-check.
- **S2**: `DecisionDenied` raised from `request_decision` propagates out of the BeeAI handler — the handler must NOT swallow or downgrade it. Verify by reading every `try` / `except` in the start handler.
- **S3**: No PII leakage in log messages. Inflight warning logs the call key only (BeeAI EventMeta path is a hierarchical name, not user content). Test by mocking a `BeeAiStartEvent` with `input=["SECRET"]` and asserting the warning string does not contain `"SECRET"`.
- **S4**: Subscriber is single-tenant — `client` is captured by closure, no per-event tenant override. If the caller multiplexes tenants, they install one subscriber per tenant. Documented in the docs page.
- **S5**: Default `claim_estimator` reuses the centralised `_default_estimator` model-dispatch logic. No new heuristic. Direct grep: no fresh `chars // 4` or similar inline.

## 2. Determinism / idempotency

- **D1**: Idempotency key derivation uses the same `derive_idempotency_key` helper as the LangChain adapter with `trigger="LLM_CALL_PRE"`. Diff: identical kwarg shape, identical scope.
- **D2**: `llm_call_id` and `decision_id` derived from the call signature via `derive_uuid_from_signature` with distinct `scope=` strings. Two different scopes means two different UUIDs even from the same signature.
- **D3**: Stable per-call key is the EventMeta path with the trailing `.start|.success|.error` stripped — and **only** that. No timestamps, no random suffixes, no `id(self)`-style ephemerals.
- **D4**: Re-running the same agent under the same run_context with the same input produces the same `llm_call_id` and `decision_id`. Tested via `test_beeai_default_e2e.py::t21` indirectly (two calls → two distinct IDs from two distinct paths) and a fresh test for replay-determinism if not already covered.

## 3. Lifecycle correctness

- **L1**: Every `start` that returns `outcome.reservation_ids` non-empty has a matching `inflight.put`. Every `success` / `error` that finds an entry calls `emit_llm_call_post` exactly once. No double-commit possible.
- **L2**: `success` / `error` without a matching `start` is a no-op — not an exception. Backed by t14.
- **L3**: `unsubscribe()` returned by `subscribe_spendguard` actually unhooks. After `unsubscribe()`, no more events flow through SpendGuard.
- **L4**: Inflight map FIFO eviction warns exactly once (idempotent boolean flag). Eviction does NOT release the reservation — sidecar TTL sweep handles it. This is documented in the warning text.
- **L5**: DEGRADE handling — mutation patch NOT applied (parity with LangChain). The handler logs at WARNING level and records inflight so the eventual commit lands.

## 4. Run-context contract

- **R1**: Re-uses the LangChain `_RUN_CONTEXT` ContextVar — no fresh ContextVar in `beeai.py`. Grep: `_RUN_CONTEXT` only appears in `langchain.py`; `beeai.py` imports `current_run_context` from there.
- **R2**: Calling the BeeAI handler outside `run_context(...)` raises a `RuntimeError` with an actionable message that mentions `run_context(RunContext(run_id=...))` and includes a snippet (mirrors LangChain message style).
- **R3**: `run_context` is re-exported from `spendguard.integrations.beeai.__all__` so users can `from spendguard.integrations.beeai import run_context` without crossing to the LangChain module.

## 5. Public surface stability

- **P1**: `subscribe_spendguard` signature change is a breaking change. Every kwarg added after v0.1.0 must default to a non-None value or be `Optional[...]` with a default. Reviewer flags any kwarg added without a default.
- **P2**: `BeeAiStartEvent` is frozen / slots-True (matches `RunContext`). Reviewer rejects field additions without bumping the docstring `versionchanged::` block.
- **P3**: `__all__` lists exactly: `BeeAiStartEvent`, `ClaimEstimator`, `CallSignatureFn`, `RunContext`, `run_context`, `subscribe_spendguard`. No private symbols leak.

## 6. Demo correctness

- **DM1**: Both demo modes (`agent_real_beeai`, `agent_real_beeai_deny`) call `subscribe_spendguard` exactly once and `unsubscribe()` in a `finally:` block. Grep the demo driver.
- **DM2**: The ALLOW demo asserts `inflight_map_size == 0` after `agent.run` returns. The DENY demo asserts `agent.run` raised `DecisionDenied` AND the upstream stub container received zero `/v1/chat/completions` requests.
- **DM3**: `verify_beeai.sql` is parameterised by `:run_id` only — no other tenant / budget params (those come from compose env). The SQL file is checked into the repo, not generated at demo time.
- **DM4**: `Makefile` arms re-use the existing `demo-base-up` / `demo-run` targets — no fresh container orchestration logic in the BeeAI arm.

## 7. Docs

- **DC1**: The integration page contains all four required sections: install, quickstart, claim-estimator note, troubleshooting (ImportError + missing run_context + DEGRADE behaviour).
- **DC2**: The quickstart snippet is byte-identical to `examples/beeai/quickstart.py` (when that file lands in slice 4). Reviewer grep.
- **DC3**: README adapter table entry has columns: framework name, language, install command, link to `docs/site/docs/integrations/beeai.md`. Same row shape as the LangChain row.
- **DC4**: No marketing language. Reviewer flags "industry-leading", "seamless", "magical", etc.

## 8. Anti-scope gates

Reviewer rejects (returns finding) any PR in D23 slices that:

- Adds TS adapter scaffolding (defer to a separate deliverable).
- Adds `tool.*` event subscription (out of scope per design.md §3).
- Adds streaming token-by-token gating (out of scope).
- Modifies `langchain.py` for anything other than exporting symbols already imported by `beeai.py` (e.g. adding a function that BeeAI needs — fine; refactoring LangChain logic — out of scope, file a separate slice).
- Modifies `spendguard.integrations.agt` (tool-call gating territory; D23 is LLM-call only).

## 9. R5 panel triggers

Escalate to Staff+ panel arbitration if any of these emerge:

- Reviewer finds that `Emitter.match` does not return an unsubscribe callable in the installed `beeai-framework` version (API drift). Panel decides between version pin downgrade vs forking the subscription mechanism.
- Reviewer finds a race in inflight map under concurrent `agent.run` invocations on the same event loop. Panel decides between asyncio.Lock vs documented single-call constraint.
- Reviewer finds the demo provider HTTP fires on DENY (catastrophic safety bug). Panel decides whether to ship the deliverable at all or block on a BeeAI upstream fix.

## 10. Self-check at slice close

Implementer self-attests before requesting review:

- [ ] All acceptance gates in `acceptance.md` for this slice run green locally.
- [ ] Lint + type-check clean.
- [ ] No `git diff` outside the slice's declared file list.
- [ ] No emoji in source or docs (project convention).
- [ ] No marketing language in docs.
- [ ] No deferred TODO markers without an issue number.
