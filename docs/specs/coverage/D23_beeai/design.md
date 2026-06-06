# D23 — BeeAI Framework `Emitter` adapter (Python)

**Status:** Spec — Tier 3 (build plan `framework-coverage-build-plan-2026-06.md` §2.3).
**Parent strategy:** [`framework-coverage-2026-06.md`](../../../strategy/framework-coverage-2026-06.md), Pattern 1 (framework event-bus middleware).
**Owner sub-agent:** AI Engineer.
**Python analog:** `sdk/python/src/spendguard/integrations/langchain.py` (378 LOC, shipped). D23 reuses its run-context, idempotency-key derivation and POST usage extraction. Integration shape is event subscription, not `BaseChatModel` wrapping.

## 1. Problem

BeeAI Framework (IBM Research + Linux Foundation, Apache-2.0, ~3k stars on `i-am-bee/beeai-framework`) is the leading IBM-aligned agent stack. `beeai-framework>=0.3` ships `ReActAgent`, `Workflow`, and a global `Emitter` that publishes `start` / `success` / `error` / `partialUpdate` / `newToken` per LLM call, tool call, and agent step. Today BeeAI users have **zero** SpendGuard coverage — they'd hand-write `Emitter.on(...)` and re-derive idempotency / prompt-hash / commit semantics, breaking audit-chain determinism.

D23 ships the Python adapter so any BeeAI agent gets the same pre-call reservation + post-call commit lifecycle the LangChain adapter provides.

## 2. Goals

1. New `spendguard.integrations.beeai` module exposing `subscribe_spendguard(agent, client, *, budget_id, window_instance_id, unit, pricing, claim_estimator=None, call_signature_fn=None, route="llm.call")` — a one-shot registration helper.
2. PyPI extras: `spendguard-sdk[beeai]` pinning `beeai-framework>=0.3,<1.0`. ImportError contract matches the LangChain adapter.
3. Event contract: `*.start` → `request_decision` (reserve), `*.success` → `emit_llm_call_post(outcome="SUCCESS")`, `*.error` → `emit_llm_call_post(outcome="PROVIDER_ERROR")` (release residual).
4. Per-call correlation via a process-local FIFO `Mapping[stable_path, _Inflight]`, bounded at 10k with one-shot eviction warning.
5. Demo modes `agent_real_beeai` + `agent_real_beeai_deny`: drive `ReActAgent` + `OpenAIChatModel` (or BeeAI `DummyChatModel`) against the sidecar UDS. ALLOW path verifies a reserve fires before provider HTTP; DENY path verifies `DecisionDenied` halts the agent and the provider is never contacted.
6. Public docs page `docs/site/docs/integrations/beeai.md` + one README adapter-table row.

## 3. Non-goals

- TypeScript BeeAI adapter (deferred; Tier 3 is Python-only per build plan §2.3).
- `Workflow.run` per-step gating beyond what `subscribe_spendguard` covers transitively via child emitters.
- `newToken` / `partialUpdate` mid-stream gating (pre + post only, LangChain parity).
- Tool-call mid-loop gating — v1 subscribes to `llm.*` only; tool gating is `integrations.agt` territory.
- Approval-resume UI helper.

## 4. Architecture

```
BeeAI ReActAgent.run(prompt)
  └→ child Emitter.namespace = ["agent","react","llm"]
        ├→ emit("start", {input, modelId, …}, meta) ──► SpendGuard subscriber
        │                                                  └→ client.request_decision(...)
        │                                                       └→ ALLOW  → no-op
        │                                                       └→ DENY   → raise DecisionDenied
        │                                                                     (propagates → agent halts; provider HTTP never fires)
        ├→ ChatModel.create(...)                  ◄── runs only on ALLOW
        ├→ emit("success", {output, usage}, meta) ──► subscriber → emit_llm_call_post(SUCCESS, usage.total_tokens)
        └→ on error: emit("error", {err}, meta)   ──► subscriber → emit_llm_call_post(PROVIDER_ERROR, 0)
```

