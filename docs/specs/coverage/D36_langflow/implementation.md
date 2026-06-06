# D36 — Implementation

**Reads:** [`design.md`](design.md), [`acceptance.md`](acceptance.md), [`review-standards.md`](review-standards.md).
**Touches:** New package tree under `plugins/langflow/` + demo orchestration + public docs. No Rust changes. No proto changes. No DB schema changes. **No `sdk/python/src/spendguard/integrations/langchain.py` changes** — D36 is a packaging-and-glue layer over that file.

## 1. Module layout

```
plugins/langflow/                                      # NEW — Langflow component source tree
├── pyproject.toml                                     # spendguard-langflow-component (Slice 3)
├── README.md                                          # operator-facing
├── src/spendguard_langflow/
│   ├── __init__.py                                    # exports SpendGuardChatModelWrapper
│   ├── component.py                                   # SpendGuardChatModelWrapper class (Slices 1-2)
│   ├── _run_context.py                                # run-context auto-binding helper (Slice 2)
│   ├── _install.py                                    # install_into_langflow CLI entry (Slice 3)
│   └── _version.py                                    # package version (Slice 3)
├── metadata/
│   └── spendguard_chat_model_wrapper.yaml             # Langflow component metadata (Slice 3)
├── tests/
│   ├── test_component_skeleton.py                     # Slice 1
│   ├── test_build_model.py                            # Slice 2 — wraps a fake BaseChatModel
│   ├── test_run_context_autobind.py                   # Slice 2
│   ├── test_install_script.py                         # Slice 3
│   └── _fake_sidecar.py                               # reused from sdk/python/tests
└── examples/
    └── flow_chat_openai_wrapped.json                  # Slice 4 — Langflow flow JSON

deploy/demo/
├── Makefile                                           # +DEMO_MODE=langflow_real branch (Slice 4)
├── compose.yaml                                       # unchanged; overlay added separately
├── langflow/                                          # NEW
│   ├── compose.override.yaml                          # Langflow + components mount
│   ├── components_seed/                               # mount point — component dropped here
│   │   └── spendguard_chat_model_wrapper.py
│   ├── flows_seed/
│   │   └── flow_chat_openai_wrapped.json              # imported on boot
│   └── README.md
├── verify_step_langflow.sql                           # NEW — SQL gate (Slice 4)
└── demo/run_demo.py                                   # +run_langflow_real_mode (Slice 4)

docs/site/docs/integrations/
└── langflow.md                                        # NEW — public docs page (Slice 5)

.github/workflows/
└── langflow-component-publish.yml                     # NEW — PyPI publish (Slice 5)
```

## 2. Slice breakdown

### Slice 1 — Component skeleton (S)

**Files:** `plugins/langflow/pyproject.toml` (placeholder), `plugins/langflow/src/spendguard_langflow/__init__.py`, `plugins/langflow/src/spendguard_langflow/component.py`, `plugins/langflow/src/spendguard_langflow/_version.py`, `plugins/langflow/tests/test_component_skeleton.py`, `plugins/langflow/README.md`.

```python
# plugins/langflow/src/spendguard_langflow/component.py
"""SpendGuard Langflow custom component.

Wraps any LangChain BaseChatModel (drag-dropped onto the canvas as a
ChatOpenAI / ChatAnthropic / etc. node) with the existing SpendGuard
LangChain integration. All gating logic lives in
`sdk/python/src/spendguard/integrations/langchain.py` — this file is
adapter glue + Langflow component metadata only.
"""
from __future__ import annotations

import os
from typing import Any

from langflow.custom import Component  # type: ignore
from langflow.inputs import HandleInput, IntInput, MessageTextInput, SecretStrInput
from langflow.io import Output
from langflow.schema.dotdict import dotdict


class SpendGuardChatModelWrapper(Component):
    """Drag-drop budget gate for any LangChain BaseChatModel.

    Inputs are defined declaratively via Langflow's input system. The
    `inner` input is a HandleInput typed `LanguageModel` so any model
    node (ChatOpenAI, ChatAnthropic, ChatVertexAI, etc.) can connect.
    """

    display_name = "SpendGuard Budget Gate"
    description = (
        "Gates any LangChain chat model through a SpendGuard sidecar. "
        "Drop a model node (ChatOpenAI, ChatAnthropic, ...) into the "
        "'Inner Model' input. Downstream nodes see a budget-gated model."
    )
    icon = "shield"
    name = "SpendGuardChatModelWrapper"
    documentation = "https://spendguard.dev/docs/integrations/langflow"

    inputs = [
        HandleInput(
            name="inner",
            display_name="Inner Model",
            input_types=["LanguageModel"],
            required=True,
            info="The LangChain BaseChatModel this gate wraps. Connect a ChatOpenAI / ChatAnthropic / etc. node here.",
        ),
        MessageTextInput(
            name="sidecar_uds_path",
            display_name="SpendGuard Sidecar UDS Path",
            value="/run/spendguard/sidecar.sock",
            required=True,
        ),
        SecretStrInput(
            name="tenant_id",
            display_name="Tenant ID",
            required=True,
        ),
        MessageTextInput(
            name="budget_id",
            display_name="Budget ID",
            required=True,
        ),
        MessageTextInput(
            name="window_instance_id",
            display_name="Window Instance ID",
            required=True,
        ),
        MessageTextInput(
            name="unit_token_kind",
            display_name="Unit Token Kind",
            value="output_token",
            advanced=True,
        ),
        MessageTextInput(
            name="model_family",
            display_name="Model Family",
            value="gpt-4",
            advanced=True,
        ),
        IntInput(
            name="claim_estimator_chars_per_token",
            display_name="Estimator chars/token",
            value=4,
            advanced=True,
        ),
    ]

    outputs = [
        Output(
            name="model",
            display_name="Gated Model",
            method="build_model",
            types=["LanguageModel"],
        ),
    ]

    def build_model(self) -> Any:  # returns SpendGuardChatModel — typed Any to dodge import-cycle warnings
        raise NotImplementedError("Wired in Slice 2")
```

