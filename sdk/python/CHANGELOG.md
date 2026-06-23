# Changelog

## 0.6.1 — 2026-06-23

Patch release: framework-drift and deny-conformance fixes for the in-process
gating adapters. No public API added or removed; all changes are internal
correctness fixes that make the adapters track newer framework versions and
genuinely block the provider call on DENY. Also fixes a runtime-version desync
(`spendguard.__version__` now reports the installed version).

### Fixed

- **`integrations.adk`** — support the ADK 1.35+ keyword-argument model
  callback signature. `SpendGuardAdkCallback.__call__` now accepts the new
  keyword-only `llm_request` / `llm_response` params (plus a `**kwargs`
  catch-all) and normalises them to the single `payload` the PRE/POST dispatch
  discriminates on. Older positional callers and the unit suite are unaffected
  (additive, backward-compatible).
- **`integrations.agno`** — corrected deny semantics for agno 2.6.x. The
  `DecisionDenied` → `InputCheckError` wrap that halts the model before the
  vendor SDK is unchanged and remains load-bearing, but a DENY surfaces as
  `Agent.arun()` returning `RunOutput(status=RunStatus.error)` rather than a
  propagated exception — callers must detect deny via `RunOutput.status`, not
  by catching `DecisionDenied`. Documentation-only change.
- **`integrations.beeai`** — four fixes for BeeAI 0.1.x `ChatModel.run`:
  (1) `_stable_call_key` strips a leading `run.` prefix so the Run-level mirror
  and inner backend emit collapse to one call key; (2) a race-safe synchronous
  inflight placeholder (claimed before the `request_decision` await, popped on
  any exception) stops the concurrently-dispatched duplicate `start` from
  double-reserving the budget; (3) the event predicate now also matches a
  `chat` path segment, not just `llm`; (4) usage extraction reads
  `ChatModelSuccessEvent.value.usage` so the commit estimate is non-zero and
  the reservation no longer leaks.
- **`integrations.letta`** — gate Letta 0.16.x's low-level
  `LLMClientBase.request_async` provider call (the 0.8.0-era `send_llm_request`
  surface was removed). Same fail-closed PRE `request_decision` (DENY raises
  `DecisionDenied` before any provider HTTP) → inner call → POST. Token and
  provider-event-id extraction are now dict-aware to read the raw provider
  `dict` that `request_async` returns, in addition to the object shape.
- **`integrations.llamaindex`** — read token counts from pydantic provider
  responses. A new `_coerce_mapping` helper normalises a non-`Mapping`
  `response.raw` (e.g. the `ChatCompletion` / `CompletionUsage` pydantic
  objects from `llama-index-llms-openai` on openai-python v1) via
  `model_dump` / `dict` / `to_dict`, so real token counts are read instead of
  0 (which previously failed the POST commit with
  `estimated_amount_atomic must be > 0`).
- **`integrations.strands`** — track the Strands GA (>=1.0)
  `BeforeInvocationEvent` / `AfterInvocationEvent` shape, which exposes
  `agent` / `messages` / `invocation_state` directly and has no
  `event.invocation` wrapper or `invocation_id`. The adapter no longer raises
  on the missing wrapper; it correlates the before/after pair via a
  `_spendguard_invocation_id` stored in the shared `invocation_state` dict and
  reads `model` from the agent.
- Fixed `spendguard.__version__`, which was pinned at `"0.5.1"` and now matches
  the package version.

## 0.6.0 — 2026-06-19

### Added

- **`spendguard.integrations.ag_ui`** — AG-UI spend-event family
  (coverage deliverable D39, slice 2). **Display-only**: the events are a
  presentation surface — SpendGuard enforcement happens in the SpendGuard
  adapters and sidecar before the provider call; the events report
  decisions already made and can neither grant nor deny spend. Pure
  builders for the five `spendguard.*` AG-UI `CUSTOM` events:
  `spendguard.budget.snapshot`, `spendguard.reservation.created`,
  `spendguard.reservation.committed`, `spendguard.reservation.released`,
  and `spendguard.decision.denied` — the 1:1 snake_case mirror of
  `@spendguard/ag-ui` (npm). Includes `canonical_event_json` (locked
  sorted-keys/UTF-8/no-whitespace rule; byte-identical to the TS output
  for identical inputs, proven against the frozen
  `sdk/fixtures/cross-language/ag_ui_v1.json` corpus) and `encode_sse`.
  Zero runtime deps beyond stdlib; new optional extra
  `ag-ui = ["ag-ui-protocol>=0.1.19,<0.2"]` exists only for users who
  validate events through the upstream pydantic models. Events are
  unsigned UI hints — never the audit chain.