Registration: a single `agent.emitter.match(predicate, handler)` call. Predicate accepts events whose `name in {"start","success","error"}` AND whose path contains an `llm` segment. The handler dispatches on `meta.name`.

## 5. Locked design decisions

- **Subscriber registration, not model wrapping.** BeeAI's idiom is `Emitter`; wrapping `ChatModel` would force callers to construct the agent twice.
- **Correlation key = `EventMeta.path` with the trailing `.start|.success|.error` segment stripped.** BeeAI emits per-call hierarchical paths; the stripped path is the stable per-call key. Verified against `framework.emitter.emitter.EventMeta` in 0.3.
- **Idempotency key:** `derive_idempotency_key(tenant_id, session_id, run_id, step_id=f"{run_id}:beeai:{path_hash}", llm_call_id=derive_uuid_from_signature(path_hash, scope="llm_call_id"), trigger="LLM_CALL_PRE")`. Identical shape to the LangChain adapter.
- **Run context via `contextvars`.** Re-uses the LangChain adapter's `_RUN_CONTEXT` ContextVar so a single `async with run_context(...)` covers BeeAI + LangChain in one app.
- **Fail-closed when no run context.** `subscribe_spendguard` never auto-generates a `run_id`; calling an agent without `run_context(...)` raises `RuntimeError` with an actionable message.
- **DEGRADE → APPLY_FAILED** parity with LangChain / pydantic-ai. Mutation patches are surfaced, not applied.
- **Inflight bound:** 10k entries, FIFO eviction, one-shot `logger.warning`. Forgotten POSTs (agent killed mid-call) cannot grow memory unbounded; evicted reservations are released by sidecar TTL sweep.

## 6. Slice plan

| Slice | Title | Size |
|-------|-------|------|
| `COV_D23_S1_module_skeleton` | Module + extras + ImportError contract + `subscribe_spendguard` signature | S |
| `COV_D23_S2_subscribe_helper` | Emitter `match` wiring + inflight map + run-context check | M |
| `COV_D23_S3_reserve_commit_release` | `*.start` reserve, `*.success` commit SUCCESS, `*.error` commit PROVIDER_ERROR + tests | M |
| `COV_D23_S4_demo_and_docs` | `agent_real_beeai` + `agent_real_beeai_deny` modes, docs page, README, verify SQL | M |

4 slices, all S/M, ~1100 LOC total (~600 impl + 400 test + 100 docs).

## 7. Interfaces

```python
# sdk/python/src/spendguard/integrations/beeai.py
def subscribe_spendguard(
    agent: "BaseAgent",
    client: SpendGuardClient,
    *,
    budget_id: str,
    window_instance_id: str,
    unit: common_pb2.UnitRef,
    pricing: common_pb2.PricingFreeze,
    claim_estimator: ClaimEstimator | None = None,
    call_signature_fn: CallSignatureFn | None = None,
    route: str = "llm.call",
) -> Callable[[], None]:  # returns an unsubscribe fn
    ...
```

`ClaimEstimator = Callable[[BeeAiStartEvent], list[common_pb2.BudgetClaim]]` where `BeeAiStartEvent` carries `input` (messages), `model_id`, `path`. When `None`, the existing `_default_estimator` is auto-installed via model-name dispatch (mirrors LangChain `model_post_init`).

## 8. Open questions (locked at spec write)

1. **BeeAI path stability 0.3 → 0.4.** Locked: `Emitter.match(predicate, handler)` matches by predicate, not exact path, so suffix renames don't break us.
2. **`Workflow` multi-LLM coverage.** `Workflow.run` propagates child events to the parent; one `subscribe_spendguard` covers every child LLM call.
3. **Async vs sync.** Async-only — BeeAI awaits async handlers; SpendGuard client is async. Sync agents deferred to v2.