Acceptance: import succeeds, class introspection lists 8 inputs + 1 output, `SpendGuardChatModelWrapper.display_name == "SpendGuard Budget Gate"`.

### Slice 2 — Wrap logic (M)

**Files:** `plugins/langflow/src/spendguard_langflow/component.py::build_model`, `plugins/langflow/src/spendguard_langflow/_run_context.py`, `plugins/langflow/tests/test_build_model.py`, `plugins/langflow/tests/test_run_context_autobind.py`.

```python
# plugins/langflow/src/spendguard_langflow/_run_context.py
"""Run-context auto-binding for Langflow-driven invocations.

Langflow nodes call ainvoke()/invoke() without wrapping the call in
spendguard.integrations.langchain.run_context(). Without a bound context
the SDK raises RuntimeError. We monkey-patch the returned wrapper's
_agenerate to enter a context using the Langflow flow_id (or a stable
uuid7 fallback) when none is bound.

Caller-bound contexts ALWAYS win — we only enter if _RUN_CONTEXT.get() is None.
"""
from __future__ import annotations

import functools
import uuid
from typing import Any

from spendguard.integrations.langchain import (
    RunContext,
    SpendGuardChatModel,
    _RUN_CONTEXT,
    run_context,
)


def install_autobind(
    wrapped: SpendGuardChatModel,
    *,
    flow_id: str | None,
) -> SpendGuardChatModel:
    """Wrap `wrapped._agenerate` so each call auto-binds a run-context
    when none is bound by the caller."""
    original_agen = wrapped._agenerate
    base_run_id = flow_id or f"langflow-{uuid.uuid4()}"
    call_counter = {"n": 0}

    @functools.wraps(original_agen)
    async def _agenerate_autobind(messages, stop=None, run_manager=None, **kwargs):
        if _RUN_CONTEXT.get() is not None:
            return await original_agen(messages, stop, run_manager, **kwargs)
        call_counter["n"] += 1
        ctx = RunContext(run_id=f"{base_run_id}:{call_counter['n']}")
        async with run_context(ctx):
            return await original_agen(messages, stop, run_manager, **kwargs)

    # Pydantic v2 BaseModel blocks attribute set on validated fields, but
    # _agenerate is a method — we use object.__setattr__ to bypass.
    object.__setattr__(wrapped, "_agenerate", _agenerate_autobind)
    return wrapped
```

