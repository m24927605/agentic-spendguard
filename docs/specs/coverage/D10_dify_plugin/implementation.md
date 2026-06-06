# D10 — Implementation

**Reads:** [`design.md`](design.md), [`acceptance.md`](acceptance.md), [`review-standards.md`](review-standards.md).
**Touches:** New plugin tree under `plugins/dify/` + demo orchestration + public docs. No Rust changes. No proto changes. No DB schema changes.

## 1. Module layout

```
plugins/dify/                                       # NEW — Dify plugin source tree
├── manifest.yaml                                   # plugin metadata (Slice 2)
├── provider/
│   ├── spendguard.yaml                             # provider schema (Slice 2)
│   └── spendguard.py                               # ProviderImpl + validate_credentials (Slice 3)
├── models/
│   └── llm/
│       ├── llm.py                                  # SpendGuardLLM class (Slices 3-6)
│       ├── _reservation.py                         # _DifyReservation delegate (Slice 3)
│       ├── _upstream/
│       │   ├── __init__.py                         # UpstreamClient interface
│       │   ├── openai.py                           # OpenAI forwarder (Slice 4)
│       │   ├── anthropic.py                        # Anthropic forwarder (Slice 5)
│       │   ├── gemini.py                           # stub raising NotImplementedError in v1
│       │   └── bedrock.py                          # stub raising NotImplementedError in v1
│       └── spendguard.yaml                         # llm model schema (Slice 2)
├── requirements.txt                                # spendguard-sdk + openai + anthropic + dify-plugin
├── pyproject.toml                                  # editable install for tests
├── tests/
│   ├── test_provider.py                            # validate_credentials roundtrip (Slice 3)
│   ├── test_reservation.py                         # _DifyReservation unit tests (Slice 3)
│   ├── test_openai_invoke.py                       # Slice 4
│   ├── test_anthropic_invoke.py                    # Slice 5
│   ├── test_streaming.py                           # Slice 6
│   └── _fake_sidecar.py                            # reused from sdk/python/tests
└── README.md                                       # operator-facing

deploy/demo/
├── Makefile                                        # +DEMO_MODE=dify_plugin_real branch (Slice 7)
├── compose.yaml                                    # +dify-api, dify-worker, dify-plugin services (Slice 7)
├── dify_plugin/                                    # NEW
│   ├── compose.override.yaml                       # Dify stack overlay
│   ├── seed_workspace.sql                          # seeds workspace + provider config
│   └── README.md
├── verify_step_dify_plugin.sql                     # NEW — SQL gate (Slice 7)
└── demo/run_demo.py                                # +run_dify_plugin_real_mode (Slice 7)

docs/site/docs/integrations/
└── dify.md                                         # NEW — public docs page (Slice 8)

.github/workflows/
└── dify-plugin-publish.yml                         # NEW — package + publish job (Slice 8)
```

## 2. Slice breakdown

### Slice 1 — Plugin scaffold (S)

**Files:** `plugins/dify/manifest.yaml` (placeholder), `plugins/dify/pyproject.toml`, `plugins/dify/requirements.txt`, `plugins/dify/README.md`.

Runs `dify plugin init --name spendguard --type model-provider` against a vendored Dify CLI binary or container (no network needed). Commits the verbatim scaffold tree, then patches `manifest.yaml` `name` / `author` / `version` / `icon` / `description` / `tags` / `created_at`. Adds `spendguard-sdk>=0.5.1`, `openai>=1.40`, `anthropic>=0.40`, `dify-plugin>=0.2.0` to `requirements.txt`. `pyproject.toml` declares an editable install for the in-tree pytest suite.

Acceptance: `cd plugins/dify && python -m dify_plugin.cli check` exits 0 (validates the scaffold structure).

### Slice 2 — Provider manifest (S)

**Files:** `plugins/dify/provider/spendguard.yaml`, `plugins/dify/models/llm/spendguard.yaml`.

`provider/spendguard.yaml` declares the credentials schema operators fill in via the Dify provider UI:

```yaml
provider: spendguard
label:
  en_US: SpendGuard (Budget-Gated Forwarder)
icon_small:
  en_US: icon_s_en.svg
icon_large:
  en_US: icon_l_en.svg
description:
  en_US: >-
    Forwards LLM calls to your chosen upstream (OpenAI/Anthropic/Gemini/Bedrock)
    after reserving against a SpendGuard sidecar. Fail-closed budget gate +
    signed audit chain. Set SPENDGUARD_SIDECAR_UDS + SPENDGUARD_TENANT_ID env
    on the plugin daemon container.
supported_model_types: [llm]
configurate_methods: [predefined-model, customizable-model]
provider_credential_schema:
  credential_form_schemas:
    - variable: upstream_provider
      label: { en_US: Upstream Provider }
      type: select
      required: true
      options:
        - value: openai
          label: { en_US: OpenAI }
        - value: anthropic
          label: { en_US: Anthropic }
        - value: gemini
          label: { en_US: Google Gemini (v1.1+) }
        - value: bedrock
          label: { en_US: AWS Bedrock (v1.1+) }
    - variable: upstream_api_key
      label: { en_US: Upstream API Key }
      type: secret-input
      required: true
    - variable: upstream_base_url
      label: { en_US: Upstream Base URL (optional) }
      type: text-input
      required: false
    - variable: spendguard_budget_id
      label: { en_US: SpendGuard Budget ID }
      type: text-input
      required: true
    - variable: spendguard_window_instance_id
      label: { en_US: SpendGuard Window Instance ID }
      type: text-input
      required: true
```

