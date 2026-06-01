# Changelog

## 0.5.0 — 2026-05-30

SLICE_12 — Python SDK gains **default token estimators** for OpenAI /
Anthropic / Gemini models so callers no longer have to write a
`claim_estimator` by hand, and a new **`with_run_plan` decorator**
wires Signal 3 (`planned_steps_hint`) per
`run-cost-projector-spec-v1alpha1.md` §5.

### Added

- **`spendguard.estimators`** package — Python mirror of the Rust
  `crates/spendguard-tokenizer` dispatch table (SLICE_03 + SLICE_04
  R2). 16-entry first-match-wins regex table covers OpenAI
  (cl100k/o200k/p50k via `tiktoken`), Anthropic Claude 3.x/3.5
  (native + Bedrock + cross-region prefix via vendored BPE),
  Gemini 1.5/2.0 (vendored Gemma approximation), and Llama Bedrock
  (server-side only; SDK warns + chars/4 fallback). Cohere
  intentionally omitted (Rust ships feature-gated pending legal
  review). Unknown models warn + chars/4 fallback.
  - Public surface: `estimator_for_model(model) → EstimatorFns`,
    `EncoderKind`, `TiktokenFamily`, `dispatch_table()`, `lookup()`.
- **Vendored tokenizer assets**: `spendguard/data/anthropic_claude3_tokenizer.json`
  (~1.7 MB, Xenova/claude-tokenizer MIT mirror) +
  `spendguard/data/gemini_1_5_tokenizer.json` (~17 MB,
  Xenova/gemma-tokenizer Apache 2.0 mirror). Both byte-identical to
  the Rust crate's vendored copy; sha256 verified at first call per
  `tokenizer-service-spec-v1alpha1.md` §7.4.1 (fail-fast on mismatch).
  See `LICENSE_NOTICES.md` for provenance + reproducibility.
- **`with_run_plan(planned_calls, planned_tools)` decorator** —
  Signal 3 power-user API per `run-cost-projector-spec-v1alpha1.md`
  §5.1. Works on sync OR async; nested usage → outer wins;
  context-var cleared on exception. The decorator stamps
  `planned_steps_hint = planned_calls + planned_tools` on every
  `DecisionRequest` issued inside the decorated frame.
- **`RunPlan` + `current_run_plan()`** — exported helpers for
  framework adapters that need to read the active plan.
- **Default `claim_estimator` in all 5 integrations** (litellm /
  langchain / pydantic_ai / openai_agents / agt). When omitted /
  `None`, each integration auto-builds a default from the inner
  model's name. Backward compat: explicit non-None still wins per
  spec §8.5.

### Use it

```python
from spendguard import with_run_plan
from spendguard.integrations.langchain import SpendGuardChatModel
from langchain_openai import ChatOpenAI

# No more manual claim_estimator!
guarded = SpendGuardChatModel(
    inner=ChatOpenAI(model="gpt-4o-mini"),
    client=client,
    budget_id="...",
    window_instance_id="...",
    unit=...,
    pricing=...,
    # claim_estimator omitted — default dispatched from "gpt-4o-mini"
)

# Signal 3 — declare expected step count
@with_run_plan(planned_calls=8, planned_tools=2)
async def my_agent(query: str) -> str:
    # ... agent runs 10 total steps ...
    return await runner.run(...)
```

### Changed

- `protobuf` runtime now `>=4.25,<6` (unchanged); proto stubs
  regenerated against `grpcio-tools 1.71` (gencode 5.28.1 — compatible
  with the existing protobuf 5.x runtime).
- `tiktoken>=0.6,<1.0` and `tokenizers>=0.20,<1.0` promoted to **core
  dependencies** (no longer optional). Every integration's default
  `claim_estimator` uses them.

### Notes

- 50 golden parity samples + 100-string property test validate the
  Python dispatch matches the Rust table byte-for-byte.