```python
# plugins/langflow/src/spendguard_langflow/component.py  (build_model)
async def _build_async(self) -> Any:
    from spendguard import SpendGuardClient
    from spendguard._proto.spendguard.common.v1 import common_pb2
    from spendguard.integrations.langchain import SpendGuardChatModel
    from ._run_context import install_autobind

    uds = self.sidecar_uds_path or os.environ.get("SPENDGUARD_SIDECAR_UDS")
    if not uds:
        raise ValueError(
            "SpendGuard sidecar UDS not configured. Set the canvas input "
            "'SpendGuard Sidecar UDS Path' or env SPENDGUARD_SIDECAR_UDS."
        )

    client = SpendGuardClient(socket_path=uds, tenant_id=self.tenant_id)
    await client.connect()
    await client.handshake()

    chars_per_token = max(1, int(self.claim_estimator_chars_per_token or 4))
    unit_ref = common_pb2.UnitRef(
        unit_id=f"{self.model_family}.{self.unit_token_kind}",
        token_kind=self.unit_token_kind,
        model_family=self.model_family,
    )

    def estimator(messages):
        chars = sum(len(getattr(m, "content", "")) for m in messages)
        projected = max(50, chars // chars_per_token)
        return [common_pb2.BudgetClaim(
            budget_id=self.budget_id,
            unit=unit_ref,
            amount_atomic=str(projected),
            direction=common_pb2.BudgetClaim.DEBIT,
            window_instance_id=self.window_instance_id,
        )]

    wrapped = SpendGuardChatModel(
        inner=self.inner,
        client=client,
        budget_id=self.budget_id,
        window_instance_id=self.window_instance_id,
        unit=unit_ref,
        pricing=common_pb2.PricingFreeze(),
        claim_estimator=estimator,
    )
    flow_id = getattr(getattr(self, "graph", None), "flow_id", None)
    return install_autobind(wrapped, flow_id=flow_id)


def build_model(self) -> Any:
    import asyncio
    try:
        loop = asyncio.get_running_loop()
    except RuntimeError:
        return asyncio.run(self._build_async())
    # Langflow may call build_model from an async context; run synchronously
    # by creating a task and waiting via a nested loop is unsafe, so we
    # require build_model to be called sync-only and document the constraint.
    if loop.is_running():
        raise RuntimeError(
            "SpendGuardChatModelWrapper.build_model() must be called outside "
            "a running event loop. Langflow's build phase is sync; if you see "
            "this error, file a bug with the Langflow version."
        )
    return loop.run_until_complete(self._build_async())
```

Tests cover: `inner` is a `FakeListChatModel`; build returns a `SpendGuardChatModel`; `ainvoke` after build → fake sidecar logs `request_decision` + `emit_llm_call_post`; auto-bind enters when no `run_context` is open; caller-bound `run_context` wins.

### Slice 3 — Metadata YAML + PyPI packaging (S)

**Files:** `plugins/langflow/metadata/spendguard_chat_model_wrapper.yaml`, `plugins/langflow/pyproject.toml`, `plugins/langflow/src/spendguard_langflow/_install.py`, `plugins/langflow/tests/test_install_script.py`.

```yaml
# plugins/langflow/metadata/spendguard_chat_model_wrapper.yaml
component:
  name: SpendGuardChatModelWrapper
  display_name: SpendGuard Budget Gate
  category: models
  icon: shield
  description: >-
    Wraps any LangChain chat model with a SpendGuard budget gate. Drop
    a ChatOpenAI / ChatAnthropic node into the Inner Model input.
  version: 0.1.0
  documentation: https://spendguard.dev/docs/integrations/langflow
  tags: [budget, governance, langchain, spendguard]
```

```toml
# plugins/langflow/pyproject.toml
[project]
name = "spendguard-langflow-component"
version = "0.1.0"
description = "SpendGuard custom component for Langflow — drag-drop budget gate for any LangChain chat model."
requires-python = ">=3.10"
license = { text = "Apache-2.0" }
authors = [{ name = "Michael Chen", email = "m24927605@gmail.com" }]
dependencies = [
  "spendguard-sdk[langchain]>=0.5.1",
  "langflow>=1.8.0",
]

[project.scripts]
spendguard-langflow-install = "spendguard_langflow._install:main"

[build-system]
requires = ["hatchling>=1.21"]
build-backend = "hatchling.build"
```

`_install.py` exposes `spendguard-langflow-install --target $LANGFLOW_COMPONENTS_PATH`, which copies the component file + metadata YAML into the target tree. Detects an existing file and prompts unless `--force` is passed. Refuses to copy outside a user-writable directory.

### Slice 4 — Demo mode (M)

**Files:** `deploy/demo/Makefile`, `deploy/demo/langflow/compose.override.yaml`, `deploy/demo/langflow/components_seed/spendguard_chat_model_wrapper.py` (symlink to plugin source), `deploy/demo/langflow/flows_seed/flow_chat_openai_wrapped.json`, `deploy/demo/verify_step_langflow.sql`, `deploy/demo/demo/run_demo.py`.

Makefile branch:

```
else ifeq ($(DEMO_MODE),langflow_real)
	@echo "[demo] DEMO_MODE=langflow_real → Langflow + component + sidecar + OpenAI upstream"
	$(COMPOSE) -f compose.yaml -f langflow/compose.override.yaml up -d --build \
	    postgres pki-init bundles-init canonical-seed-init manifest-init \
	    endpoint-catalog ledger canonical-ingest tokenizer sidecar \
	    langflow
```