`models/llm/spendguard.yaml` lists the canonical model entries surfaced in Dify's chat-app model picker (`spendguard/gpt-4o-mini`, `spendguard/claude-3-5-sonnet-latest`, etc.); features carry over from each upstream model (function-calling, vision, etc.).

### Slice 3 — `SpendGuardLLM` skeleton + `_DifyReservation` delegate (M)

**Files:** `plugins/dify/models/llm/llm.py`, `plugins/dify/models/llm/_reservation.py`, `plugins/dify/provider/spendguard.py`, `plugins/dify/tests/test_reservation.py`, `plugins/dify/tests/test_provider.py`.

```python
# plugins/dify/models/llm/_reservation.py
"""Reservation/commit/release delegate for the Dify LLM provider plugin.

Mirrors the shape of sdk/python/src/spendguard/integrations/litellm.py
SpendGuardLiteLLMCallback (lines 204-760). Composition over inheritance:
the Dify SDK base class (LargeLanguageModel) and the SpendGuard reservation
lifecycle are orthogonal state machines.
"""
from __future__ import annotations

import asyncio
import logging
import os
from dataclasses import dataclass
from typing import Any, Mapping

from spendguard.client import SpendGuardClient
from spendguard.errors import (
    DecisionDenied, SidecarUnavailable, SpendGuardConfigError, SpendGuardError,
)
from spendguard.ids import derive_idempotency_key, derive_uuid_from_signature
from spendguard.prompt_hash import compute as compute_prompt_hash

log = logging.getLogger("spendguard.dify_plugin.reservation")


@dataclass(frozen=True, slots=True)
class DifyCallContext:
    workspace_id: str
    app_id: str | None
    model: str
    prompt_messages: list[Any]
    stream: bool
    credentials: Mapping[str, Any]


@dataclass(frozen=True, slots=True)
class ReservationHandle:
    decision_id: str
    reservation_id: str
    llm_call_id: str
    run_id: str
    step_id: str
    binding: "BudgetBinding"
    estimator_snapshot: Any  # frozen primitive snapshot
    stream: bool


class _DifyReservation:
    def __init__(self, *, socket_path: str, tenant_id: str) -> None:
        self._socket_path = socket_path
        self._tenant_id = tenant_id
        self._client: SpendGuardClient | None = None
        self._init_lock = asyncio.Lock()
        self._fail_open_dev = (
            os.environ.get("SPENDGUARD_DIFY_FAIL_OPEN", "").strip() == "1"
        )

    async def _ensure_client(self) -> SpendGuardClient:
        # Identical pattern to _LoopBoundCallback._ensure_client (litellm.py:804-863)
        # 5s deadline, per-attempt 1s timeout, backoff, deadline-bounded.
        ...

    async def reserve(self, ctx: DifyCallContext) -> ReservationHandle:
        """Build binding, estimate, request_decision. Raises DecisionDenied on
        DENY, SidecarUnavailable on DEGRADE (unless fail-open env set)."""
        ...

    async def commit_success(
        self, handle: ReservationHandle, real_usage: Mapping[str, int],
        provider_event_id: str,
    ) -> None: ...

    async def release_failure(
        self, handle: ReservationHandle, exc: BaseException | str,
    ) -> None:
        """Swallows release-RPC errors (TTL sweep is durable backstop)."""
        ...
```

