"""LlamaIndex ``CallbackManager`` adapter — gates ``CBEventType.LLM`` events.

LlamaIndex (Apache-2.0, ~47k stars) is the dominant RAG framework. Its
provider integrations (``llama-index-llms-openai``, ``-anthropic``,
``-gemini``, ``-bedrock-converse``) call vendor SDKs directly —
bypassing every existing SpendGuard provider-level adapter unless
operators route through ``llama-index-llms-litellm`` (covered
transitively by D12).

D27 closes this gap by gating at the LlamaIndex callback boundary:
``Settings.callback_manager = CallbackManager([handler])`` propagates
to every ``LLM`` instance and fires ``on_event_start`` /
``on_event_end`` on every ``CBEventType.LLM`` event — without
subclassing ``OpenAI`` / ``Anthropic`` / etc.

Coverage matrix:

  ``llama-index-llms-litellm``    → D12 (LiteLLM SDK shim) — transitive.
  ``llama-index-llms-openai``     → D27 (this module).
  ``llama-index-llms-anthropic``  → D27.
  ``llama-index-llms-gemini``     → D27.
  ``llama-index-llms-bedrock``    → D27.

Operators using mixed setups install BOTH D12 + D27. D12's contextvar
recursion guard prevents double-reservation on the LiteLLM-routed inner
call (the LlamaIndex event still fires PRE; D12 short-circuits the
inner ``acompletion`` reserve).

Install with::

    pip install 'spendguard-sdk[llamaindex]'

Integration shape::

    from llama_index.core import Settings, VectorStoreIndex, Document
    from llama_index.core.callbacks import CallbackManager
    from llama_index.llms.openai import OpenAI

    from spendguard import SpendGuardClient
    from spendguard.integrations.llamaindex import (
        SpendGuardLlamaIndexHandler,
        SpendGuardLlamaIndexDenied,
    )
    from spendguard._proto.spendguard.common.v1 import common_pb2

    client = SpendGuardClient(socket_path=..., tenant_id=...)
    await client.connect()
    await client.handshake()

    handler = SpendGuardLlamaIndexHandler(
        client=client,
        budget_id="...",
        window_instance_id="...",
        unit=common_pb2.UnitRef(
            unit_id="usd_micros",
            token_kind="output_token",
            model_family="gpt-4"),
        pricing=common_pb2.PricingFreeze(pricing_version="2026-q2"),
    )
    Settings.callback_manager = CallbackManager([handler])
    Settings.llm = OpenAI(model="gpt-4o-mini")

    docs = [Document(text="The budget cap is 100 atomic units per window.")]
    index = VectorStoreIndex.from_documents(docs)
    response = index.as_query_engine().query("What is the budget cap?")

    # On DENY:
    try:
        index.as_query_engine().query("...")
    except SpendGuardLlamaIndexDenied as exc:
        print(f"denied: {exc.reason_codes}")

POC scope:
  - Vendor coverage: OpenAI / Anthropic / Gemini / Bedrock Converse via
    response-shape detection (no class-name parsing).
  - Streaming intra-chunk gating: not supported. ``CBEventType.CHUNK``
    is observational; commit fires at turn boundary
    (parity with LangChain / openai-agents priors).
  - ``CBEventType.EMBEDDING`` / ``RETRIEVE`` gating: out-of-budget by
    SpendGuard policy — filtered with a single enum compare at handler
    entry.
  - DENY raises ``SpendGuardLlamaIndexDenied`` from ``on_event_start``.
    LlamaIndex has no documented "skip event" return channel — raising
    IS the documented stop signal.
  - Sync-from-async bridging: the handler owns a per-instance daemon
    thread + asyncio loop and dispatches each async client call onto it
    via ``run_coroutine_threadsafe``. Avoids nest_asyncio and works for
    both sync ``.query()`` and async ``.aquery()``.
"""

from __future__ import annotations

# Import-time guard: surface a helpful error pointing at the
# ``pip install 'spendguard-sdk[llamaindex]'`` install command when the
# user imports this module without ``llama-index-core`` installed. The
# guard fires once at module load; the handler class itself is
# import-resilient (``_hook`` falls back to a plain base class when
# llama-index-core is missing) so the unit suite still runs via direct
# ``importlib.import_module`` bypass.
try:
    from llama_index.core.callbacks.base_handler import (  # noqa: F401
        BaseCallbackHandler,
    )
    from llama_index.core.callbacks.schema import (  # noqa: F401
        CBEventType,
        EventPayload,
    )
except ImportError as exc:  # pragma: no cover
    raise ImportError(
        "spendguard.integrations.llamaindex requires the [llamaindex] extra. "
        "Install with: pip install 'spendguard-sdk[llamaindex]'"
    ) from exc

from ._errors import (
    ApprovalRequired,
    DecisionDenied,
    DecisionSkipped,
    DecisionStopped,
    HandshakeError,
    SidecarUnavailable,
    SpendGuardConfigError,
    SpendGuardError,
    SpendGuardLlamaIndexDenied,
)
from ._hook import (
    ClaimEstimator,
    RunIdFn,
    SpendGuardLlamaIndexHandler,
    current_run_context,
    run_context,
)
from ._options import (
    LlamaIndexRunContext,
    SpendGuardLlamaIndexOptions,
)

__all__ = [
    # Primary handler class
    "SpendGuardLlamaIndexHandler",
    # Denied exception (raised from on_event_start on DENY)
    "SpendGuardLlamaIndexDenied",
    # Type aliases for advanced configuration
    "ClaimEstimator",
    "RunIdFn",
    # Optional POCO config + run context
    "LlamaIndexRunContext",
    "SpendGuardLlamaIndexOptions",
    "current_run_context",
    "run_context",
    # Error re-exports (catch-from-one-place)
    "ApprovalRequired",
    "DecisionDenied",
    "DecisionSkipped",
    "DecisionStopped",
    "HandshakeError",
    "SidecarUnavailable",
    "SpendGuardConfigError",
    "SpendGuardError",
]