- **`spendguard.integrations.litellm_sdk_shim`** — LiteLLM SDK
  monkey-patch shim that closes [LiteLLM Issue #8842](https://github.com/BerriAI/litellm/issues/8842).
  Calling `install_shim(SpendGuardShimOptions(...))` once at boot
  replaces `litellm.acompletion`, `litellm.completion`,
  `litellm.atext_completion`, `litellm.text_completion`, and
  `litellm.Router.acompletion` (plus operator-defined Router
  subclasses) with SpendGuard-gated wrappers. Direct callers AND
  every transitive caller — CrewAI, DSPy, SmolAgents, Strands,
  BeeAI, AutoGen, Atomic Agents — gain a fail-closed pre-call
  budget gate with NO framework-side changes.
  - Public API: `install_shim`, `uninstall_shim`, `is_installed`,
    `SpendGuardShimOptions`, `SpendGuardShimAlreadyInstalled`,
    `SpendGuardShimSyncInAsyncContext`.
  - Idempotent install via `config_signature` hash; same options =
    silent no-op, different options = raise.
  - `uninstall_shim` walks `state.originals` in reverse so subclass
    restores precede parent restores.
  - Re-entry guard via `contextvars.ContextVar[bool]` so
    LiteLLM-internal fallback / Router retry chains that re-enter
    a patched entry point short-circuit to the saved original (no
    double-reserve).
  - Sync wrappers refuse to bridge from inside a running event loop
    (would deadlock); raise `SpendGuardShimSyncInAsyncContext` with
    a hint to use `await litellm.acompletion(...)` instead.
  - End-of-stream commit reads `response.usage.completion_tokens`
    (OpenAI shape; LiteLLM normalises Anthropic / Bedrock / Gemini).
    `asyncio.CancelledError` mid-call routes through
    `emit_llm_call_post(outcome=CANCELLED)` then re-raises.
  - D12 ships via the **existing `spendguard-sdk` PyPI package** — no
    new package, no new extras.
- Two new demo modes prove the shim end-to-end against a real
  SpendGuard sidecar:
  - `DEMO_MODE=litellm_sdk_real` — 3-step matrix (ALLOW + STREAM +
    TRANSITIVE/CrewAI).
  - `DEMO_MODE=litellm_sdk_deny` — 3-substep fail-closed matrix
    (ALLOW positive control + DENY budget-exhausted + DENY
    sidecar-unreachable).
- Documentation: `docs/site-v2/.../integrations/litellm-sdk-shim.mdx`
  with the 4-surface decision matrix (proxy callback / proxy
  guardrail / direct acompletion / SDK shim).

### Fixed

- `litellm_sdk_shim._core` now catches `asyncio.CancelledError`
  alongside `Exception` so cancellation mid-call routes through the
  release commit path (the audit row gets `outcome=CANCELLED`).
  Previously cancellation skipped the release commit + leaked the
  reservation past TTL. The release commit is wrapped in
  `asyncio.shield` so a follow-on cancel of the outer task does not
  abort the audit-side commit.

## 0.5.1 — 2026-06-02

### Changed

- Patch release for the hardened predictor-upgrade mainline after CI
  efficiency fixes. The SDK package content remains the 0.5.x API
  surface: default token estimators, `with_run_plan`, and regenerated
  proto stubs for the current SpendGuard wire contracts.
- Pins the SDK proto generator path to `grpcio-tools<1.72` while the
  runtime dependency remains `protobuf<6`, preventing protobuf 6.x
  gencode from entering release wheels.

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