```python
# plugins/dify/models/llm/llm.py
from dify_plugin import LargeLanguageModel  # type: ignore
from dify_plugin.errors.model import (
    InvokeAuthorizationError, InvokeServerUnavailableError, InvokeError,
)
from ._reservation import _DifyReservation, DifyCallContext
from ._upstream import build_upstream_client


class SpendGuardLLM(LargeLanguageModel):
    def __init__(self, *a: Any, **kw: Any) -> None:
        super().__init__(*a, **kw)
        self._reservation = _DifyReservation(
            socket_path=os.environ["SPENDGUARD_SIDECAR_UDS"],
            tenant_id=os.environ["SPENDGUARD_TENANT_ID"],
        )

    def _invoke(
        self, model: str, credentials: dict, prompt_messages: list,
        model_parameters: dict, tools: list | None = None,
        stop: list | None = None, stream: bool = True, user: str | None = None,
    ) -> "LLMResult | Iterator[LLMResultChunk]":
        # Dify SDK calls _invoke synchronously; we run the async reservation
        # via asyncio.run_coroutine_threadsafe against a daemon-scoped loop
        # (lazy-initialised). Pattern matches dify-plugin-sdk reference impls.
        ctx = DifyCallContext(
            workspace_id=str(credentials.get("__dify_workspace_id", "")),
            app_id=credentials.get("__dify_app_id"),
            model=model, prompt_messages=prompt_messages, stream=stream,
            credentials=credentials,
        )
        if stream:
            return self._stream_generate(ctx, model_parameters, tools, stop, user)
        return self._generate(ctx, model_parameters, tools, stop, user)
```

`provider/spendguard.py::SpendGuardProvider.validate_credentials` issues a 1-token reserve+release roundtrip to prove sidecar wiring (Slice 3 acceptance gate).

### Slice 4 — OpenAI upstream (M)

**Files:** `plugins/dify/models/llm/_upstream/openai.py`, `plugins/dify/tests/test_openai_invoke.py`.

`_upstream/openai.py` builds an `openai.OpenAI` client from `credentials.upstream_api_key` + `upstream_base_url`. Non-streaming `_generate` calls `client.chat.completions.create(...)`, returns Dify's `LLMResult` constructed from `response.choices[0].message` + `response.usage`. Real usage feeds `_reservation.commit_success`.

```python
class OpenAIUpstream:
    def generate(self, ctx, params, tools, stop, user) -> LLMResult:
        client = openai.OpenAI(
            api_key=ctx.credentials["upstream_api_key"],
            base_url=ctx.credentials.get("upstream_base_url") or None,
            timeout=60.0,
        )
        kwargs = {"model": ctx.model.removeprefix("spendguard/"), ...}
        response = client.chat.completions.create(**kwargs)
        return self._to_dify_result(response)
```

Reconciler reads `response.usage.completion_tokens` + `prompt_tokens`. Failure path raises Dify's `InvokeError` subclasses based on the upstream `openai.APIError` hierarchy.

### Slice 5 — Anthropic upstream + `get_num_tokens` (M)

**Files:** `plugins/dify/models/llm/_upstream/anthropic.py`, `plugins/dify/tests/test_anthropic_invoke.py`.

Same pattern as Slice 4 with `anthropic.Anthropic` client. Reconciler reads `response.usage.input_tokens` + `output_tokens`. Message-format adapter translates Dify's `prompt_messages` (OpenAI shape) into Anthropic's `system` + `messages` split.

`SpendGuardLLM.get_num_tokens` dispatches via the sidecar `count_tokens` UDS RPC keyed on the upstream provider — no bundled tokenizer.

### Slice 6 — Streaming path (M)

**Files:** `plugins/dify/models/llm/llm.py::_stream_generate`, `plugins/dify/tests/test_streaming.py`.

`_stream_generate` yields `LLMResultChunk` per upstream SSE event. Reservation is taken once at the top; commit fires after the stream completes via try/finally with a `_streaming_accumulator` that captures the final `usage` chunk (OpenAI) or the synthetic `message_delta.usage` event (Anthropic). When upstream omits `usage`, falls back to estimator snapshot + WARN log (same contract as `_async_log_success_streaming` in `litellm.py:599-607`).

Cancellation (caller closes the SSE) routes to `release_failure(ctx, asyncio.CancelledError())`, which classifies as CANCELLED (same classification as `_classify_failure`).

### Slice 7 — Demo mode (M)

**Files:** `deploy/demo/Makefile`, `deploy/demo/compose.yaml`, `deploy/demo/dify_plugin/compose.override.yaml`, `deploy/demo/dify_plugin/seed_workspace.sql`, `deploy/demo/verify_step_dify_plugin.sql`, `deploy/demo/demo/run_demo.py`.

Makefile branch:

```
else ifeq ($(DEMO_MODE),dify_plugin_real)
	@echo "[demo] DEMO_MODE=dify_plugin_real → Dify + plugin daemon + sidecar"
	$(COMPOSE) -f compose.yaml -f dify_plugin/compose.override.yaml up -d --build \
	    postgres pki-init bundles-init canonical-seed-init manifest-init \
	    endpoint-catalog ledger canonical-ingest tokenizer sidecar \
	    dify-api dify-worker dify-plugin-daemon
```