`langflow/compose.override.yaml` runs `langflowai/langflow:1.8` with:
- `LANGFLOW_COMPONENTS_PATH=/app/components_seed`
- Bind mount the plugin source + Langflow flow seed
- Mount the sidecar UDS
- Postgres database `langflow_db` (separate from `spendguard_ledger`)

`flow_chat_openai_wrapped.json` is a 3-node flow: `ChatInput` → `ChatOpenAI` → `SpendGuardChatModelWrapper` (wraps `ChatOpenAI`) → `ChatOutput`. The wrapper holds the canonical demo budget/window IDs.

Demo driver `run_langflow_real_mode` (~150 LOC):
1. POST `/api/v1/run/{flow_id}` against Langflow's API with a small prompt. Assert 200, sidecar audit row reserved + committed.
2. POST one that exceeds budget. Assert non-2xx + a SpendGuard DENY decision audited; **no upstream HTTP** (counting stub in front of `api.openai.com` registers zero hits for the DENY step).
3. POST a streaming-mode request. Assert 200, end-of-stream commit row.

### Slice 5 — Docs + publish workflow (S)

**Files:** `docs/site/docs/integrations/langflow.md`, `.github/workflows/langflow-component-publish.yml`, `README.md` (1 row added).

Docs page covers: "Why SpendGuard for Langflow", "Install (PyPI)", "Install (vendor-drop)", a decision matrix vs egress-proxy, limitations (no embeddings gate, no token-by-token cap, global-provider config deferred), and a canvas screenshot showing the wrapped flow.

Publish workflow: PyPI Trusted Publisher (OIDC) on `langflow-component-v*` tags. Build wheel + sdist; verify the metadata YAML ships in the wheel; upload.

README row: `Langflow custom component | pip install spendguard-langflow-component && spendguard-langflow-install`.

## 3. Backwards compatibility

| Surface | Action |
|---------|--------|
| `sdk/python/src/spendguard/integrations/langchain.py` | **Untouched.** D36 imports `SpendGuardChatModel` + `RunContext` + `run_context` + `_RUN_CONTEXT` from it. |
| Existing demo modes | Unchanged. The Langflow services live in an overlay file, opt-in per DEMO_MODE branch. |
| `spendguard-sdk` PyPI extras | Unchanged. D36 is a separate PyPI package. |
| Existing DB schemas | Unchanged. Langflow uses `langflow_db` on the shared Postgres instance. |

## 4. Failure modes (must be tested)

| Mode | Expected | Test |
|------|----------|------|
| `langflow` < 1.8 | ImportError at component load with install hint | `test_component_skeleton::test_import_floor` |
| `sidecar_uds_path` empty + `SPENDGUARD_SIDECAR_UDS` unset | `ValueError("SpendGuard sidecar UDS not configured...")` on build | `test_build_model::test_missing_uds` |
| `inner` not provided | Langflow runtime validation fires (required input) | `test_component_skeleton::test_required_inputs` |
| Sidecar DENY | `DecisionDenied` raised by SDK; Langflow surfaces non-2xx | demo step 2 |
| Sidecar DEGRADE | `DecisionSkipped` raised by SDK; Langflow surfaces error node | covered by SDK tests; demo step verifies surface |
| Caller has bound `run_context` already | Auto-bind no-ops; caller's context is used | `test_run_context_autobind::test_caller_bound_wins` |
| `build_model` called inside running event loop | Clear `RuntimeError` with version-bug hint | `test_build_model::test_running_loop_raises` |
| `spendguard-langflow-install` targets path outside user-writable | Refuses with explicit error | `test_install_script::test_refuses_system_path` |

## 5. Code skeleton — total LOC budget

| File | Impl LOC | Test LOC |
|------|----------|----------|
| `component.py` | ~180 | covered by test_build_model + test_component_skeleton |
| `_run_context.py` | ~60 | covered by test_run_context_autobind |
| `_install.py` | ~80 | ~80 |
| `__init__.py` + `_version.py` | ~20 | — |
| `test_component_skeleton.py` | — | ~80 |
| `test_build_model.py` | — | ~140 |
| `test_run_context_autobind.py` | — | ~70 |
| Demo driver / verify SQL | ~150 + ~80 | — |
| **Total** | **~570 + 230 demo** | **~370** |

## 6. Out of scope

Everything in design.md §3. Plus: no changes to `sdk/python/src/spendguard/integrations/langchain.py`. Plus: no proto changes. Plus: no control-plane API changes. Plus: no Langflow upstream PR — we ship as a third-party component package.