- Asset sha256 verification: `LICENSE_NOTICES.md` is the source of
  truth; estimator modules + shipped assets MUST all agree (parity
  test `test_vendored_assets.py::TestLicenseNoticesParity`).
- Demo regression: `make demo-up DEMO_MODE=agent_real_langgraph` works
  without caller-supplied `claim_estimator` (default dispatched from
  inner ChatOpenAI's `model_name`).

## 0.4.0 — 2026-05-23

Adds the explicit `release_reservation()` SDK method, the first
adapter caller of the new sidecar `ReleaseReservation` gRPC RPC
(PR #84). Closes the Agent Spend Protocol Draft-01 §4 Release wire
surface end-to-end: an ASP-conformant Python adapter can now reserve
budget, abort the operation, and explicitly release the held
reservation via the wire shape the spec defines.

### Added

- **`SpendGuardClient.release_reservation()`** — async method that
  calls the sidecar's new `ReleaseReservation` RPC. Takes
  `reservation_id`, `idempotency_key`, optional `reason_codes` (free-
  form per ASP Draft-01 §4; sidecar maps known values to internal
  release-reason enum, unknown → Explicit). Returns
  `ReleaseOutcome(audit_event_signature, ledger_transaction_id,
  released_reservation_ids)`.
- **`ReleaseOutcome` dataclass** — exported from top-level
  `spendguard` package. Detached Ed25519 signature of the emitted
  `audit.release` CloudEvent (non-empty on first success; empty on
  cache-miss ledger replay branch).
- Demo coverage: `DEMO_MODE=decision` now exercises the explicit
  Release RPC as a smoke step before the main commit flow. Verifies
  SDK → sidecar → ledger → audit chain end-to-end. Output line:
  `[demo] release_reservation OK reservation_id=… ledger_tx=…
  sig_bytes=64`.

### Use it

```python
from spendguard import SpendGuardClient

async with SpendGuardClient(socket_path=..., tenant_id=...) as c:
    await c.handshake()
    outcome = await c.request_decision(...)
    reservation_id = outcome.reservation_ids[0]

    # ... agent run aborts, provider call cancelled, etc. ...

    release = await c.release_reservation(
        reservation_id=reservation_id,
        idempotency_key=f"my-app:{reservation_id}",
        reason_codes=("run_cancelled",),
    )
    # release.ledger_transaction_id — stable across retries
    # release.audit_event_signature — pin the release receipt
```

### Spec alignment

- ASP Draft-01 §4 wire shape: SDK request fields match
  `reservation_id` / `idempotency_key` / `reason_codes` canonical
  tags 1-3; SpendGuard-specific extensions (`tenant_id`,
  `workload_instance_id`, `session_id`) ride at proto tags 100+.
- ASP Draft-01 §8 closed-deltas section now lists this as the first
  closed delta with end-to-end SDK exercise.

### Post-GA hardening update — 2026-06-02

- Same-process `ReleaseReservation` replay now returns the original
  `audit_event_signature` from the sidecar replay cache. Cache-miss
  replay still returns empty bytes rather than fabricating a receipt.
- Retry replay is allowed to reach the ledger idempotency branch after
  local fencing TTL movement; first-time release mutations still require
  active fencing.
- Ledger `IdempotencyConflict` now uses the shared
  `IDEMPOTENCY_CONFLICT` proto code and maps to gRPC
  `FailedPrecondition`.

### Unchanged

- All `0.3.0` LiteLLM / OpenAI Agents / LangChain / LangGraph /
  Pydantic-AI / AGT integrations carry forward identically.
- No breaking changes to existing public API.

---

## 0.3.0 — 2026-05-20

First non-pre-release. Bumps Dev Status from 3-Alpha → 4-Beta.
LiteLLM integration end-to-end + signed audit chain enrichment.

### Added

- **`spendguard.integrations.litellm`** — full LiteLLM proxy + direct
  async integration (Phase 5):
  - `SpendGuardLiteLLMCallback` + `_LoopBoundCallback` — proxy
    CustomLogger that gates every `/v1/chat/completions` call (pre-call
    reserve, post-call commit, streaming reconciler, failure release).
  - `SpendGuardDirectAcompletion` — async wrapper for
    `litellm.acompletion()` direct callers (no proxy needed). Sync
    `litellm.completion()` NOT supported (see ADR-005).
  - Fail-closed defaults; `SPENDGUARD_LITELLM_FAIL_OPEN=1` dev override.
- `spendguard.integrations.openai_agents` — OpenAI Agents SDK gating
  (model-wrap + composite paths).
- `spendguard.integrations.langchain` / `.langgraph` — ChatModel
  gating with streaming reconciler.
- `spendguard.integrations.agt` — Microsoft AGT composite evaluator
  (PolicyEngine + SpendGuard as policy plugin).
- 5 new demo modes: `agent_real_openai_agents`, `agent_real_agt`,
  `litellm_real`, `litellm_deny`, `litellm_direct`.
- `examples/litellm-proxy-composite/` — runnable example with
  docker-compose + production-shape callback template.

### Changed

- `DecisionDenied.status_code = 403` — LiteLLM proxy now maps
  SpendGuard policy denials to HTTP 403 (was: 500).
- `SidecarUnavailable.status_code = 503` — transient infra failures
  map to HTTP 503 Service Unavailable.
- `litellm` extra now pulls `litellm[proxy]` transitively
  (fastapi/uvicorn/gunicorn) so `python -m litellm.proxy.proxy_cli`
  works out of the box.

### Audit chain (GH #77 closed)

The sidecar now extracts a 12-key allowlist from
`request_decision.runtime_metadata` and emits the values into
`canonical_events.payload_json.data.spendguard.*` for both ALLOW +
DENY CloudEvent payloads. Allowlist: integration / litellm_call_id /
model / pricing_version / price_snapshot_hash_hex / fx_rate_version /
unit_conversion_version / prompt_hash / call_type / stream / mode /
team_id. PII smuggling guard: only scalar kinds accepted; Struct/List
dropped with WARN.

### Documentation

- `docs/specs/litellm-integration/PROXY_RECIPE.md` — operator recipe.
- `docs/site/docs/integrations/litellm.md` — public docs page.
- `docs/specs/litellm-integration/100-percent-design.md` — architect's
  design lock for the 100%-completion epics.

## 0.1.0a1 (Phase 4 O1) — 2026-05-09

Initial SDK release. Restructured from `spendguard-pydantic-ai` to the
multi-framework `spendguard-sdk` with optional extras.

### Added

- Top-level package `spendguard` with framework-agnostic core
  (`SpendGuardClient`, `DecisionStopped`, etc.)
- `spendguard.integrations.pydantic_ai` (was `spendguard_pydantic_ai`
  top-level) — gated behind `pip install 'spendguard-sdk[pydantic-ai]'`
- Slots reserved for `spendguard.integrations.langchain`,
  `spendguard.integrations.langgraph`,
  `spendguard.integrations.openai_agents` (Phase 4 O5).

### Changed

- Package name: `spendguard-pydantic-ai` → `spendguard-sdk`
- Python module: `spendguard_pydantic_ai` → `spendguard`
- Pydantic-AI wrapper moved from top-level to
  `spendguard.integrations.pydantic_ai`
- Internal contextvar renamed `spendguard_pydantic_ai_run_context` →
  `spendguard_run_context`

### Migration from `spendguard-pydantic-ai`

```python
# Before
from spendguard_pydantic_ai import SpendGuardClient, SpendGuardModel, RunContext

# After
from spendguard import SpendGuardClient
from spendguard.integrations.pydantic_ai import SpendGuardModel, RunContext
```

```bash
# Before
pip install spendguard-pydantic-ai

# After
pip install 'spendguard-sdk[pydantic-ai]'
```