`dify_plugin/compose.override.yaml` mounts `langgenius/dify-api:1.0` and `langgenius/dify-worker:1.0` against a local Redis + a separate `dify_db` Postgres database (Dify can share the demo's Postgres instance under a distinct database name to avoid schema collisions with `spendguard_ledger`). The `dify-plugin-daemon` service mounts `plugins/dify/` (read-only) and the sidecar UDS.

`seed_workspace.sql` (run by `dify_plugin-init`) creates a Dify workspace + an app + a provider credential row pointing at `spendguard` with `upstream_provider=openai` + the budget/window IDs that match the canonical seed.

Demo driver `run_dify_plugin_real_mode` (~150 LOC):
1. POST `/v1/chat-messages` against Dify's app endpoint, small messages. Assert 200, sidecar audit row reserved + committed.
2. POST that exceeds budget. Assert 403 (Dify's translation of `InvokeAuthorizationError`), DENY decision audited, **no upstream HTTP** (verified via a counting stub in front of `api.openai.com` for the demo).
3. POST with `response_mode=streaming`. Assert 200 SSE, end-of-stream commit row.

### Slice 8 — Docs + publish job (S)

**Files:** `docs/site/docs/integrations/dify.md`, `.github/workflows/dify-plugin-publish.yml`, `README.md`.

Docs page covers: "Why SpendGuard for Dify", "Install (Dify Cloud)", "Install (Self-hosted)", a decision matrix vs egress-proxy, limitations (no workflow-step gating), and the `validate_credentials` install probe.

Publish workflow runs `dify plugin package` to produce `spendguard.difypkg`, signs it, uploads as a GH Release asset, and (when secrets are set) pushes to the Dify plugin marketplace registry.

README gains one row in the adapter integrations table: `Dify Model Provider | Python plugin | dify plugin install spendguard.difypkg`.

## 3. Backwards compatibility

| Surface | Action |
|---------|--------|
| Existing `examples/` / `sdk/python/` integrations | Untouched. |
| `compose.yaml` for other demo modes | Unchanged. The Dify services live in an overlay file, opt-in per DEMO_MODE branch. |
| Existing PyPI extras of `spendguard-sdk` | Unchanged. The Dify plugin is its own package, not an SDK extra. |
| Existing DB schemas | Unchanged. Dify uses a separate database name (`dify_db`) on the same Postgres instance. |

## 4. Failure modes (must be tested)

| Mode | Expected | Test |
|------|----------|------|
| `dify-plugin-sdks` < 0.2 | ImportError at plugin load with install hint | `test_provider::test_import_floor` |
| `SPENDGUARD_SIDECAR_UDS` unset | `SpendGuardConfigError` at plugin daemon boot | `test_reservation::test_env_missing_uds` |
| `upstream_provider=openai` + missing `upstream_api_key` | Dify validation error before plugin runs | `test_provider::test_validate_rejects_empty_key` |
| Sidecar DENY | Dify HTTP 403 (`InvokeAuthorizationError`); **no upstream HTTP** | `test_openai_invoke::test_deny_no_upstream` + demo step 2 |
| Sidecar DEGRADE | Dify HTTP 503 (`InvokeServerUnavailableError`) | `test_reservation::test_degrade_fail_closed` |
| `SPENDGUARD_DIFY_FAIL_OPEN=1` + DEGRADE | Forwards to upstream + WARN + no commit row | `test_reservation::test_fail_open_skips_commit` |
| Upstream `usage` is None on streaming | Estimator-snapshot commit + WARN | `test_streaming::test_no_usage_estimator_fallback` |
| Upstream raises `openai.APIError` | `_reservation.release_failure` fires + raise translated `InvokeError` | `test_openai_invoke::test_upstream_failure_releases` |
| Plugin v1.1+ upstream `gemini` selected in v1 | `InvokeError("upstream provider gemini not supported in this plugin version")` at `_invoke` | `test_openai_invoke::test_gemini_stub_raises` |

## 5. Code skeleton — total LOC budget

| File | Impl LOC | Test LOC |
|------|----------|----------|
| `_reservation.py` | ~280 | covered by test_reservation.py |
| `llm.py` | ~180 | covered by test_*_invoke.py |
| `_upstream/openai.py` | ~140 | ~150 |
| `_upstream/anthropic.py` | ~150 | ~150 |
| `_upstream/__init__.py` | ~30 | — |
| `provider/spendguard.py` | ~80 | ~80 |
| `test_reservation.py` | — | ~250 |
| `test_streaming.py` | — | ~150 |
| Demo driver / verify SQL | ~150 + ~80 | — |
| **Total** | **~1010 + 230 demo** | **~780** |

## 6. Out of scope

Everything in design.md §3. Plus: no changes to `sdk/python/src/spendguard/integrations/`. Plus: no proto changes. Plus: no control-plane API changes. Plus: Gemini + Bedrock upstream implementations beyond NotImplementedError stubs (deferred to v1.1; tracked as GH issues).
